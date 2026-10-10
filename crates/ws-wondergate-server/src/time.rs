// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::malformed;
use formatkit_core::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
    time::{SystemTime, UNIX_EPOCH},
};

/// UTC seconds since Unix epoch, restricted to calendar years 1..9999 (also JST).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Timestamp(i64);
const MIN_TIME: i64 = -62135596800;
const MAX_TIME: i64 = 253402300799 - 9 * 3600;
impl TryFrom<i64> for Timestamp {
    type Error = formatkit_core::Error;
    fn try_from(seconds: i64) -> Result<Self> {
        if !(MIN_TIME..=MAX_TIME).contains(&seconds) {
            return Err(malformed("timestamp outside calendar bounds"));
        }
        Ok(Self(seconds))
    }
}
impl From<Timestamp> for i64 {
    fn from(value: Timestamp) -> Self {
        value.0
    }
}
fn leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn month_days(year: i32, month: u32) -> u32 {
    match month {
        2 => 28 + u32::from(leap(year)),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
impl Timestamp {
    pub fn from_utc(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<Self> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > month_days(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(malformed("invalid calendar timestamp"));
        }
        let mut days = 0i64;
        if year >= 1970 {
            for y in 1970..year {
                days += if leap(y) { 366 } else { 365 };
            }
        } else {
            for y in year..1970 {
                days -= if leap(y) { 366 } else { 365 };
            }
        }
        for m in 1..month {
            days += i64::from(month_days(year, m));
        }
        days += i64::from(day - 1);
        Self::try_from(days * 86400 + i64::from(hour * 3600 + minute * 60 + second))
    }
    /// YYYYMMDDhhmmss in UTC, as used by fixed-clock CLI/corpus arguments.
    pub fn parse_fixed(text: &str) -> Result<Self> {
        if text.len() != 14 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(malformed("clock needs 14 UTC digits"));
        }
        let part = |range: std::ops::Range<usize>| {
            text[range]
                .parse::<u32>()
                .map_err(|_| malformed("invalid clock"))
        };
        Self::from_utc(
            part(0..4)? as i32,
            part(4..6)?,
            part(6..8)?,
            part(8..10)?,
            part(10..12)?,
            part(12..14)?,
        )
    }
    fn parts(self, offset: i64) -> (i32, u32, u32, i64, i64, i64) {
        let seconds = self.0 + offset;
        let mut days = seconds.div_euclid(86400);
        let time = seconds.rem_euclid(86400);
        let mut year = 1970;
        while days < 0 {
            year -= 1;
            days += if leap(year) { 366 } else { 365 };
        }
        loop {
            let count = if leap(year) { 366 } else { 365 };
            if days < count {
                break;
            }
            days -= count;
            year += 1;
        }
        let mut month = 1;
        while days >= i64::from(month_days(year, month)) {
            days -= i64::from(month_days(year, month));
            month += 1;
        }
        (
            year,
            month,
            days as u32 + 1,
            time / 3600,
            time / 60 % 60,
            time % 60,
        )
    }
    pub fn iso_utc(self) -> String {
        let (y, m, d, h, n, s) = self.parts(0);
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{n:02}:{s:02}+00:00")
    }
    pub fn jst_digits(self) -> String {
        let (y, m, d, h, n, s) = self.parts(9 * 3600);
        format!("{y:04}{m:02}{d:02}{h:02}{n:02}{s:02}")
    }
}
pub trait Clock: Send + Sync {
    fn now_utc(&self) -> Timestamp;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now_utc(&self) -> Timestamp {
        let value = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(MAX_TIME),
            Err(error) => -i64::try_from(error.duration().as_secs()).unwrap_or(-MIN_TIME),
        };
        Timestamp(value.clamp(MIN_TIME, MAX_TIME))
    }
}
#[derive(Clone, Copy)]
pub struct FixedClock(pub Timestamp);
impl Clock for FixedClock {
    fn now_utc(&self) -> Timestamp {
        self.0
    }
}

/// Entropy failure is a returned error, never a weak random fallback or panic.
pub trait Rng: Send {
    fn fill(&mut self, buf: &mut [u8]) -> io::Result<()>;
}
pub struct OsRng {
    source: File,
}
impl OsRng {
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                source: File::open("/dev/urandom")?,
            })
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "OS entropy source unavailable on this platform",
            ))
        }
    }
}
impl Rng for OsRng {
    fn fill(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.source.read_exact(buf)
    }
}
/// SplitMix64, for repeatable synthetic sessions, independent of fill chunking.
pub struct SeededRng {
    state: u64,
    pending: [u8; 8],
    remaining: usize,
}
impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed,
            pending: [0; 8],
            remaining: 0,
        }
    }
}
impl Rng for SeededRng {
    fn fill(&mut self, buf: &mut [u8]) -> io::Result<()> {
        for byte in buf {
            if self.remaining == 0 {
                self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
                let mut value = self.state;
                value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
                self.pending = (value ^ (value >> 31)).to_le_bytes();
                self.remaining = 8;
            }
            *byte = self.pending[8 - self.remaining];
            self.remaining -= 1;
        }
        Ok(())
    }
}
