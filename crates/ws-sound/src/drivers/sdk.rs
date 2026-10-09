//! SDK standard two-byte cells. Compact variants need separate layout evidence.
use super::check_count;
use formatkit_core::{bytes_at, Endian, Error, Reader, Result};
use formatkit_layout::U16Field;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackEntry {
    pub slot_offset: u16,
    pub channel: u16,
    pub sequence: u16,
}

/// Four descriptor words loaded by the standard voice-start helper. The high
/// byte of bank_word is preserved; chaining semantics belong to the IRQ pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceDescriptor {
    pub pointer: u16,
    pub bank_word: u16,
    pub length: u16,
    pub next: u16,
}

impl VoiceDescriptor {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        Ok(Self {
            pointer: reader.u16_le()?,
            bank_word: reader.u16_le()?,
            length: reader.u16_le()?,
            next: reader.u16_le()?,
        })
    }
}

/// A single standard SDK track preview: notes/rests, volume and wave selection,
/// A7 wait and reserved no-ops. Control flow, envelopes, sweeps, slides and voice
/// need their linked driver state and are reported as unsupported.
pub fn render_sequence(
    bytes: &[u8],
    pitch_words: &[u8],
    waves: &[crate::Wave],
    channel: u8,
    frames_per_tick: usize,
    max_frames: usize,
) -> Result<formatkit_core::Pcm> {
    use crate::render::{render, Action, Event, MAX_EVENTS, MAX_FRAMES};
    if channel >= 4 || frames_per_tick == 0 {
        return Err(Error::Malformed(
            "invalid SDK preview channel/tick duration".into(),
        ));
    }
    if max_frames > MAX_FRAMES {
        return Err(Error::ResourceLimit {
            resource: "SDK PCM frames",
            requested: max_frames as u64,
            limit: MAX_FRAMES as u64,
        });
    }
    let mut events = vec![
        Event {
            frame: 0,
            action: Action::Write {
                port: 0x91,
                value: 8,
            },
        },
        Event {
            frame: 0,
            action: Action::Write {
                port: 0x90,
                value: 1 << channel,
            },
        },
    ];
    let mut reader = Reader::new(bytes);
    let mut frame = 0usize;
    let mut volume = 0xff;
    let mut bank = crate::WaveBank::default();
    bank.0[usize::from(channel)] = *waves
        .first()
        .ok_or_else(|| Error::Malformed("SDK preview requires a wave".into()))?;
    events.push(Event {
        frame: 0,
        action: Action::Waves(bank),
    });
    for _ in 0..super::MAX_RECORDS {
        let cell = Cell::parse(reader.bytes(2)?)?;
        if cell == Cell::End {
            events.retain(|event| event.frame < frame);
            return render(&events, frame);
        }
        let duration = match cell {
            Cell::Note {
                class,
                octave,
                duration,
            } => {
                let divisor = note_divisor(pitch_words, class, octave, false)?;
                events.push(Event {
                    frame,
                    action: Action::Write {
                        port: 0x80 + channel * 2,
                        value: divisor as u8,
                    },
                });
                events.push(Event {
                    frame,
                    action: Action::Write {
                        port: 0x81 + channel * 2,
                        value: (divisor >> 8) as u8,
                    },
                });
                events.push(Event {
                    frame,
                    action: Action::Write {
                        port: 0x88 + channel,
                        value: volume,
                    },
                });
                usize::from(duration).max(1)
            }
            Cell::Rest { duration } => {
                events.push(Event {
                    frame,
                    action: Action::Write {
                        port: 0x88 + channel,
                        value: 0,
                    },
                });
                usize::from(duration).max(1)
            }
            Cell::Control {
                opcode: 0xa0,
                operand,
            } => {
                volume = operand;
                0
            }
            Cell::Control {
                opcode: 0xa2,
                operand,
            } => {
                bank.0[usize::from(channel)] =
                    *waves
                        .get(usize::from(operand))
                        .ok_or_else(|| Error::InvalidField {
                            what: "SDK wave index",
                            value: operand.into(),
                        })?;
                events.push(Event {
                    frame,
                    action: Action::Waves(bank),
                });
                0
            }
            Cell::Control {
                opcode: 0xa7,
                operand,
            } => usize::from(operand).max(1),
            Cell::Reserved { .. } => 0,
            _ => {
                return Err(Error::Unsupported(
                    "SDK preview cell needs linked driver effects/control flow".into(),
                ))
            }
        };
        frame = frame
            .checked_add(
                duration
                    .checked_mul(frames_per_tick)
                    .ok_or_else(|| Error::Malformed("SDK duration overflow".into()))?,
            )
            .ok_or_else(|| Error::Malformed("SDK timeline overflow".into()))?;
        if frame > max_frames {
            return Err(Error::ResourceLimit {
                resource: "SDK PCM frames",
                requested: frame as u64,
                limit: max_frames as u64,
            });
        }
        if events.len() > MAX_EVENTS {
            return Err(Error::ResourceLimit {
                resource: "SDK preview events",
                requested: events.len() as u64,
                limit: MAX_EVENTS as u64,
            });
        }
    }
    Err(Error::ResourceLimit {
        resource: "SDK preview cells",
        requested: super::MAX_RECORDS as u64 + 1,
        limit: super::MAX_RECORDS as u64,
    })
}

