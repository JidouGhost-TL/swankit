//! Replays a captured driver schedule through the tick-level SDK model and
//! diffs its writes against the captured register stream, per tick.
//!
//! Usage: sdk_oracle_diff CONFIG.json ROM ORACLE_DIR [OUT.json]
//!
//! The oracle directory holds tab-separated captures: `ticks.tsv` (tick
//! entries with a driver RAM snapshot), `play.tsv` (driver entry calls with AX
//! and CX), `ports.tsv` (sound-port writes with their ROM site) and
//! `wram.tsv` (wave-RAM word writes). Rows are ordered by their `seq` column.
//! The config names the data segment, the link, the driver code ranges and
//! what each entry call does. Only the schedule (which entry ran between which
//! ticks, with its register arguments) is taken from the capture; every
//! compared write and RAM byte comes from the model.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use ws_sound::drivers::sdk_driver::{field, OracleVerdict, SdkDriver, SdkLink, Target};

#[derive(Deserialize)]
struct Config {
    segment_file_offset: usize,
    link: SdkLink,
    headphones: bool,
    ram_lo: u16,
    ram_hi: u16,
    /// ROM ranges (inclusive) of code that runs inside the tick call.
    tick_code: Vec<[usize; 2]>,
    /// ROM ranges of the other driver entries (reset, play, helpers).
    entry_code: Vec<[usize; 2]>,
    /// Entry ROM offset (hex string) to action.
    calls: BTreeMap<String, Call>,
    /// Report writes and RAM from this tick on (0 = from the first tick).
    #[serde(default)]
    from_tick: u32,
    #[serde(default)]
    wave_ram_lo: Option<u16>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Call {
    Reset,
    StopVoice,
    ClearVoice,
    Start { count: u16 },
    Deferred,
    FirstTrack { first: u8, shadow: Shadow },
    Ignore,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Shadow {
    Al,
    Zero,
    Keep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct W {
    tick: u32,
    target: Target,
    value: u16,
}

enum Ev {
    Tick {
        seq: u64,
        frame: u64,
        cycle: u64,
        ram: Vec<u8>,
    },
    Call {
        seq: u64,
        cycle: u64,
        entry: usize,
        ax: u16,
        cx: u16,
    },
}

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex field")
}
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex bytes"))
        .collect()
}
fn rows(dir: &str, name: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(format!("{dir}/{name}")).unwrap_or_default();
    text.lines()
        .skip(1)
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}
fn within(ranges: &[[usize; 2]], at: usize) -> bool {
    ranges.iter().any(|r| r[0] <= at && at <= r[1])
}

fn slot_report(ram: &[u8], link: &SdkLink, data: &[u8], base: u16) -> String {
    let mut out = String::new();
    for slot in 0..8u16 {
        let s = usize::from(base + slot * link.slot_stride);
        let rw = |o: u16| {
            u16::from(ram[s + usize::from(o)]) | u16::from(ram[s + usize::from(o) + 1]) << 8
        };
        let rb = |o: u16| ram[s + usize::from(o)];
        let pos = rw(field::POSITION);
        if pos == 0xffff {
            let _ = writeln!(out, "  slot {slot}: idle");
            continue;
        }
        let seq = rw(field::SEQUENCE);
        let at = usize::from(pos.wrapping_mul(2).wrapping_add(seq));
        let cells: Vec<String> = (0..4)
            .filter_map(|i| data.get(at + 2 * i..at + 2 * i + 2))
            .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
            .collect();
        let _ = writeln!(
            out,
            "  slot {slot}: ch {} flags {:02x} pos {pos} seq {seq:04x} next [{}] dur {} vol {:02x} freq {:04x} mirror {:02x} cap {:02x} env {:02x}/{:02x}/{:02x} keyon {:02x}/{:02x} tempo {:02x}+{:02x} slide {:02x}/{:02x}/{:02x} wave {:02x}",
            rb(field::CHANNEL_FLAGS) & 3,
            rb(field::CHANNEL_FLAGS),
            cells.join(" "),
            rb(field::DURATION),
            rb(field::VOLUME),
            rw(field::FREQUENCY),
            rb(field::VOLUME_MIRROR),
            rb(field::VOLUME_CAP),
            rb(field::ENVELOPE_GROUP),
            rb(field::ENVELOPE_PERIOD),
            rb(field::ENVELOPE_POSITION),
            rb(field::KEY_ON_RELOAD),
            rb(field::KEY_ON_COUNTER),
            rb(field::HEADER_FLAGS),
            rb(field::TEMPO_ACCUMULATOR),
            rb(field::SLIDE_BUDGET),
            rb(field::SLIDE_PERIOD),
            rb(field::SLIDE_DIVIDER),
            rb(field::WAVE_OFFSET),
        );
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: sdk_oracle_diff CONFIG.json ROM ORACLE_DIR [OUT.json]");
        std::process::exit(2);
    }
    let config: Config = serde_json::from_str(&std::fs::read_to_string(&args[1]).expect("config"))
        .expect("config json");
    let rom = std::fs::read(&args[2]).expect("rom");
    let dir = &args[3];
    let data =
        &rom[config.segment_file_offset..(config.segment_file_offset + 0x10000).min(rom.len())];
    let calls: BTreeMap<usize, Call> = config
        .calls
        .iter()
        .map(|(k, v)| (hex(k) as usize, *v))
        .collect();

    // Schedule.
    let mut events: Vec<Ev> = Vec::new();
    for r in rows(dir, "ticks.tsv") {
        events.push(Ev::Tick {
            seq: r[0].parse().unwrap(),
            frame: r[1].parse().unwrap(),
            cycle: r[2].parse().unwrap(),
            ram: bytes(&r[3]),
        });
    }
    for r in rows(dir, "play.tsv") {
        events.push(Ev::Call {
            seq: r[0].parse().unwrap(),
            cycle: r[2].parse().unwrap(),
            entry: hex(&r[3]) as usize,
            ax: hex(&r[4]) as u16,
            cx: hex(&r[5]) as u16,
        });
    }
    let seq_of = |e: &Ev| match e {
        Ev::Tick { seq, .. } | Ev::Call { seq, .. } => *seq,
    };
    events.sort_by_key(seq_of);
    let tick_seqs: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Ev::Tick { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    let completed_before = |seq: u64| tick_seqs.partition_point(|&t| t < seq) as u32;

    // Oracle writes, attributed to tick indices.
    let mut oracle: Vec<W> = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    let mut oracle_pc: Vec<String> = Vec::new();
    for r in rows(dir, "ports.tsv") {
        let seq: u64 = r[0].parse().unwrap();
        let site = hex(&r[8]) as usize;
        let entries = completed_before(seq);
        let port = hex(&r[4]) as u8;
        let value = hex(&r[5]) as u16;
        let tick = if within(&config.tick_code, site) {
            entries.saturating_sub(1)
        } else if within(&config.entry_code, site) {
            entries
        } else {
            foreign.push(format!(
                "frame {} port {port:02x}={value:02x} site {site:06x}",
                r[1]
            ));
            continue;
        };
        oracle.push(W {
            tick,
            target: Target::Port(port),
            value,
        });
        oracle_pc.push(format!("frame {} site {site:06x}", r[1]));
    }
    let wave_lo = config.wave_ram_lo.unwrap_or(config.link.wave_ram);
    let mut wave_rows: Vec<(u64, W)> = Vec::new();
    for r in rows(dir, "wram.tsv") {
        let seq: u64 = r[0].parse().unwrap();
        let addr = hex(&r[3]) as u16;
        if addr < wave_lo || addr >= wave_lo + 0x40 {
            continue;
        }
        let entries = completed_before(seq);
        wave_rows.push((
            seq,
            W {
                tick: entries.saturating_sub(1),
                target: Target::WaveWord(addr),
                value: hex(&r[4]) as u16,
            },
        ));
    }
    let oracle_port_cycles: Vec<u64> = rows(dir, "ports.tsv")
        .iter()
        .filter(|r| {
            let site = hex(&r[8]) as usize;
            within(&config.tick_code, site) || within(&config.entry_code, site)
        })
        .map(|r| r[2].parse().unwrap())
        .collect();
    let oracle_wave_cycles: Vec<u64> = rows(dir, "wram.tsv")
        .iter()
        .filter(|r| {
            let a = hex(&r[3]) as u16;
            a >= wave_lo && a < wave_lo + 0x40
        })
        .map(|r| r[2].parse().unwrap())
        .collect();
    let wide_waves = rows(dir, "wram.tsv").iter().any(|r| r[4].len() > 2);

    // Model replay.
    let mut model = SdkDriver::new(config.link, data, config.headphones).expect("model");
    let mut ram_mismatch: Option<String> = None;
    let mut model_error: Option<String> = None;
    let mut tick_frames: Vec<u64> = Vec::new();
    let mut ram_ticks_compared = 0u64;
    let mut snapshots: Vec<Vec<u8>> = Vec::new();
    let mut write_cycles: Vec<u64> = Vec::new();
    for event in &events {
        let before = model.writes.len();
        let event_cycle = match event {
            Ev::Tick { cycle, .. } | Ev::Call { cycle, .. } => *cycle,
        };
        let result = match event {
            Ev::Call { entry, ax, cx, .. } => match calls.get(entry) {
                Some(Call::Reset) => model.reset(0),
                Some(Call::StopVoice) => model.stop_voice(),
                Some(Call::Start { count }) => model.start(*ax, *count),
                Some(Call::Deferred) => {
                    model.set_deferred((cx >> 8) as u8, *cx as u8);
                    Ok(())
                }
                Some(Call::FirstTrack { first, shadow }) => {
                    let shadow = match shadow {
                        Shadow::Al => Some(*ax as u8),
                        Shadow::Zero => Some(0),
                        Shadow::Keep => None,
                    };
                    model.set_first_track(*first, shadow);
                    Ok(())
                }
                Some(Call::ClearVoice) => {
                    model.clear_voice();
                    Ok(())
                }
                Some(Call::Ignore) => Ok(()),
                None => Err(formatkit_core::Error::Unsupported(format!(
                    "unconfigured entry {entry:06x}"
                ))),
            },
            Ev::Tick { frame, ram, .. } => {
                tick_frames.push(*frame);
                if model.ticks >= config.from_tick && ram_mismatch.is_none() {
                    ram_ticks_compared += 1;
                    let lo = usize::from(config.ram_lo);
                    for (i, &b) in ram.iter().enumerate() {
                        let at = lo + i;
                        if at > usize::from(config.ram_hi) {
                            break;
                        }
                        if model.ram[at] != b {
                            ram_mismatch = Some(format!(
                                "tick {} (frame {frame}) RAM {at:04x}: oracle {b:02x} model {:02x}\noracle slots:\n{}model slots:\n{}",
                                model.ticks,
                                model.ram[at],
                                {
                                    let mut full = vec![0u8; 0x10000];
                                    full[lo..lo + ram.len()].copy_from_slice(ram);
                                    slot_report(&full, &config.link, data, config.link.slots)
                                },
                                slot_report(&model.ram, &config.link, data, config.link.slots)
                            ));
                            break;
                        }
                    }
                }
                snapshots.push(model.ram.clone());
                model.tick()
            }
        };
        write_cycles.resize(model.writes.len().max(before), event_cycle);
        if let Err(e) = result {
            model_error = Some(format!("tick {}: {e}", model.ticks));
            break;
        }
    }
    let model_writes: Vec<W> = model
        .writes
        .iter()
        .map(|w| W {
            tick: w.tick,
            target: w.target,
            value: w.value,
        })
        .collect();

    // Diff of ports (ordered within each tick) from `from_tick`.
    let split = |ws: &[W], ports: bool| -> Vec<W> {
        ws.iter()
            .copied()
            .filter(|w| w.tick >= config.from_tick && matches!(w.target, Target::Port(_)) == ports)
            .collect()
    };
    let last_tick = model.ticks;
    let o_sites: Vec<&String> = oracle
        .iter()
        .zip(&oracle_pc)
        .filter(|(w, _)| w.tick >= config.from_tick && w.tick < last_tick)
        .map(|(_, s)| s)
        .collect();
    let o_ports: Vec<W> = split(&oracle, true)
        .into_iter()
        .filter(|w| w.tick < last_tick)
        .collect();
    let m_ports = split(&model_writes, true);
    let mut first_port_divergence = None;
    let mut matched = 0usize;
    for i in 0..o_ports.len().max(m_ports.len()) {
        if o_ports.get(i) == m_ports.get(i) {
            matched += 1;
            continue;
        }
        let tick = o_ports
            .get(i)
            .or(m_ports.get(i))
            .map(|w| w.tick)
            .unwrap_or(0);
        let ctx = |ws: &[W]| -> Vec<String> {
            ws.iter()
                .filter(|w| w.tick == tick)
                .map(|w| match w.target {
                    Target::Port(p) => format!("{p:02x}={:02x}", w.value),
                    Target::WaveWord(a) => format!("[{a:04x}]={:04x}", w.value),
                })
                .collect()
        };
        let state = snapshots
            .get(tick as usize)
            .map(|ram| slot_report(ram, &config.link, data, config.link.slots))
            .unwrap_or_default();
        first_port_divergence = Some(serde_json::json!({
            "index": i,
            "tick": tick,
            "frame": tick_frames.get(tick as usize),
            "oracle": o_ports.get(i).map(|w| format!("{:?} {:02x} (tick {})", w.target, w.value, w.tick)),
            "oracle_site": o_sites.get(i),
            "model": m_ports.get(i).map(|w| format!("{:?} {:02x} (tick {})", w.target, w.value, w.tick)),
            "oracle_tick_writes": ctx(&o_ports),
            "model_tick_writes": ctx(&m_ports),
            "model_state_at_tick_start": state,
        }));
        break;
    }
    // Wave words: compare by order within the window.
    let o_waves: Vec<W> = wave_rows
        .iter()
        .map(|(_, w)| *w)
        .filter(|w| w.tick >= config.from_tick && w.tick < last_tick)
        .collect();
    let m_waves: Vec<W> = split(&model_writes, false)
        .into_iter()
        .map(|w| {
            if wide_waves {
                w
            } else {
                W {
                    value: w.value & 0xff,
                    ..w
                }
            }
        })
        .collect();
    let wave_first = (0..o_waves.len().max(m_waves.len()))
        .find(|&i| o_waves.get(i) != m_waves.get(i))
        .map(|i| {
            format!(
                "index {i}: oracle {:?} model {:?}",
                o_waves.get(i),
                m_waves.get(i)
            )
        });

    let verdict = OracleVerdict {
        oracle_ticks: tick_seqs.len() as u32,
        model_ticks: last_tick,
        model_error: model_error.clone(),
        ports_equal: first_port_divergence.is_none() && o_ports.len() == m_ports.len(),
        waves_equal: wave_first.is_none(),
        ram_equal: ram_mismatch.is_none(),
    };
    let summary = serde_json::json!({
        "oracle": dir,
        "ticks": last_tick,
        "oracle_ticks": verdict.oracle_ticks,
        "model_complete": verdict.complete(),
        "pass": verdict.passed(),
        "from_tick": config.from_tick,
        "port_writes_oracle": o_ports.len(),
        "port_writes_model": m_ports.len(),
        "port_writes_matched_prefix": matched,
        "port_exact": verdict.port_exact(),
        "first_port_divergence": first_port_divergence,
        "wave_words_oracle": o_waves.len(),
        "wave_words_model": m_waves.len(),
        "wave_values_16bit": wide_waves,
        "wave_exact": verdict.wave_exact(),
        "first_wave_divergence": wave_first,
        "ram_ticks_compared": ram_ticks_compared,
        "ram_exact": verdict.ram_exact(),
        "first_ram_divergence": ram_mismatch,
        "foreign_writes": foreign.len(),
        "foreign_examples": foreign.iter().take(8).collect::<Vec<_>>(),
        "model_error": model_error,
    });
    let text = serde_json::to_string_pretty(&summary).unwrap();
    // Optional hardware render of the model's stream: writes timed at the
    // schedule event that produced them ("tick"), or at the captured cycle of
    // the matching oracle write ("oracle", exact streams only).
    if let (Some(spec), Some(path)) = (args.get(5), args.get(6)) {
        let (mode, renderer) = spec.split_once(':').unwrap_or((spec.as_str(), "frame"));
        let exact = verdict.passed() && config.from_tick == 0;
        let mut cycles = write_cycles.clone();
        if mode == "oracle" {
            assert!(exact, "oracle timing needs an exact full-length stream");
            let (mut p, mut w) = (0usize, 0usize);
            for (i, write) in model.writes.iter().enumerate() {
                match write.target {
                    Target::Port(_) => {
                        cycles[i] = oracle_port_cycles[p];
                        p += 1;
                    }
                    Target::WaveWord(_) => {
                        cycles[i] = oracle_wave_cycles[w];
                        w += 1;
                    }
                }
            }
        }
        use ws_sound::render::{render, render_cycles, Action, CounterReload, CycleEvent, Event};
        // Diagnostic: SDK_RENDER_CHANNELS=bits keeps only those channel enables.
        let keep: u8 = std::env::var("SDK_RENDER_CHANNELS")
            .ok()
            .map(|v| u8::from_str_radix(&v, 16).expect("channel mask"))
            .unwrap_or(0x0f);
        let mut image = [0u8; 64];
        let mut order: Vec<usize> = (0..model.writes.len()).collect();
        order.sort_by_key(|&i| cycles[i]);
        let mut render_events = Vec::new();
        for i in order {
            let write = model.writes[i];
            let frame = (cycles[i] / 128) as usize;
            let action = match write.target {
                Target::Port(p) => Action::Write {
                    port: p,
                    value: if p == 0x90 {
                        write.value as u8 & (0xf0 | keep)
                    } else {
                        write.value as u8
                    },
                },
                Target::WaveWord(a) => {
                    let at = usize::from(a - config.link.wave_ram);
                    image[at] = write.value as u8;
                    image[at + 1] = (write.value >> 8) as u8;
                    Action::Waves(ws_sound::WaveBank::decode(&image).unwrap())
                }
            };
            render_events.push((cycles[i], Event { frame, action }));
        }
        let last_cycle = events
            .iter()
            .map(|e| match e {
                Ev::Tick { cycle, .. } | Ev::Call { cycle, .. } => *cycle,
            })
            .max()
            .unwrap_or(0);
        let frames = last_cycle / 128 + 318;
        let pcm = match renderer {
            "frame" => render(
                &render_events.iter().map(|e| e.1).collect::<Vec<_>>(),
                frames as usize,
            ),
            "remainder" | "next" => render_cycles(
                &render_events
                    .iter()
                    .map(|(c, e)| CycleEvent {
                        cycle: *c,
                        action: e.action,
                    })
                    .collect::<Vec<_>>(),
                frames as usize,
                if renderer == "next" {
                    CounterReload::NextPeriod
                } else {
                    CounterReload::Remainder
                },
            ),
            other => panic!("unknown renderer {other}"),
        }
        .expect("render");
        std::fs::write(path, pcm.to_wav_bytes().unwrap()).expect("write wav");
    }
    if let Some(out) = args.get(4) {
        std::fs::write(out, &text).expect("write output");
        let model_tsv: String = model_writes
            .iter()
            .zip(&write_cycles)
            .map(|(w, c)| match w.target {
                Target::Port(p) => format!("{}\t{c}\tport\t{p:02x}\t{:02x}\n", w.tick, w.value),
                Target::WaveWord(a) => format!("{}\t{c}\twave\t{a:04x}\t{:04x}\n", w.tick, w.value),
            })
            .collect();
        std::fs::write(format!("{out}.model.tsv"), model_tsv).expect("write model writes");
    }
    println!("{text}");
    if !verdict.passed() {
        std::process::exit(1);
    }
}
