//! Deterministic static discovery of standard SDK two-byte-cell sound-driver
//! link parameters directly from cartridge ROM bytes.
//!
//! Matches the 16-bit x86 instruction patterns of the standard SDK driver
//! (tick, cell fetch, play loop, and voice helpers) without any title-specific
//! tables or addresses. All bounds are checked before indexing.

use crate::drivers::sdk_driver::{MuteTail, SdkGlobals, SdkLink};
use formatkit_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Jump-sizing variant of the standard cell fetcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchVariant {
    /// Near 16-bit relative jumps (`0xE9`).
    NearJumps,
    /// Short 8-bit relative jumps (`0xEB`).
    ShortJumps,
    /// Unclassified jump encoding.
    Unknown,
}

/// Complete static discovery result for one standard SDK driver image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkDiscovery {
    pub segment_file_offset: usize,
    pub data_segment: u16,
    pub link: SdkLink,
    pub fetch_variant: FetchVariant,
    pub tick_offset: u16,
    pub fetch_offset: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_voice_offset: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_voice_offset: Option<u16>,
}

/// Byte pattern element for 16-bit x86 machine-code matching.
#[derive(Clone, Copy)]
enum Pat {
    Lit(u8),
    Any,
}

use Pat::{Any, Lit};

fn find_pattern(haystack: &[u8], pattern: &[Pat]) -> Option<usize> {
    if pattern.is_empty() || haystack.len() < pattern.len() {
        return None;
    }
    'outer: for start in 0..=haystack.len() - pattern.len() {
        for (i, &p) in pattern.iter().enumerate() {
            if let Lit(b) = p {
                if haystack[start + i] != b {
                    continue 'outer;
                }
            }
        }
        return Some(start);
    }
    None
}

