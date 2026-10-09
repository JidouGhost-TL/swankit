use super::{check_count, RomView};
use crate::hardware::DMA_RATES;
use formatkit_core::{Error, ErrorContext, Reader, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    A,
    B,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackPointer {
    pub slot: u8,
    pub segment: u16,
    pub offset: u16,
    pub bank: Option<u8>,
}

impl TrackPointer {
    pub fn channel(self, dialect: Dialect) -> u8 {
        match dialect {
            Dialect::B => (self.slot ^ 3) & 3,
            _ => self.slot & 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub mask: u8,
    pub priority: u8,
    pub segment: u16,
    pub bank: Option<u8>,
    pub tracks: Vec<TrackPointer>,
}

impl Header {
    /// A: mask, priority, segment; B: mask, priority; C: mask, priority,
    /// bank, segment. B assigns pointers from mask bit 7 down, A/C from bit 0 up.
    pub fn parse(bytes: &[u8], dialect: Dialect, fixed_segment: u16) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let mask = reader.u8()?;
        let priority = reader.u8()?;
        let bank = if dialect == Dialect::C {
            Some(reader.u8()?)
        } else {
            None
        };
        let segment = if dialect == Dialect::B {
            fixed_segment
        } else {
            reader.u16_le()?
        };
        if mask == 0 && dialect == Dialect::C {
            return Err(Error::Unsupported(
                "mask-zero byte-C headers use the PCM descriptor grammar".into(),
            ));
        }
        let mut tracks = Vec::with_capacity(mask.count_ones() as usize);
        for ordinal in 0..8 {
            let bit = if dialect == Dialect::B {
                7 - ordinal
            } else {
                ordinal
            };
            if mask & (1 << bit) != 0 {
                tracks.push(TrackPointer {
                    slot: ordinal,
                    segment,
                    bank,
                    offset: reader.u16_le()?,
                });
            }
        }
        Ok(Self {
            mask,
            priority,
            segment,
            bank,
            tracks,
        })
    }
}

pub fn table(
    view: RomView<'_>,
    table_offset: usize,
    count: usize,
    header_segment: u16,
    dialect: Dialect,
    fixed_segment: u16,
) -> Result<Vec<Header>> {
    let mut reader = Reader::new(view.file_span(table_offset, check_count(count, 2)?)?);
    let mut headers = Vec::with_capacity(count);
    for index in 0..count {
        let offset = reader.u16_le()?;
        let first = view.span(header_segment, offset, 1)?[0];
        let prefix = match dialect {
            Dialect::A => 4,
            Dialect::B => 2,
            Dialect::C => 5,
        };
        let bytes = view.span(
            header_segment,
            offset,
            prefix + 2 * first.count_ones() as usize,
        )?;
        headers.push(
            Header::parse(bytes, dialect, fixed_segment)
                .map_err(|e| e.context(ErrorContext::Record { index }))?,
        );
    }
    Ok(headers)
}

/// At most two bytes, seven bits per byte, most significant group first.
/// The second continuation bit never causes a third read.
pub fn varlen(reader: &mut Reader<'_>) -> Result<u16> {
    let first = reader.u8()?;
    if first & 0x80 == 0 {
        return Ok(u16::from(first));
    }
    Ok((u16::from(first & 127) << 7) | u16::from(reader.u8()? & 127))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmCommand {
    pub segment: u16,
    pub offset: u16,
    pub length: u16,
    pub repeat: bool,
    pub reverse: bool,
    pub rate: u32,
}

impl PcmCommand {
    pub fn parse(reader: &mut Reader<'_>) -> Result<Self> {
        let segment = reader.u16_le()?;
        let offset = reader.u16_le()?;
        let length = reader.u16_le()?;
        let repeat = reader.u8()? == 0;
        let reverse = reader.u8()? & 0x40 != 0;
        let rate = DMA_RATES[(reader.u8()? & 3) as usize];
        Ok(Self {
            segment,
            offset,
            length,
            repeat,
            reverse,
            rate,
        })
    }
}

/// An instruction preserves raw operand bytes and exact consumed extent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    Note {
        pitch: u16,
        on: u16,
        off: u16,
    },
    PackedNote {
        note: u8,
        duration: u8,
        rest: bool,
    },
    Command {
        opcode: u8,
        operands: Vec<u8>,
    },
    Trigger {
        header: u16,
        on: u16,
        off: u16,
    },
    Pcm(PcmCommand),
    Direct {
        wait: u16,
        opcode: u8,
        operands: Vec<u8>,
    },
    DirectEnd {
        wait: u16,
    },
    End,
}

/// Mode state belongs to a track, not to a shared decoder.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mode {
    pub direct: bool,
    pub direct_divisor: bool,
    pub channel: u8,
    pub call_depth_bytes: u8,
}