pub fn tracks(bytes: &[u8], count: usize) -> Result<Vec<TrackEntry>> {
    let mut reader = Reader::new(bytes_at(bytes, 0, check_count(count, 6)?)?);
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let entry = TrackEntry {
            slot_offset: reader.u16_le()?,
            channel: reader.u16_le()?,
            sequence: reader.u16_le()?,
        };
        if !entry.slot_offset.is_multiple_of(0x2a)
            || entry.slot_offset / 0x2a >= 8
            || entry.channel > 3
        {
            return Err(Error::Malformed(
                "invalid standard SDK track slot/channel".into(),
            ));
        }
        result.push(entry);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Note { class: u8, octave: u8, duration: u8 },
    Rest { duration: u8 },
    Control { opcode: u8, operand: u8 },
    Loop { count: u8, operand: u8 },
    Envelope { group: u8, period: u8 },
    Slide { amount: i8, period: u8 },
    Marker { operand: u8 },
    Reserved { opcode: u8, operand: u8 },
    End,
}

impl Cell {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let opcode = reader.u8()?;
        let operand = reader.u8()?;
        Ok(match opcode {
            0x00..=0x9f => {
                if opcode & 15 >= 12 {
                    Self::Rest { duration: operand }
                } else {
                    Self::Note {
                        class: opcode & 15,
                        octave: opcode >> 4,
                        duration: operand,
                    }
                }
            }
            0xa0..=0xa5 | 0xa7 | 0xae | 0xaf => Self::Control { opcode, operand },
            0xb0..=0xbf => Self::Loop {
                count: opcode & 15,
                operand,
            },
            0xc0..=0xcf => Self::Envelope {
                group: opcode & 15,
                period: operand,
            },
            0xd0..=0xef => Self::Slide {
                amount: if opcode < 0xe0 {
                    (opcode & 15) as i8
                } else {
                    -((opcode & 15) as i8)
                },
                period: operand,
            },
            0xfd => Self::Marker { operand },
            0xff => Self::End,
            _ => Self::Reserved { opcode, operand },
        })
    }
}

pub fn note_divisor(pitch_words: &[u8], class: u8, octave: u8, detuned: bool) -> Result<u16> {
    if class >= 12 || octave >= 10 {
        return Err(Error::InvalidField {
            what: "SDK pitch class/octave",
            value: u64::from(class) << 8 | u64::from(octave),
        });
    }
    let offset = usize::from(class) * 2 + if detuned { 24 } else { 0 };
    let period = U16Field::new(offset, Endian::Little).read(pitch_words)? >> octave;
    Ok(2048u16.wrapping_sub(period) & 0x7ff)
}

/// Linear cell parsing, with a bounded count and a required terminator.
pub fn sequence(bytes: &[u8], limit: usize) -> Result<Vec<Cell>> {
    check_count(limit, 2)?;
    let mut reader = Reader::new(bytes);
    let mut result = Vec::new();
    for _ in 0..limit {
        let cell = Cell::parse(reader.bytes(2)?)?;
        result.push(cell);
        if cell == Cell::End {
            return Ok(result);
        }
    }
    Err(Error::ResourceLimit {
        resource: "SDK sequence cells",
        requested: limit as u64 + 1,
        limit: limit as u64,
    })
}
