//! Four-byte-cell driver: directory groups, 13-byte songs, tracks and PCM.
use super::{check_count, RomView, MAX_RECORDS};
use formatkit_core::{bytes_at, Endian, Error, Reader, Result};
use formatkit_layout::{U16Field, U8Field};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub segment: u16,
    pub first_id: u8,
}

pub fn groups(bytes: &[u8], count: usize) -> Result<Vec<Group>> {
    let mut reader = Reader::new(bytes_at(bytes, 0, check_count(count, 4)?)?);
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(Group {
            segment: reader.u16_le()?,
            first_id: reader.u8()?,
        });
        reader.skip(1)?;
    }
    if result
        .windows(2)
        .any(|pair| pair[0].first_id >= pair[1].first_id)
    {
        return Err(Error::Malformed("song group ids must increase".into()));
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Song {
    pub tracks: [u16; 4],
    pub tempo: u16,
    pub wave_table: u16,
    pub flag: u8,
}

impl Song {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let record = bytes_at(bytes, 0, 13)?;
        let field = |offset| U16Field::new(offset, Endian::Little).read(record);
        Ok(Self {
            tracks: [field(0)?, field(2)?, field(4)?, field(6)?],
            tempo: field(8)?,
            wave_table: field(10)?,
            flag: U8Field::new(12).read(record)?,
        })
    }
    pub fn pcm(self, view: RomView<'_>, segment: u16) -> Result<Option<PcmDescriptor>> {
        let mut reader = Reader::new(view.span(segment, self.tracks[0], 4)?);
        if reader.u16_le()? != 0x8003 {
            return Ok(None);
        }
        let timer_rate = reader.u8()?;
        let length = Reader::new(view.span(segment, self.tracks[2], 2)?).u16_le()?;
        if length == 0 {
            return Err(Error::Malformed("empty cell-driver PCM sample".into()));
        }
        view.span(segment, self.tracks[1], usize::from(length))?;
        Ok(Some(PcmDescriptor {
            offset: self.tracks[1],
            length,
            timer_rate,
        }))
    }
    pub fn waves(self, view: RomView<'_>, segment: u16) -> Result<[u8; 4]> {
        let data = view.span(segment, self.wave_table, 4)?;
        Ok([data[0], data[1], data[2], data[3]])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmDescriptor {
    pub offset: u16,
    pub length: u16,
    pub timer_rate: u8,
}

/// Timer state outside this descriptor determines the sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Cell {
    Note { duration: u8, key: u8, divisor: u16 },
    Release { duration: u8, divisor: u16 },
    Command { opcode: u8, operand: u16 },
    Jump(u16),
    End,
}

impl Cell {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let duration = reader.u8()?;
        let key = reader.u8()?;
        let divisor = reader.u16_le()?;
        if key == 0x80 {
            if duration > 11 {
                return Err(Error::InvalidField {
                    what: "cell command",
                    value: duration.into(),
                });
            }
            if duration == 2 || duration >= 5 {
                return Ok(Self::Command {
                    opcode: duration,
                    operand: divisor,
                });
            }
        }
        if duration == 0xff {
            return Ok(if divisor == 0xffff {
                Self::End
            } else {
                Self::Jump(divisor)
            });
        }
        if key == 0 || key == 0xff {
            return Ok(Self::Release { duration, divisor });
        }
        Ok(Self::Note {
            duration,
            key,
            divisor,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub empty: bool,
    pub cells: Vec<[u8; 4]>,
    pub jump: Option<u16>,
}

/// Static linear extent through the first jump/end; jumps remain explicit.
pub fn track(view: RomView<'_>, segment: u16, offset: u16, limit: usize) -> Result<Track> {
    check_count(limit, 4)?;
    let first = view.span(segment, offset, 8)?;
    if U16Field::new(4, Endian::Little).read(first)? == 0xffff {
        return Ok(Track {
            empty: true,
            cells: vec![],
            jump: None,
        });
    }
    let mut cells = Vec::new();
    let mut current = usize::from(offset);
    for _ in 0..limit.min(MAX_RECORDS) {
        let position = u16::try_from(current)
            .map_err(|_| Error::Malformed("cell track wraps its segment".into()))?;
        let data = view.span(segment, position, 4)?;
        let cell = Cell::parse(data)?;
        cells.push([data[0], data[1], data[2], data[3]]);
        current += 4;
        match cell {
            Cell::End => {
                return Ok(Track {
                    empty: false,
                    cells,
                    jump: None,
                })
            }
            Cell::Jump(target) => {
                view.span(segment, target, 4)?;
                return Ok(Track {
                    empty: false,
                    cells,
                    jump: Some(target),
                });
            }
            _ => {}
        }
    }
    Err(Error::ResourceLimit {
        resource: "cell track steps",
        requested: limit as u64 + 1,
        limit: limit as u64,
    })
}
