//! Replays a captured four-byte-cell driver schedule through `CellDriver` and
//! diffs its port and wave-RAM writes against the captured register stream.
//!
//! Usage: cell_oracle_diff CONFIG.json ROM ORACLE_DIR [OUT.json]
use serde::Deserialize;
use ws_rom::BankState;
use ws_sound::drivers::{
    cell_driver::{CellCommand, CellDriver, CellLink},
    sdk_driver::{OracleVerdict, Target},
};

#[derive(Deserialize)]
struct Config {
    link: CellLink,
    init_entry: String,
    queue_entry: String,
    ram_lo: u16,
    queue_count_addr: u16,
    queue_data_addr: u16,
    driver_code: Vec<[usize; 2]>,
    #[serde(default)]
    trim_incomplete_last_tick: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct W {
    tick: u32,
    target: Target,
    value: u16,
}

enum Ev {
    Tick { seq: u64, frame: u64 },
    Init { seq: u64 },
    Queue { seq: u64, cmds: Vec<CellCommand> },
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: cell_oracle_diff CONFIG.json ROM ORACLE_DIR [OUT.json]");
        std::process::exit(2);
    }
    let config: Config = serde_json::from_str(&std::fs::read_to_string(&args[1]).expect("config"))
        .expect("config json");
    let rom = std::fs::read(&args[2]).expect("rom");
    let dir = &args[3];
    let init_entry = hex(&config.init_entry) as usize;
    let queue_entry = hex(&config.queue_entry) as usize;

    let mut events: Vec<Ev> = Vec::new();
    for r in rows(dir, "ticks.tsv") {
        events.push(Ev::Tick {
            seq: r[0].parse().unwrap(),
            frame: r[1].parse().unwrap(),
        });
    }
    for r in rows(dir, "play.tsv") {
        let seq: u64 = r[0].parse().unwrap();
        let entry = hex(&r[3]) as usize;
        if entry == init_entry {
            events.push(Ev::Init { seq });
        } else if entry == queue_entry {
            let ram = bytes(&r[6]);
            let cnt_off = usize::from(config.queue_count_addr - config.ram_lo);
            let data_off = usize::from(config.queue_data_addr - config.ram_lo);
            let count = usize::from(*ram.get(cnt_off).unwrap_or(&0)).min(8);
            let mut cmds = Vec::with_capacity(count);
            for i in 0..count {
                let c = *ram.get(data_off + i * 2).unwrap_or(&0);
                let a = *ram.get(data_off + i * 2 + 1).unwrap_or(&0);
                let cmd = match c {
                    0 => CellCommand::Nop,
                    1 => CellCommand::Play { song_id: a },
                    2 => CellCommand::Stop,
                    9 => CellCommand::Mute,
                    10 => CellCommand::Unmute,
                    13 => CellCommand::SetVolume { volume: a },
                    other => panic!("unsupported cell queue cmd {other} arg {a:#x}"),
                };
                cmds.push(cmd);
            }
            if !cmds.is_empty() {
                events.push(Ev::Queue { seq, cmds });
            }
        }
    }
    let seq_of = |e: &Ev| match e {
        Ev::Tick { seq, .. } | Ev::Init { seq } | Ev::Queue { seq, .. } => *seq,
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

    // Map each seq to whether the most recent driver entry was a Tick (vs Init/Queue).
    let in_tick = |seq: u64| -> bool {
        let idx = events.partition_point(|e| seq_of(e) < seq);
        idx > 0 && matches!(events[idx - 1], Ev::Tick { .. })
    };

    let mut oracle_ports: Vec<(W, String)> = Vec::new();
    for r in rows(dir, "ports.tsv") {
        let seq: u64 = r[0].parse().unwrap();
        let site = hex(&r[8]) as usize;
        if !within(&config.driver_code, site) {
            continue;
        }
        let port = hex(&r[4]) as u8;
        let value = hex(&r[5]) as u16;
        let entries = completed_before(seq);
        let tick = if in_tick(seq) {
            entries.saturating_sub(1)
        } else {
            entries
        };
        oracle_ports.push((
            W {
                tick,
                target: Target::Port(port),
                value,
            },
            format!("frame {} site {site:06x}", r[1]),
        ));
    }

    let wave_lo = config.link.wave_ram;
    let mut oracle_waves: Vec<W> = Vec::new();
    for r in rows(dir, "wram.tsv") {
        let seq: u64 = r[0].parse().unwrap();
        let addr = hex(&r[3]) as u16;
        if addr < wave_lo || addr >= wave_lo + 0x40 {
            continue;
        }
        let entries = completed_before(seq);
        let tick = if in_tick(seq) {
            entries.saturating_sub(1)
        } else {
            entries
        };
        oracle_waves.push(W {
            tick,
            target: Target::WaveWord(addr),
            value: hex(&r[4]) as u16,
        });
    }

    let mut tick_count = tick_seqs.len() as u32;
    if config.trim_incomplete_last_tick {
        if let Some(Ev::Tick { seq: last_seq, .. }) = events.last() {
            let max_port_seq = rows(dir, "ports.tsv")
                .iter()
                .map(|r| r[0].parse::<u64>().unwrap())
                .max()
                .unwrap_or(0);
            if *last_seq > max_port_seq {
                events.pop();
                tick_count = tick_count.saturating_sub(1);
            }
        }
    }

    let mut model = CellDriver::new(config.link, &rom, BankState::RESET).expect("cell driver");
    let mut model_error = None;
    let mut tick_frames = Vec::new();
    for ev in &events {
        let res = match ev {
            Ev::Init { .. } => model.init(),
            Ev::Queue { cmds, .. } => {
                let mut r = Ok(());
                for &c in cmds {
                    if let Err(e) = model.dispatch(c) {
                        r = Err(e);
                        break;
                    }
                }
                r
            }
            Ev::Tick { frame, .. } => {
                tick_frames.push(*frame);
                model.tick()
            }
        };
        if let Err(e) = res {
            model_error = Some(format!("tick {}: {e}", model.ticks));
            break;
        }
    }

    let last_tick = model.ticks;
    let o_ports: Vec<W> = oracle_ports
        .iter()
        .filter(|(w, _)| w.tick < last_tick)
        .map(|(w, _)| *w)
        .collect();
    let o_sites: Vec<&String> = oracle_ports
        .iter()
        .filter(|(w, _)| w.tick < last_tick)
        .map(|(_, s)| s)
        .collect();
    let m_ports: Vec<W> = model
        .writes
        .iter()
        .filter(|w| matches!(w.target, Target::Port(_)))
        .map(|w| W {
            tick: w.tick,
            target: w.target,
            value: w.value,
        })
        .collect();

    let mut first_port_div = None;
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
        first_port_div = Some(serde_json::json!({
            "index": i,
            "tick": tick,
            "frame": tick_frames.get(tick as usize),
            "oracle": o_ports.get(i).map(|w| format!("{:?} {:02x} (tick {})", w.target, w.value, w.tick)),
            "oracle_site": o_sites.get(i),
            "model": m_ports.get(i).map(|w| format!("{:?} {:02x} (tick {})", w.target, w.value, w.tick)),
        }));
        break;
    }

