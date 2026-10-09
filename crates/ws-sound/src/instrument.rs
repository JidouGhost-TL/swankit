//! Bounded wave banks and decoded byte-driver envelope records.
use crate::{
    drivers::{byte::varlen, check_count},
    Wave,
};
use formatkit_core::{bytes_at, Error, Reader, Result};

pub fn wave_bank(bytes: &[u8], count: usize) -> Result<Vec<Wave>> {
    let mut reader = Reader::new(bytes_at(bytes, 0, check_count(count, 16)?)?);
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(Wave::decode(reader.bytes(16)?)?);
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchEnvelope {
    pub delay: u16,
    pub steps: Vec<(u8, i8)>,
}

impl PitchEnvelope {
    /// Initial two-group varlen delay, ticks/delta pairs, one-byte FF end.
    pub fn parse(reader: &mut Reader<'_>, limit: usize) -> Result<Self> {
        check_count(limit, 2)?;
        let delay = varlen(reader)?;
        let mut steps = Vec::new();
        for index in 0..=limit {
            let ticks = reader.u8()?;
            if ticks == 0xff {
                return Ok(Self { delay, steps });
            }
            if index == limit {
                break;
            }
            steps.push((ticks, reader.u8()? as i8));
        }
        Err(Error::ResourceLimit {
            resource: "pitch envelope steps",
            requested: limit as u64 + 1,
            limit: limit as u64,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeEnvelope {
    pub first_state: u8,
    /// State 1..5 pairs: parameter, next state. Table mode 6..15:
    /// level, ticks pairs through level 10h. High state flags remain visible.
    pub steps: Vec<(u8, u8)>,
    pub table: Vec<(u8, u8)>,
}

impl VolumeEnvelope {
    pub fn parse(reader: &mut Reader<'_>, limit: usize) -> Result<Self> {
        check_count(limit, 2)?;
        let first_state = reader.u8()?;
        let mut raw_state = first_state;
        let mut result = Self {
            first_state,
            steps: vec![],
            table: vec![],
        };
        loop {
            let state = raw_state & 31;
            if state == 0 || state >= 16 {
                return Ok(result);
            }
            if result.steps.len() + result.table.len() >= limit {
                break;
            }
            if state >= 6 {
                // In table mode the state's own byte is the first volume.
                let mut level = raw_state;
                loop {
                    if level == 0x10 {
                        return Ok(result);
                    }
                    if result.steps.len() + result.table.len() >= limit {
                        break;
                    }
                    result.table.push((level, reader.u8()?));
                    level = reader.u8()?;
                }
                break;
            }
            let parameter = reader.u8()?;
            let next = reader.u8()?;
            result.steps.push((parameter, next));
            raw_state = next;
        }
        Err(Error::ResourceLimit {
            resource: "volume envelope steps",
            requested: limit as u64 + 1,
            limit: limit as u64,
        })
    }
}