pub fn instruction(reader: &mut Reader<'_>, dialect: Dialect, mode: Mode) -> Result<Instruction> {
    if mode.channel >= 4 {
        return Err(Error::InvalidField {
            what: "byte track channel",
            value: mode.channel.into(),
        });
    }
    if dialect == Dialect::A && mode.direct {
        let wait = varlen(reader)?;
        let opcode = reader.u8()?;
        let len = match opcode {
            0 => 2,
            1 | 2 | 4 => 1,
            3 | 5 | 7 => 0,
            6 => 4,
            8 => return Ok(Instruction::DirectEnd { wait }),
            _ => return Err(Error::Unsupported(format!("direct opcode {opcode:02x}"))),
        };
        return Ok(Instruction::Direct {
            wait,
            opcode,
            operands: reader.bytes(len)?.to_vec(),
        });
    }
    let opcode = reader.u8()?;
    if opcode == 0xff {
        return Ok(Instruction::End);
    }
    if dialect == Dialect::B {
        if opcode < 0xe0 {
            return Ok(Instruction::PackedNote {
                note: opcode >> 4,
                duration: opcode & 15,
                rest: opcode >= 0xc0,
            });
        }
        let len = match opcode {
            0xe0..=0xe7 | 0xf5 | 0xf9..=0xfc => 0,
            0xea | 0xee => 3,
            0xf4 | 0xf7 | 0xf8 => 2,
            _ => 1,
        };
        return Ok(Instruction::Command {
            opcode,
            operands: reader.bytes(len)?.to_vec(),
        });
    }
    if opcode < 0x80 {
        let pitch = if dialect == Dialect::C && mode.direct_divisor {
            ((u16::from(opcode) << 8) | u16::from(reader.u8()?)) & 0x7ff
        } else {
            opcode.into()
        };
        return Ok(Instruction::Note {
            pitch,
            on: varlen(reader)?,
            off: varlen(reader)?,
        });
    }
    if opcode == 0x9b || dialect == Dialect::C && matches!(opcode, 0x8a | 0x90 | 0x91) {
        return Ok(Instruction::End);
    }
    if opcode == 0x99 && dialect == Dialect::A {
        return Ok(Instruction::Trigger {
            header: reader.u16_le()?,
            on: varlen(reader)?,
            off: varlen(reader)?,
        });
    }
    if opcode == 0x8f && dialect == Dialect::A && mode.channel == 1 && mode.call_depth_bytes < 6 {
        return Ok(Instruction::Pcm(PcmCommand::parse(reader)?));
    }
    let len = match opcode {
        0x80 | 0x81 | 0x83..=0x85 | 0x87 | 0x96 => 1,
        0x82 | 0x8c | 0x8e => 4,
        0x88 => 2,
        0x8a if dialect == Dialect::A => 5,
        0x8f if dialect == Dialect::A => 8, // observed skip-path quirk
        0x92 | 0x94 => {
            if dialect == Dialect::C {
                2
            } else {
                4
            }
        }
        0x86 | 0x89 | 0x8b | 0x8d | 0x8f | 0x90 | 0x91 | 0x93 | 0x95 | 0x97..=0x9a => 0,
        0x9c..=0xa3 if dialect == Dialect::A => [2, 1, 1, 0, 1, 0, 4, 0][(opcode - 0x9c) as usize],
        _ => {
            return Err(Error::Unsupported(format!(
                "byte sequence opcode {opcode:02x}"
            )))
        }
    };
    Ok(Instruction::Command {
        opcode,
        operands: reader.bytes(len)?.to_vec(),
    })
}

/// The historical 11-byte PCM-directory pattern is an SE sequence: opcode,
/// nine operand bytes, end. Rate comes from the command, never from a guess.
pub fn pcm_se(bytes: &[u8]) -> Result<PcmCommand> {
    let mut reader = Reader::new(bytes);
    if reader.u8()? != 0x8f {
        return Err(Error::Malformed("PCM SE must start with its opcode".into()));
    }
    let command = PcmCommand::parse(&mut reader)?;
    if reader.u8()? != 0xff {
        return Err(Error::Malformed("PCM SE lacks an end opcode".into()));
    }
    Ok(command)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteCPcm {
    pub flags: u8,
    pub bank: u8,
    pub segment: u16,
    pub offset: u16,
    pub length: u16,
    pub repeat: bool,
    pub rate: u32,
}

impl ByteCPcm {
    /// Mask-zero grammar has different positions from the music header.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.u8()? != 0 {
            return Err(Error::Malformed(
                "byte-C PCM header mask must be zero".into(),
            ));
        }
        let flags = reader.u8()?;
        let bank = reader.u8()?;
        let segment = reader.u16_le()?;
        let offset = reader.u16_le()?;
        let repeat = reader.u8()? == 0;
        let length = reader.u16_le()?;
        Ok(Self {
            flags,
            bank,
            segment,
            offset,
            length,
            repeat,
            rate: DMA_RATES[(3 - (flags & 3)) as usize],
        })
    }
}
