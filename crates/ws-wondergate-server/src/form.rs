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
        if !crate::definition::field_key(key) {
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
    if fields.keys().any(|key| !specs.contains_key(key)) {
        return Err(malformed("unexpected request fields"));
    }
    let mut values = BTreeMap::new();
    for (key, spec) in specs {
        let Some(entries) = fields.get(key) else {
            if let Some(default) = &spec.default {
                values.insert(key.clone(), validate_value(default, spec)?);
            } else if !spec.optional {
                return Err(malformed("missing request field"));
            }
            continue;
        };
        if entries.is_empty() || (matches!(spec.repeats, Repeats::One) && entries.len() != 1) {
            return Err(malformed("missing or repeated request field"));
        }
        let mut parsed = entries
            .iter()
            .map(|raw| validate_value(raw, spec))
            .collect::<Result<Vec<_>>>()?;
        let chosen = match spec.repeats {
            Repeats::One | Repeats::First => parsed.remove(0),
            Repeats::Last => parsed.pop().ok_or_else(|| malformed("missing field"))?,
        };
        values.insert(key.clone(), chosen);
    }
    Ok(values)
}
pub(crate) fn validate_value(raw: &[u8], spec: &FieldSpec) -> Result<Field> {
    let mut bytes = match spec.decode {
        Decode::Raw => raw.to_vec(),
        Decode::PercentPlus => percent_plus(raw)?,
    };
    let [min, max] = spec.bytes.unwrap_or([0, 512]);
    name(&bytes, min, max, &spec.deny, spec.min_byte.unwrap_or(0))?;
    if spec
        .equals
        .as_ref()
        .is_some_and(|expected| expected != &bytes)
        || spec
            .choices
            .as_ref()
            .is_some_and(|choices| !choices.contains(&bytes))
        || spec
            .last_bytes
            .as_ref()
            .is_some_and(|allowed| bytes.last().is_none_or(|b| !allowed.contains(b)))
        || spec
            .alphabet
            .as_ref()
            .is_some_and(|a| bytes.iter().any(|b| !a.as_bytes().contains(b)))
        || spec
            .prefix
            .as_ref()
            .is_some_and(|p| !bytes.starts_with(p.as_bytes()))
        || spec
            .suffix
            .as_ref()
            .is_some_and(|p| !bytes.ends_with(p.as_bytes()))
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
    let numeric = if spec.regex.is_some() || spec.number.is_some() {
        let (lo, hi) = spec
            .regex
            .as_deref()
            .map(decimal_width)
            .transpose()?
            .unwrap_or((1, 20));
        let maximum = if matches!(spec.number, Some(Number::U32)) {
            u64::from(u32::MAX)
        } else {
            u64::MAX
        };
        Some(number(
            &bytes,
            lo,
            hi,
            maximum.min(spec.max_number.unwrap_or(u64::MAX)),
        )?)
    } else {
        None
    };
    if let Some(byte) = spec.trim_end {
        while bytes.last() == Some(&byte) {
            bytes.pop();
        }
    }
    Ok(Field {
        bytes,
        number: numeric,
    })
}
pub(crate) type Tables = BTreeMap<String, BTreeMap<String, Vec<Field>>>;
/// Repeated delimited rows are validated completely before any store action.
pub(crate) fn tables(
    fields: &mut Fields,
    specs: &BTreeMap<String, crate::definition::TableSpec>,
) -> Result<Tables> {
    let mut result = Tables::new();
    for (key, spec) in specs {
        let entries = fields.remove(key).unwrap_or_default();
        if entries.is_empty() && !spec.optional {
            return Err(malformed("missing table field"));
        }
        let mut rows = BTreeMap::new();
        for raw in entries {
            let bytes = percent_plus(&raw)?;
            let mut parts = bytes.split(|b| *b == spec.delimiter);
            let raw_tag = parts.next().ok_or_else(|| malformed("missing row tag"))?;
            let tag = if let Some([lo, hi]) = spec.tag_decimal_width {
                number(raw_tag, lo, hi, u64::MAX)?.to_string()
            } else {
                std::str::from_utf8(raw_tag)
                    .map_err(|_| malformed("non-ASCII row tag"))?
                    .to_owned()
            };
            let columns = spec
                .rows
                .get(&tag)
                .ok_or_else(|| malformed("unknown row tag"))?;
            let parts: Vec<_> = parts.collect();
            if parts.len() != columns.len() || rows.contains_key(&tag) {
                return Err(malformed("duplicate row or wrong column count"));
            }
            let row = parts
                .into_iter()
                .zip(columns)
                .map(|(raw, spec)| validate_value(raw, spec))
                .collect::<Result<Vec<_>>>()?;
            rows.insert(tag, row);
        }
        result.insert(key.clone(), rows);
    }
    Ok(result)
}

/// The Python oracle uses standard Shift-JIS, without Windows extensions.
/// Invalid sequences consume one byte, preserving a following valid byte.
pub fn shift_jis(bytes: &[u8], replacement: bool) -> Result<String> {
    if bytes.len() > 512 {
        return Err(malformed("oversized Shift-JIS field"));
    }
    if replacement {
        Ok(formatkit_core::text::decode_shift_jis_jis0208_lossy(bytes))
    } else {
        formatkit_core::text::decode_shift_jis_jis0208(bytes)
            .map_err(|_| malformed("invalid Shift-JIS field"))
    }
}