    let o_waves: Vec<W> = oracle_waves
        .into_iter()
        .filter(|w| w.tick < last_tick)
        .collect();
    let m_waves: Vec<W> = model
        .writes
        .iter()
        .filter(|w| matches!(w.target, Target::WaveWord(_)))
        .map(|w| W {
            tick: w.tick,
            target: w.target,
            value: w.value,
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
        oracle_ticks: tick_count,
        model_ticks: last_tick,
        model_error: model_error.clone(),
        ports_equal: first_port_div.is_none() && o_ports.len() == m_ports.len(),
        waves_equal: wave_first.is_none(),
        ram_equal: true,
    };
    let summary = serde_json::json!({
        "oracle": dir,
        "ticks": last_tick,
        "oracle_ticks": verdict.oracle_ticks,
        "pass": verdict.passed(),
        "port_writes_oracle": o_ports.len(),
        "port_writes_model": m_ports.len(),
        "port_writes_matched_prefix": matched,
        "port_exact": verdict.port_exact(),
        "first_port_divergence": first_port_div,
        "wave_words_oracle": o_waves.len(),
        "wave_words_model": m_waves.len(),
        "wave_exact": verdict.wave_exact(),
        "first_wave_divergence": wave_first,
        "model_error": model_error,
    });
    let text = serde_json::to_string_pretty(&summary).unwrap();
    if let Some(out) = args.get(4) {
        std::fs::write(out, &text).expect("write out");
    }
    println!("{text}");
    if !verdict.passed() {
        std::process::exit(1);
    }
}
