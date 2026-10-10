// SPDX-License-Identifier: MIT
// Ported field validation from the private revival server's query/form helpers.
// Copyright (C) 2026 jidoughost. Bounded, data-declared policies are new.
use crate::{
    definition::{decimal_width, Charset, Decode, FieldSpec, Number, Repeats},
    malformed,
};
use formatkit_core::{Reader, Result};
use std::collections::BTreeMap;
pub type Fields = BTreeMap<String, Vec<Vec<u8>>>;
/// Split bytes without Unicode normalization or decoding keys. Values retain
/// percent escapes until their declared decoding policy is applied.
pub fn fields(bytes: &[u8]) -> Result<Fields> {
    if bytes.len() > 65536 {
        return Err(malformed("oversized form"));
    }
    let mut result: Fields = BTreeMap::new();
    if bytes.is_empty() {
        return Ok(result);
    }
    for (i, pair) in bytes.split(|b| *b == b'&').enumerate() {
        if i >= 128 {
            return Err(malformed("too many form fields"));
        }
        let split = pair
            .iter()
            .position(|b| *b == b'=')
            .ok_or_else(|| malformed("missing field equals"))?;
        let key =
            std::str::from_utf8(&pair[..split]).map_err(|_| malformed("non-ASCII field key"))?;
        if !crate::definition::identifier(key) {
            return Err(malformed("invalid field key"));
        }
        result
            .entry(key.into())
            .or_default()
            .push(pair[split + 1..].to_vec());
    }
    Ok(result)
}
pub fn one<'a>(fields: &'a Fields, key: &str) -> Result<&'a [u8]> {
    let values = fields
        .get(key)
        .ok_or_else(|| malformed("missing form field"))?;
    if values.len() != 1 {
        return Err(malformed("repeated form field"));
    }
    Ok(&values[0])
}
pub fn percent_plus(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > 65536 {
        return Err(malformed("oversized percent field"));
    }
    let mut reader = Reader::new(bytes);
    let mut decoded = Vec::with_capacity(bytes.len());
    while reader.remaining() != 0 {
        decoded.push(match reader.u8()? {
            b'+' => b' ',
            b'%' => {
                let hex = reader.bytes(2)?;
                let digit = |b: u8| {
                    char::from(b)
                        .to_digit(16)
                        .map(|v| v as u8)
                        .ok_or_else(|| malformed("invalid percent escape"))
                };
                digit(hex[0])? * 16 + digit(hex[1])?
            }
            byte => byte,
        });
    }
    Ok(decoded)
}
/// Raw names can contain multibyte text; length and delimiters apply to bytes.
pub fn name(bytes: &[u8], min: usize, max: usize, deny: &[u8], min_byte: u8) -> Result<()> {
    if max > 512
        || min > max
        || bytes.len() < min
        || bytes.len() > max
        || bytes.iter().any(|b| *b < min_byte || deny.contains(b))
    {
        return Err(malformed("invalid name field"));
    }
    Ok(())
}
pub fn number(bytes: &[u8], min_digits: usize, max_digits: usize, max: u64) -> Result<u64> {
    if min_digits == 0
        || max_digits > 20
        || max_digits < min_digits
        || bytes.len() < min_digits
        || bytes.len() > max_digits
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return Err(malformed("invalid decimal field"));
    }
    let mut value = 0u64;
    for byte in bytes {
        value = value
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(*byte - b'0')))
            .ok_or_else(|| malformed("decimal field overflow"))?;
    }
    if value > max {
        return Err(malformed("decimal field outside range"));
    }
    Ok(value)
}
pub(crate) fn latin1(text: &str) -> Result<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).map_err(|_| malformed("non-Latin-1 request field")))
        .collect()
}
#[derive(Clone)]
pub(crate) struct Field {
    pub bytes: Vec<u8>,
    pub number: Option<u64>,
}
pub(crate) fn validate(
    fields: &Fields,
    specs: &BTreeMap<String, FieldSpec>,
) -> Result<BTreeMap<String, Field>> {
    if fields.keys().ne(specs.keys()) {
        return Err(malformed("unexpected or missing request fields"));
    }
    let mut values = BTreeMap::new();
    for (key, spec) in specs {
        let raw = match spec.repeats {
            Repeats::One => one(fields, key)?,
            Repeats::First => fields[key]
                .first()
                .ok_or_else(|| malformed("missing field"))?,
            Repeats::Last => fields[key]
                .last()
                .ok_or_else(|| malformed("missing field"))?,
        };
        // All occurrences must be well-formed, even when first/last wins.
        let mut chosen = None;
        for entry in &fields[key] {
            let bytes = match spec.decode {
                Decode::Raw => entry.clone(),
                Decode::PercentPlus => percent_plus(entry)?,
            };
            let [min, max] = spec.bytes.unwrap_or([0, 512]);
            name(&bytes, min, max, &spec.deny, spec.min_byte.unwrap_or(0))?;
            if spec
                .equals
                .as_ref()
                .is_some_and(|expected| expected != &bytes)
            {
                return Err(malformed("unexpected field value"));
            }
            match spec.charset {
                Charset::Ascii if !bytes.is_ascii() => return Err(malformed("non-ASCII field")),
                Charset::ShiftJis if shift_jis(&bytes, false).is_err() => {
                    return Err(malformed("invalid Shift-JIS field"))
                }
                _ => {}
            }
            let mut numeric = None;
            if spec.regex.is_some() || spec.number.is_some() {
                let (lo, hi) = spec
                    .regex
                    .as_deref()
                    .map(decimal_width)
                    .transpose()?
                    .unwrap_or((1, 20));
                let max = if matches!(spec.number, Some(Number::U32)) {
                    u64::from(u32::MAX)
                } else {
                    u64::MAX
                };
                numeric = Some(number(&bytes, lo, hi, max)?);
            }
            if entry.as_slice() == raw {
                chosen = Some(Field {
                    bytes,
                    number: numeric,
                });
            }
        }
        values.insert(
            key.clone(),
            chosen.ok_or_else(|| malformed("missing chosen field"))?,
        );
    }
    Ok(values)
}