fn read_u16(buf: &[u8], at: usize) -> Option<u16> {
    let bytes = buf.get(at..at + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

/// Locate a standard SDK two-byte-cell sound driver in `rom` and extract its
/// linked RAM/table offsets and segment file base.
pub fn discover_sdk_link(rom: &[u8]) -> Result<SdkDiscovery> {
    // Standard cell-fetch prologue:
    //   MOV BX, [SI]       ; 8B 1C
    //   SHL BX, 1          ; D1 E3
    //   ADD BX, [SI+3]     ; 03 5C 03
    //   MOV AX, <es_seg>   ; B8 lo hi
    //   MOV ES, AX         ; 8E C0
    //   INC WORD [SI]      ; FF 04
    //   MOV AL, ES:[BX]    ; 26 8A 07
    //   INC BX             ; 43
    const FETCH_PAT: &[Pat] = &[
        Lit(0x8b),
        Lit(0x1c),
        Lit(0xd1),
        Lit(0xe3),
        Lit(0x03),
        Lit(0x5c),
        Lit(0x03),
        Lit(0xb8),
        Any,
        Any,
        Lit(0x8e),
        Lit(0xc0),
        Lit(0xff),
        Lit(0x04),
        Lit(0x26),
        Lit(0x8a),
        Lit(0x07),
        Lit(0x43),
    ];

    let fetch_file = find_pattern(rom, FETCH_PAT)
        .ok_or_else(|| Error::Unsupported("no standard SDK sound fetcher found in ROM".into()))?;
    let data_segment = read_u16(rom, fetch_file + 8)
        .ok_or_else(|| Error::Malformed("truncated SDK fetch ES immediate".into()))?;

    let segment_file_offset = compute_segment_offset(rom.len(), fetch_file, data_segment);
    let seg_end = segment_file_offset.saturating_add(0x10000).min(rom.len());
    let seg = &rom[segment_file_offset..seg_end];
    let fetch_off = fetch_file - segment_file_offset;

    // Tick output-route head:
    //   IN AL, 91h         ; E4 91
    //   CMP AL, [route]    ; 3A 06 lo hi
    //   JE +rel8           ; 74 ??
    //   MOV [route], AL    ; A2 lo hi
    //   TEST AL, 80h       ; A8 80
    const TICK_HEAD_PAT: &[Pat] = &[
        Lit(0xe4),
        Lit(0x91),
        Lit(0x3a),
        Lit(0x06),
        Any,
        Any,
        Lit(0x74),
        Any,
        Lit(0xa2),
        Any,
        Any,
        Lit(0xa8),
        Lit(0x80),
    ];

    // Tick track-scan setup (two compiler forms for clearing taken_mask):
    // Form A: MOV AL,[first]; MOV [trk],AL; MOV BYTE [taken],0; INC BYTE [phase]; MOV SI,slots
    const TICK_SCAN_A: &[Pat] = &[
        Lit(0xa0),
        Any,
        Any,
        Lit(0xa2),
        Any,
        Any,
        Lit(0xc6),
        Lit(0x06),
        Any,
        Any,
        Lit(0x00),
        Lit(0xfe),
        Lit(0x06),
        Any,
        Any,
        Lit(0xbe),
        Any,
        Any,
    ];
    // Form B: MOV AL,[first]; MOV [trk],AL; XOR AX,AX; MOV [taken],AL; INC BYTE [phase]; MOV SI,slots
    const TICK_SCAN_B: &[Pat] = &[
        Lit(0xa0),
        Any,
        Any,
        Lit(0xa2),
        Any,
        Any,
        Lit(0x33),
        Lit(0xc0),
        Lit(0xa2),
        Any,
        Any,
        Lit(0xfe),
        Lit(0x06),
        Any,
        Any,
        Lit(0xbe),
        Any,
        Any,
    ];

    let (tick_scan_off, first_track, track_index, taken_mask, phase_counter, slots) =
        if let Some(pos) = find_pattern(seg, TICK_SCAN_A) {
            (
                pos,
                read_u16(seg, pos + 1).unwrap(),
                read_u16(seg, pos + 4).unwrap(),
                read_u16(seg, pos + 8).unwrap(),
                read_u16(seg, pos + 13).unwrap(),
                read_u16(seg, pos + 16).unwrap(),
            )
        } else if let Some(pos) = find_pattern(seg, TICK_SCAN_B) {
            (
                pos,
                read_u16(seg, pos + 1).unwrap(),
                read_u16(seg, pos + 4).unwrap(),
                read_u16(seg, pos + 9).unwrap(),
                read_u16(seg, pos + 13).unwrap(),
                read_u16(seg, pos + 16).unwrap(),
            )
        } else {
            return Err(Error::Unsupported(
                "standard SDK tick scan sequence not found".into(),
            ));
        };

    let (tick_off, route_last) = match find_pattern(seg, TICK_HEAD_PAT) {
        Some(pos) if pos < tick_scan_off && tick_scan_off - pos < 64 => {
            (pos, read_u16(seg, pos + 4).unwrap())
        }
        _ => {
            return Err(Error::Unsupported(
                "standard SDK tick output-route header not found".into(),
            ));
        }
    };

    let tick_end = seg
        .len()
        .min(fetch_off.max(tick_scan_off.saturating_add(0x500)));
    let tick_win = &seg[tick_scan_off..tick_end];

    // Channel index & channel bits:
    //   MOV CL,[SI+2]; AND CL,3; MOV [ch_idx],CL; INC CL; MOV AL,88h; ROL AL,1; DEC CL; JNE -4; MOV [ch_bits],AL
    const CH_PAT: &[Pat] = &[
        Lit(0x8a),
        Lit(0x4c),
        Lit(0x02),
        Lit(0x80),
        Lit(0xe1),
        Lit(0x03),
        Lit(0x88),
        Lit(0x0e),
        Any,
        Any,
        Lit(0xfe),
        Lit(0xc1),
        Lit(0xb0),
        Lit(0x88),
        Lit(0xd0),
        Lit(0xc0),
        Lit(0xfe),
        Lit(0xc9),
        Lit(0x75),
        Lit(0xfa),
        Lit(0xa2),
        Any,
        Any,
    ];
    let ch_pos = find_pattern(tick_win, CH_PAT)
        .ok_or_else(|| Error::Unsupported("SDK channel-index pattern not found".into()))?;
    let channel_index = read_u16(tick_win, ch_pos + 8).unwrap();
    let channel_bits = read_u16(tick_win, ch_pos + 21).unwrap();

    // Vibrato table:
    //   MOV BX,<vib>; ADD BX,AX; MOV DX,[SI+10h]; MOV AL,CS:[BX]; CBW; ADD AX,DX; MOV DX,80h
    const VIB_PAT: &[Pat] = &[
        Lit(0xbb),
        Any,
        Any,
        Lit(0x03),
        Lit(0xd8),
        Lit(0x8b),
        Lit(0x54),
        Lit(0x10),
        Lit(0x2e),
        Lit(0x8a),
        Lit(0x07),
        Lit(0x98),
        Lit(0x03),
        Lit(0xc2),
        Lit(0xba),
        Lit(0x80),
        Lit(0x00),
    ];
    let vib_pos = find_pattern(tick_win, VIB_PAT)
        .ok_or_else(|| Error::Unsupported("SDK vibrato-table pattern not found".into()))?;
    let vibrato_table = read_u16(tick_win, vib_pos + 1).unwrap();

    // Envelope groups & envelope volumes:
    const ENV_PAT: &[Pat] = &[
        Lit(0xbf),
        Any,
        Any,
        Lit(0x83),
        Lit(0xef),
        Lit(0x10),
        Lit(0x03),
        Lit(0xf8),
        Lit(0x8a),
        Lit(0x64),
        Lit(0x08),
        Lit(0x2e),
        Lit(0x8a),
        Lit(0x05),
        Lit(0x24),
        Lit(0xf0),
        Lit(0x0a),
        Lit(0xc4),
        Lit(0xbf),
        Any,
        Any,
        Lit(0x32),
        Lit(0xe4),
        Lit(0x03),
        Lit(0xf8),
        Lit(0x2e),
        Lit(0x8a),
        Lit(0x05),
        Lit(0x88),
        Lit(0x44),
        Lit(0x26),
    ];
    let env_pos = find_pattern(tick_win, ENV_PAT)
        .ok_or_else(|| Error::Unsupported("SDK envelope-table pattern not found".into()))?;
    let envelope_groups = read_u16(tick_win, env_pos + 1).unwrap();
    let envelope_volumes = read_u16(tick_win, env_pos + 19).unwrap();

    // Slot stride: ADD SI,<stride>; INC BYTE [track_index]; CMP BYTE [track_index], 8
    let ti = track_index.to_le_bytes();
    let stride_pat = [
        Lit(0x83),
        Lit(0xc6),
        Any,
        Lit(0xfe),
        Lit(0x06),
        Lit(ti[0]),
        Lit(ti[1]),
        Lit(0x80),
        Lit(0x3e),
        Lit(ti[0]),
        Lit(ti[1]),
        Lit(0x08),
    ];
    let stride_pos = find_pattern(tick_win, &stride_pat)
        .ok_or_else(|| Error::Unsupported("SDK slot-stride pattern not found".into()))?;
    let slot_stride = u16::from(tick_win[stride_pos + 2]);
    if slot_stride == 0 {
        return Err(Error::Malformed(
            "discovered SDK slot_stride is zero".into(),
        ));
    }

    // Control merge + deferred parameter apply:
    const CTRL_PAT: &[Pat] = &[
        Lit(0xe4),
        Lit(0x90),
        Lit(0x8a),
        Lit(0x26),
        Any,
        Any,
        Lit(0x80),
        Lit(0xe4),
        Lit(0x0f),
        Lit(0x0a),
        Lit(0xc4),
        Lit(0x80),
        Lit(0x3e),
        Any,
        Any,
        Lit(0x00),
        Lit(0x74),
        Any,
        Lit(0x0c),
        Lit(0x22),
        Lit(0xe6),
        Lit(0x90),
        Lit(0xa0),
        Any,
        Any,
        Lit(0x22),
        Lit(0x06),
        Any,
        Any,
        Lit(0x74),
        Any,
        Lit(0xbb),
        Any,
        Any,
        Lit(0x8a),
        Lit(0x26),
        Any,
        Any,
    ];
    let ctrl_pos = find_pattern(tick_win, CTRL_PAT)
        .ok_or_else(|| Error::Unsupported("SDK control-merge pattern not found".into()))?;
    let control_shadow = read_u16(tick_win, ctrl_pos + 4).unwrap();
    let voice_active = read_u16(tick_win, ctrl_pos + 13).unwrap();
    let expired_mask = read_u16(tick_win, ctrl_pos + 23).unwrap();
    let deferred_mask = read_u16(tick_win, ctrl_pos + 27).unwrap();
    let deferred_target = read_u16(tick_win, ctrl_pos + 32).unwrap();
    let deferred_value = read_u16(tick_win, ctrl_pos + 36).unwrap();

    // Channel-3 mute tail: IN AX,84h; CMP AX,96h; JB rel8
    const MUTE_PAT: &[Pat] = &[
        Lit(0xe5),
        Lit(0x84),
        Lit(0x3d),
        Lit(0x96),
        Lit(0x00),
        Lit(0x72),
        Any,
    ];
    let mute_pos = find_pattern(tick_win, MUTE_PAT)
        .ok_or_else(|| Error::Unsupported("SDK channel-3 mute tail pattern not found".into()))?;
    let rel = tick_win[mute_pos + 6] as i8;
    let target_idx = (mute_pos as isize + 7 + isize::from(rel)) as usize;
    let mute_tail = match tick_win.get(target_idx).copied() {
        Some(0x5e) => MuteTail::SavedPointer,
        Some(0xb8) => MuteTail::LivePointer,
        _ => {
            return Err(Error::Unsupported(
                "unrecognized SDK channel-3 mute tail opcode".into(),
            ));
        }
    };

    // Fetch prefix (preceding fetch_off):
    let prefix_start = fetch_off.saturating_sub(0x200);
    let prefix = &seg[prefix_start..fetch_off];

    // Pitch table: XOR AH,AH; MOV BX,<pitch>; ADD BX,AX; MOV BX,CS:[BX]
    const PITCH_PAT: &[Pat] = &[
        Lit(0x32),
        Lit(0xe4),
        Lit(0xbb),
        Any,
        Any,
        Lit(0x03),
        Lit(0xd8),
        Lit(0x2e),
        Lit(0x8b),
        Lit(0x1f),
    ];
    let pitch_pos = find_pattern(prefix, PITCH_PAT)
        .ok_or_else(|| Error::Unsupported("SDK pitch-table reference not found".into()))?;
    let pitch_table = read_u16(prefix, pitch_pos + 3).unwrap();

    // Wave shadow, wave bank, wave RAM:
    const WAVE_PAT: &[Pat] = &[
        Lit(0xbf),
        Any,
        Any,
        Lit(0x8a),
        Lit(0x0e),
        Any,
        Any,
        Lit(0x32),
        Lit(0xed),
        Lit(0x03),
        Lit(0xf9),
        Lit(0x8a),
        Lit(0x44),
        Lit(0x25),
        Lit(0x3a),
        Lit(0x05),
        Lit(0x74),
        Any,
        Lit(0x88),
        Lit(0x05),
        Lit(0x32),
        Lit(0xe4),
        Lit(0x56),
        Lit(0xbe),
        Any,
        Any,
        Lit(0x32),
        Lit(0xe4),
        Lit(0x03),
        Lit(0xf0),
        Lit(0xbf),
        Any,
        Any,
    ];
    let wave_pos = find_pattern(prefix, WAVE_PAT)
        .ok_or_else(|| Error::Unsupported("SDK wave-copy pattern not found".into()))?;
    let wave_shadow = read_u16(prefix, wave_pos + 1).unwrap();
    let wave_bank = read_u16(prefix, wave_pos + 24).unwrap();
    let wave_ram = read_u16(prefix, wave_pos + 31).unwrap();

    // Sweep table: OUT 8Dh,AL; MOV AL,AH; AND AX,0Fh; DEC AX; MOV SI,<sweep>; ADD SI,AX; MOV AL,CS:[SI]; OUT 8Ch,AL
    const SWEEP_PAT: &[Pat] = &[
        Lit(0xe6),
        Lit(0x8d),
        Lit(0x8a),
        Lit(0xc4),
        Lit(0x25),
        Lit(0x0f),
        Lit(0x00),
        Lit(0x48),
        Lit(0xbe),
        Any,
        Any,
        Lit(0x03),
        Lit(0xf0),
        Lit(0x2e),
        Lit(0x8a),
        Lit(0x04),
        Lit(0xe6),
        Lit(0x8c),
    ];
    let sweep_pos = find_pattern(prefix, SWEEP_PAT)
        .ok_or_else(|| Error::Unsupported("SDK sweep-table pattern not found".into()))?;
    let sweep_table = read_u16(prefix, sweep_pos + 9).unwrap();

    // Voice clear / start helper calls at the tail of fetch prefix:
    //   CALL clear_voice; MOV CL,AL; XOR CH,CH; MOV AL,[SI+0Ch]; XOR AH,AH; CALL start_voice; RET
    const VCALL_MID: &[Pat] = &[
        Lit(0x8a),
        Lit(0xc8),
        Lit(0x32),
        Lit(0xed),
        Lit(0x8a),
        Lit(0x44),
        Lit(0x0c),
        Lit(0x32),
        Lit(0xe4),
    ];
    let mut clear_voice_offset = None;
    let mut start_voice_offset = None;
    if let Some(mid) = find_pattern(prefix, VCALL_MID) {
        // Preceding call: far (9A off seg) or near (E8 rel16)
        if mid >= 5 && prefix[mid - 5] == 0x9a {
            clear_voice_offset = read_u16(prefix, mid - 4);
        } else if mid >= 3 && prefix[mid - 3] == 0xe8 {
            let rel = read_u16(prefix, mid - 2).unwrap() as i16;
            clear_voice_offset = Some((prefix_start + mid).wrapping_add(rel as usize) as u16);
        }
        // Following call: far (9A off seg) or near (E8 rel16)
        let after = mid + VCALL_MID.len();
        if after + 5 <= prefix.len() && prefix[after] == 0x9a {
            start_voice_offset = read_u16(prefix, after + 1);
        } else if after + 3 <= prefix.len() && prefix[after] == 0xe8 {
            let rel = read_u16(prefix, after + 1).unwrap() as i16;
            start_voice_offset = Some((prefix_start + after + 3).wrapping_add(rel as usize) as u16);
        }
    }

    let (voice_state, voice_table) =
        discover_voice_links(seg, clear_voice_offset, start_voice_offset)?;

    // Play loop: PUSH CX; MOV [play_index],AX; MOV DI,<track_table>; SHL AX,1; MOV CX,AX; SHL AX,1; ADD AX,CX; ADD DI,AX
    const PLAY_PAT: &[Pat] = &[
        Lit(0x51),
        Lit(0xa3),
        Any,
        Any,
        Lit(0xbf),
        Any,
        Any,
        Lit(0xd1),
        Lit(0xe0),
        Lit(0x8b),
        Lit(0xc8),
        Lit(0xd1),
        Lit(0xe0),
        Lit(0x03),
        Lit(0xc1),
        Lit(0x03),
        Lit(0xf8),
    ];
    let play_pos = find_pattern(seg, PLAY_PAT)
        .ok_or_else(|| Error::Unsupported("SDK play-loop pattern not found".into()))?;
    let play_index = read_u16(seg, play_pos + 2).unwrap();
    let track_table = read_u16(seg, play_pos + 5).unwrap();

    let fetch_variant = match seg.get(fetch_off + 26).copied() {
        Some(0xe9) => FetchVariant::NearJumps,
        Some(0xeb) => FetchVariant::ShortJumps,
        _ => FetchVariant::Unknown,
    };

    Ok(SdkDiscovery {
        segment_file_offset,
        data_segment,
        link: SdkLink {
            slots,
            slot_stride,
            globals: SdkGlobals {
                taken_mask,
                control_shadow,
                channel_index,
                channel_bits,
                phase_counter,
                track_index,
                deferred_mask,
                deferred_value,
                expired_mask,
                first_track,
                route_last,
                voice_active,
                play_index,
            },
            wave_shadow,
            wave_ram,
            deferred_target,
            pitch_table,
            wave_bank,
            envelope_groups,
            envelope_volumes,
            vibrato_table,
            sweep_table,
            track_table,
            voice_state,
            voice_table,
            mute_tail,
        },
        fetch_variant,
        tick_offset: tick_off as u16,
        fetch_offset: fetch_off as u16,
        clear_voice_offset,
        start_voice_offset,
    })
}

fn compute_segment_offset(rom_len: usize, fetch_file: usize, data_segment: u16) -> usize {
    if data_segment >= 0x4000 && (data_segment & 0x0fff) == 0 {
        fetch_file & !0xffff
    } else if data_segment >= 0x4000 {
        let tail = (0x10000usize - usize::from(data_segment)) << 4;
        if let Some(cand) = rom_len.checked_sub(tail) {
            if cand <= fetch_file && fetch_file < cand.saturating_add(0x10000) {
                return cand;
            }
        }
        fetch_file & !0xffff
    } else {
        fetch_file & !0xffff
    }
}

fn discover_voice_links(
    seg: &[u8],
    clear_voice_offset: Option<u16>,
    start_voice_offset: Option<u16>,
) -> Result<(u16, Option<u16>)> {
    // clear_voice: MOV BYTE [voice_active], 0; MOV WORD [voice_state], 0
    const CV_PAT: &[Pat] = &[
        Lit(0xc6),
        Lit(0x06),
        Any,
        Any,
        Lit(0x00),
        Lit(0xc7),
        Lit(0x06),
        Any,
        Any,
        Lit(0x00),
        Lit(0x00),
    ];
    // voice_start table lookup: MOV BX,<voice_table>; MOV AX,[voice_state]; SHL AX,3|4; ADD BX,AX; MOV AX,CS:[BX]
    const VT_PAT_3: &[Pat] = &[
        Lit(0xbb),
        Any,
        Any,
        Lit(0xa1),
        Any,
        Any,
        Lit(0xc1),
        Lit(0xe0),
        Lit(0x03),
        Lit(0x03),
        Lit(0xd8),
        Lit(0x2e),
        Lit(0x8b),
        Lit(0x07),
    ];
    const VT_PAT_4: &[Pat] = &[
        Lit(0xbb),
        Any,
        Any,
        Lit(0xa1),
        Any,
        Any,
        Lit(0xc1),
        Lit(0xe0),
        Lit(0x04),
        Lit(0x03),
        Lit(0xd8),
        Lit(0x2e),
        Lit(0x8b),
        Lit(0x07),
    ];
    // voice_start body: XOR AX,AX; OUT 89h,AL; IN AL,90h; OR AL,22h; OUT 90h,AL
    const VS_BODY_PAT: &[Pat] = &[
        Lit(0x33),
        Lit(0xc0),
        Lit(0xe6),
        Lit(0x89),
        Lit(0xe4),
        Lit(0x90),
        Lit(0x0c),
        Lit(0x22),
        Lit(0xe6),
        Lit(0x90),
    ];

    let mut voice_state = None;
    let mut voice_table = None;

    if let Some(off) = clear_voice_offset.map(usize::from) {
        if off < seg.len() {
            let win = &seg[off..seg.len().min(off + 32)];
            if let Some(pos) = find_pattern(win, CV_PAT) {
                voice_state = read_u16(win, pos + 7);
            }
        }
    }
    if let Some(off) = start_voice_offset.map(usize::from) {
        if off < seg.len() {
            let win = &seg[off..seg.len().min(off + 96)];
            if let Some(pos) = find_pattern(win, VT_PAT_3).or_else(|| find_pattern(win, VT_PAT_4)) {
                voice_table = read_u16(win, pos + 1);
                if voice_state.is_none() {
                    voice_state = read_u16(win, pos + 4);
                }
            }
        }
    }
    if voice_state.is_none() || voice_table.is_none() {
        if let Some(vs_pos) = find_pattern(seg, VS_BODY_PAT) {
            if voice_state.is_none() && vs_pos >= 3 && seg[vs_pos - 3] == 0xa3 {
                voice_state = read_u16(seg, vs_pos - 2);
            }
            if voice_table.is_none() {
                let win = &seg[vs_pos..seg.len().min(vs_pos + 96)];
                if let Some(pos) =
                    find_pattern(win, VT_PAT_3).or_else(|| find_pattern(win, VT_PAT_4))
                {
                    voice_table = read_u16(win, pos + 1);
                    if voice_state.is_none() {
                        voice_state = read_u16(win, pos + 4);
                    }
                }
            }
        }
    }

    let voice_state = voice_state
        .ok_or_else(|| Error::Unsupported("SDK voice_state address not found".into()))?;
    Ok((voice_state, voice_table))
}
