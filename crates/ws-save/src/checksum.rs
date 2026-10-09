//! Explicit arithmetic variants from independently documented save grammars.
use formatkit_core::{Endian, Error, Result};
use formatkit_layout::{U16Field, U8Field};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Byte,
    Word(Endian),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    Sum,
    Xor,
    RotateThenSum(u32),
    SumThenRotate(u32),
    WeightedOffset,
}

/// Fold a bounded byte region; offsets in a weighted sum include `offset_base`.
/// Mask width, seed and xor-out are explicit, so NOT and key-mixed variants
/// cannot be confused with an unseeded sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub unit: Unit,
    pub fold: Fold,
    pub bits: u32,
    pub seed: u64,
    pub xor_out: u64,
    pub offset_base: u64,
}

impl Rule {
    pub fn compute(self, bytes: &[u8]) -> Result<u64> {
        if !(1..=64).contains(&self.bits) {
            return Err(Error::Malformed("checksum width".into()));
        }
        let mask = u64::MAX >> (64 - self.bits);
        let width = match self.unit {
            Unit::Byte => 1,
            Unit::Word(_) => 2,
        };
        if !bytes.len().is_multiple_of(width) {
            return Err(Error::Malformed("odd word checksum extent".into()));
        }
        let rotate = |v: u64, n: u32| {
            let n = n % self.bits;
            if n == 0 {
                v & mask
            } else {
                ((v << n) | (v >> (self.bits - n))) & mask
            }
        };
        let mut value = self.seed & mask;
        for offset in (0..bytes.len()).step_by(width) {
            let x = match self.unit {
                Unit::Byte => u64::from(U8Field::new(offset).read(bytes)?),
                Unit::Word(e) => u64::from(U16Field::new(offset, e).read(bytes)?),
            };
            value = match self.fold {
                Fold::Sum => value.wrapping_add(x),
                Fold::Xor => value ^ x,
                Fold::RotateThenSum(n) => rotate(value, n).wrapping_add(x),
                Fold::SumThenRotate(n) => rotate(value.wrapping_add(x) & mask, n),
                Fold::WeightedOffset => {
                    value.wrapping_add(x.wrapping_mul(self.offset_base.wrapping_add(offset as u64)))
                }
            } & mask;
        }
        Ok((value ^ self.xor_out) & mask)
    }
}

/// Reflected CRC family. KERMIT and X-25 use the same 0x8408 polynomial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crc16 {
    pub polynomial: u16,
    pub seed: u16,
    pub xor_out: u16,
    pub swap_bytes: bool,
}

impl Crc16 {
    pub const KERMIT: Self = Self {
        polynomial: 0x8408,
        seed: 0,
        xor_out: 0,
        swap_bytes: false,
    };
    pub const X25: Self = Self {
        polynomial: 0x8408,
        seed: 0xffff,
        xor_out: 0xffff,
        swap_bytes: false,
    };
    pub fn compute(self, bytes: &[u8]) -> u16 {
        let mut c = self.seed;
        for &byte in bytes {
            c ^= u16::from(byte);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ self.polynomial
                } else {
                    c >> 1
                };
            }
        }
        c ^= self.xor_out;
        if self.swap_bytes {
            c.swap_bytes()
        } else {
            c
        }
    }
    /// Table-driven counterpart for parity with table-based save algorithms.
    pub fn compute_table(self, bytes: &[u8]) -> u16 {
        let mut table = [0u16; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u16;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ self.polynomial
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        let mut c = self.seed;
        for &byte in bytes {
            c = (c >> 8) ^ table[usize::from((c ^ u16::from(byte)) & 255)];
        }
        c ^= self.xor_out;
        if self.swap_bytes {
            c.swap_bytes()
        } else {
            c
        }
    }
}

/// XOR in the low byte, sum modulo 256 in the high byte.
pub fn xor_sum8(bytes: &[u8]) -> u16 {
    let (mut x, mut sum) = (0u8, 0u8);
    for &byte in bytes {
        x ^= byte;
        sum = sum.wrapping_add(byte);
    }
    u16::from(x) | (u16::from(sum) << 8)
}

/// A word permutation with per-position masks, for scrambled save records.
/// It is invertible only when each source word occurs once.
pub fn scramble_words(bytes: &[u8], order: &[usize], masks: &[u16]) -> Result<Vec<u8>> {
    if order.len() != masks.len()
        || bytes.len() / 2 != order.len()
        || !bytes.len().is_multiple_of(2)
    {
        return Err(Error::Malformed("scrambled word geometry".into()));
    }
    let mut seen = vec![false; order.len()];
    for &index in order {
        let entry = seen
            .get_mut(index)
            .ok_or_else(|| Error::Malformed("scrambled word index".into()))?;
        if *entry {
            return Err(Error::Malformed("scrambled word duplicate".into()));
        }
        *entry = true;
    }
    let mut out = vec![0; bytes.len()];
    for (i, (&index, &mask)) in order.iter().zip(masks).enumerate() {
        U16Field::new(i * 2, Endian::Little).write(
            &mut out,
            U16Field::new(index * 2, Endian::Little).read(bytes)? ^ mask,
        )?;
    }
    Ok(out)
}

/// Rotate a final word sum right by its own low byte modulo sixteen.
pub fn rotate_sum_right(bytes: &[u8]) -> Result<u16> {
    let sum = Rule {
        unit: Unit::Word(Endian::Little),
        fold: Fold::Sum,
        bits: 16,
        seed: 0,
        xor_out: 0,
        offset_base: 0,
    }
    .compute(bytes)? as u16;
    Ok(sum.rotate_right(u32::from(sum & 255) % 16))
}
/// Sum low bytes and XOR high bytes of successive little-endian words.
pub fn lane_sum_xor(bytes: &[u8]) -> Result<u16> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::Malformed("odd lane checksum extent".into()));
    }
    let (mut sum, mut xor) = (0u8, 0u8);
    for offset in (0..bytes.len()).step_by(2) {
        sum = sum.wrapping_add(U8Field::new(offset).read(bytes)?);
        xor ^= U8Field::new(offset + 1).read(bytes)?;
    }
    Ok(u16::from(sum) | (u16::from(xor) << 8))
}
/// Two 8-bit accumulators: B doubles, A adds the byte, B adds A.
pub fn rolling_pair(bytes: &[u8]) -> u16 {
    let (mut a, mut b) = (0u8, 0u8);
    for &byte in bytes {
        b = b.wrapping_mul(2);
        a = a.wrapping_add(byte);
        b = b.wrapping_add(a);
    }
    u16::from(a) | (u16::from(b) << 8)
}
/// Two's-complement finalization, modulo an explicitly supplied width.
pub fn negate(value: u64, bits: u32) -> Result<u64> {
    if !(1..=64).contains(&bits) {
        return Err(Error::Malformed("negated checksum width".into()));
    }
    Ok(value.wrapping_neg() & (u64::MAX >> (64 - bits)))
}