/// The Python oracle uses standard Shift-JIS, without Windows extensions.
/// Invalid sequences consume one byte, preserving a following valid byte.
pub fn shift_jis(bytes: &[u8], replacement: bool) -> Result<String> {
    if bytes.len() > 512 {
        return Err(malformed("oversized Shift-JIS field"));
    }
    let mut reader = Reader::new(bytes);
    let mut text = String::new();
    while reader.remaining() != 0 {
        let lead = reader.u8()?;
        if lead < 0x80 {
            text.push(char::from(lead));
            continue;
        }
        if (0xa1..=0xdf).contains(&lead) {
            // This interval fits Unicode's half-width katakana block.
            text.push(
                char::from_u32(0xff61 + u32::from(lead - 0xa1))
                    .ok_or_else(|| malformed("invalid half-width character"))?,
            );
            continue;
        }
        if ((0x81..=0x9f).contains(&lead) || (0xe0..=0xea).contains(&lead))
            && lead != 0x87
            && reader.remaining() != 0
        {
            let trail = bytes[reader.pos()];
            let pair = [lead, trail];
            if let Some(decoded) =
                encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(&pair)
            {
                reader.skip(1)?;
                // JIS X 0208 mappings differ from Windows-31J at six points.
                let standard = match u16::from_be_bytes(pair) {
                    0x8160 => Some('\u{301c}'),
                    0x8161 => Some('\u{2016}'),
                    0x817c => Some('\u{2212}'),
                    0x8191 => Some('\u{00a2}'),
                    0x8192 => Some('\u{00a3}'),
                    0x81ca => Some('\u{00ac}'),
                    _ => None,
                };
                if let Some(character) = standard {
                    text.push(character);
                } else {
                    text.push_str(&decoded);
                }
                continue;
            }
        }
        if !replacement {
            return Err(malformed("invalid Shift-JIS field"));
        }
        text.push('\u{fffd}');
    }
    Ok(text)
}
