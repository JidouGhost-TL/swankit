//! Bounded instruction encodings, not a disassembler or a source profile.
//! Encoding reference: Intel 8086 Family User's Manual, instruction reference.
use formatkit_core::Reader;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceStatus {
    RawByteMatch,
    RetainedInstructionsCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Captures {
    IndexedEightWordCopy {
        slot_base: u16,
        index_address: u16,
        source_base: u16,
        source_segment_literal: u16,
        destination_base: u16,
        destination_segment: DestinationSegment,
        copy_words: u8,
        index_shift_width_bits: u8,
        preceding_shift4_store: bool,
    },
    TokenDistance12Length4Plus3 {
        pointer_bp_displacement: i8,
        second_byte_register: SecondByteRegister,
        pointer_increment_bits: u8,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DestinationSegment {
    DS,
    SS,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecondByteRegister {
    AL,
    DL,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Variant {
    DestinationDs,
    DestinationSs,
    AlSiAx,
    DlCxDx,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub pattern: String,
    pub version: u16,
    pub variant: Variant,
    pub source_status: SourceStatus,
    pub captures: Captures,
}

fn word(src: &[u8], at: usize) -> Option<u16> {
    Reader::new(src.get(at..at + 2)?).u16_le().ok()
}

pub(super) fn table(src: &[u8], preceding: &[u8]) -> Option<(usize, Match)> {
    const CORE: [u8; 43] = [
        0xbf, 0, 0, 0x8a, 0x0e, 0, 0, 0x32, 0xed, 0x03, 0xf9, 0x88, 0x05, 0x56, 0xbe, 0, 0, 0x32,
        0xe4, 0x03, 0xf0, 0xbf, 0, 0, 0xa0, 0, 0, 0xc0, 0xe0, 0x04, 0x03, 0xf8, 0xb8, 0, 0, 0x8e,
        0xc0, 0xb9, 8, 0, 0x26, 0x8b, 0x04,
    ];
    let head = src.get(..43)?;
    if head
        .iter()
        .zip(CORE)
        .enumerate()
        .any(|(at, (&a, b))| ![1, 2, 5, 6, 15, 16, 22, 23, 25, 26, 33, 34].contains(&at) && a != b)
        || word(src, 5)? != word(src, 25)?
    {
        return None;
    }
    let ss = *src.get(43)? == 0x36;
    let at = 43 + usize::from(ss);
    let tail = src.get(at..at + 11)?;
    if tail[..9] != [0x89, 0x05, 0x83, 0xc6, 2, 0x83, 0xc7, 2, 0xe2]
        || tail[10] != 0x5e
        || (at + 10) as isize + (tail[9] as i8) as isize != 40
    {
        return None;
    }
    Some((
        at + 11,
        Match {
            pattern: "indexed-eight-word-copy".into(),
            version: 1,
            variant: if ss {
                Variant::DestinationSs
            } else {
                Variant::DestinationDs
            },
            source_status: SourceStatus::RawByteMatch,
            captures: Captures::IndexedEightWordCopy {
                slot_base: word(src, 1)?,
                index_address: word(src, 5)?,
                source_base: word(src, 15)?,
                source_segment_literal: word(src, 33)?,
                destination_base: word(src, 22)?,
                destination_segment: if ss {
                    DestinationSegment::SS
                } else {
                    DestinationSegment::DS
                },
                copy_words: 8,
                index_shift_width_bits: 8,
                preceding_shift4_store: preceding
                    .ends_with(&[0x26, 0x8a, 0x47, 1, 0xc0, 0xe0, 4, 0x88, 0x44, 0x25]),
            },
        },
    ))
}

pub(super) fn token(src: &[u8]) -> Option<(usize, Match)> {
    const CORE: [u8; 38] = [
        0x4b, 0x26, 0x8a, 0x0f, 0xff, 0x46, 0, 0xc4, 0x5e, 0, 0x4b, 0x26, 0x8a, 0, 0x32, 0xed,
        0xd1, 0xe1, 0xd1, 0xe1, 0xd1, 0xe1, 0xd1, 0xe1, 0x8a, 0, 0x32, 0xff, 0xd1, 0xeb, 0xd1,
        0xeb, 0xd1, 0xeb, 0xd1, 0xeb, 0x03, 0xcb,
    ];
    let head = src.get(..38)?;
    if head
        .iter()
        .zip(CORE)
        .enumerate()
        .any(|(at, (&a, b))| ![6, 9, 13, 25].contains(&at) && a != b)
        || head[6] != head[9]
    {
        return None;
    }
    let (tail, lane): (&[u8], _) = match (head[13], head[25]) {
        (0x07, 0xd8) => (
            &[0x8b, 0xf1, 0x24, 0x0f, 0x32, 0xe4, 0x05, 3, 0],
            SecondByteRegister::AL,
        ),
        (0x17, 0xda) => (
            &[0x80, 0xe2, 0x0f, 0x32, 0xf6, 0x83, 0xc2, 3],
            SecondByteRegister::DL,
        ),
        _ => return None,
    };
    if src.get(38..38 + tail.len())? != tail {
        return None;
    }
    Some((
        38 + tail.len(),
        Match {
            pattern: "token-distance12-length4-plus3".into(),
            version: 1,
            variant: if lane == SecondByteRegister::AL {
                Variant::AlSiAx
            } else {
                Variant::DlCxDx
            },
            source_status: SourceStatus::RawByteMatch,
            captures: Captures::TokenDistance12Length4Plus3 {
                pointer_bp_displacement: head[6] as i8,
                second_byte_register: lane,
                pointer_increment_bits: 16,
            },
        },
    ))
}

/// Exact instruction lengths; source joins must retain each boundary, not just bytes.
pub(super) fn instruction_lengths(m: &Match) -> Vec<usize> {
    match m.captures {
        Captures::IndexedEightWordCopy {
            destination_segment,
            ..
        } => {
            let mut n = vec![3, 4, 2, 2, 2, 1, 3, 2, 2, 3, 3, 3, 2, 3, 2, 3, 3];
            n.extend([
                if destination_segment == DestinationSegment::SS {
                    3
                } else {
                    2
                },
                3,
                3,
                2,
                1,
            ]);
            n
        }
        Captures::TokenDistance12Length4Plus3 {
            second_byte_register,
            ..
        } => {
            let mut n = vec![1, 3, 3, 3, 1, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];
            n.extend(if second_byte_register == SecondByteRegister::AL {
                vec![2, 2, 2, 3]
            } else {
                vec![3, 2, 3]
            });
            n
        }
    }
}
