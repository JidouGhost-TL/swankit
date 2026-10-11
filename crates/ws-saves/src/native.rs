//! Primitive operations used by generated game grammars; byte fields stay in Formatkit.
use crate::{engine::Engine, value::*};
use formatkit_core::{bytes_at, Endian, Result};
use formatkit_layout::{
    patch_bytes, I16Field, I32Field, I8Field, U16Field, U32Field, U64Field, U8Field,
};
use sha2::Digest;
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

fn arg(a: &[Value], i: usize) -> Result<&Value> {
    a.get(i).ok_or_else(|| bad("missing primitive argument"))
}
fn length(v: &Value) -> Result<usize> {
    Ok(match v {
        Value::Bytes(b) => b.borrow().len(),
        Value::List(l) | Value::Set(l) => l.borrow().len(),
        Value::Map(m) => m.borrow().len(),
        Value::Text(t) => t.chars().count(),
        _ => return Err(bad("length target")),
    })
}
fn checked_count(n: usize) -> Result<usize> {
    if n > MAX_ITEMS {
        Err(bad("allocation limit"))
    } else {
        Ok(n)
    }
}
pub(crate) fn format_value(v: &Value, spec: &str) -> Result<String> {
    if spec.is_empty() {
        return Ok(v.text());
    }
    let kind = spec.chars().last().ok_or_else(|| bad("format spec"))?;
    let digits = spec.trim_end_matches(|c: char| c.is_alphabetic());
    if kind == 'f' {
        let precision = digits
            .split('.')
            .nth(1)
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(6);
        if precision > 32 {
            return Err(bad("format precision"));
        }
        let width = digits
            .split('.')
            .next()
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(0);
        checked_count(width)?;
        return Ok(if digits.starts_with('0') {
            format!("{:0width$.precision$}", v.number()?)
        } else {
            format!("{:width$.precision$}", v.number()?)
        });
    }
    let prefix = digits.contains('#');
    let zero = digits.contains('0');
    let width = digits
        .trim_start_matches(['#', '0', '>', '<'])
        .parse::<usize>()
        .unwrap_or(0);
    checked_count(width)?;
    let raw = match kind {
        'x' => format!("{:x}", v.integer()?),
        'X' => format!("{:X}", v.integer()?),
        'd' => v.integer()?.to_string(),
        'b' => format!("{:b}", v.integer()?),
        's' => v.text(),
        _ => return Err(bad(format!("format kind {kind}"))),
    };
    let pre = if prefix {
        match kind {
            'x' => "0x",
            'X' => "0X",
            'b' => "0b",
            _ => "",
        }
    } else {
        ""
    };
    let padding = width.saturating_sub(pre.len() + raw.len());
    Ok(if zero {
        format!("{pre}{}{raw}", "0".repeat(padding))
    } else {
        format!("{}{pre}{raw}", " ".repeat(padding))
    })
}
pub(crate) fn percent_format(template: &str, v: &Value) -> Result<String> {
    let args = if matches!(v, Value::List(_)) {
        v.sequence()?
    } else {
        vec![v.clone()]
    };
    let mut chars = template.chars().peekable();
    let mut out = String::new();
    let mut i = 0;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        let mut spec = String::new();
        loop {
            let c = chars.next().ok_or_else(|| bad("unfinished format"))?;
            spec.push(c);
            if c.is_alphabetic() {
                break;
            }
        }
        out.push_str(&format_value(arg(&args, i)?, &spec)?);
        i += 1;
    }
    Ok(out)
}

impl Engine {
    pub fn checksum_native(&self, name: &str, a: &[Value]) -> Option<Result<Value>> {
        let rule = self.checksums.get(name)?;
        if rule["algorithm"] != "crc16" {
            return None;
        }
        Some((|| {
            let p = &rule["parameters"];
            let word = |key: &str| {
                p[key]
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| bad("CRC parameter"))
            };
            if rule["algorithm"] != "crc16" {
                return Err(bad("checksum algorithm"));
            }
            let mut crc = ws_save::checksum::Crc16 {
                polynomial: word("polynomial")?,
                seed: word("seed")?,
                xor_out: word("xor_out")?,
                swap_bytes: p["swap_bytes"]
                    .as_bool()
                    .ok_or_else(|| bad("CRC byte order"))?,
            };
            if let Some(seed) = p["seed_argument"]
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| a.get(i))
            {
                crc.seed = u16::try_from(seed.integer()?).map_err(|_| bad("CRC seed"))?;
            }
            Ok(Value::Int(crc.compute(&arg(a, 0)?.raw()?) as i128))
        })())
    }
    pub fn native(
        &mut self,
        name: &str,
        a: Vec<Value>,
        kw: HashMap<String, Value>,
    ) -> Result<Value> {
        match name {
            "definition_checksum" => {
                let rule = self
                    .checksums
                    .get(&arg(&a, 0)?.text())
                    .ok_or_else(|| bad("unknown checksum binding"))?;
                let p = &rule["parameters"];
                let number = |key: &str| p[key].as_u64().ok_or_else(|| bad("checksum parameter"));
                let unit = match p["unit"].as_str() {
                    Some("byte") => ws_save::checksum::Unit::Byte,
                    Some("word-le") => ws_save::checksum::Unit::Word(Endian::Little),
                    _ => return Err(bad("checksum unit")),
                };
                let operand = arg(&a, 1)?;
                let values = if unit == ws_save::checksum::Unit::Word(Endian::Little)
                    && matches!(operand, Value::Bytes(_))
                {
                    let bytes = operand.raw()?;
                    if !bytes.len().is_multiple_of(2) {
                        return Err(bad("odd checksum word extent"));
                    }
                    (0..bytes.len())
                        .step_by(2)
                        .map(|offset| {
                            U16Field::new(offset, Endian::Little)
                                .read(&bytes)
                                .map(|v| Value::Int(v.into()))
                        })
                        .collect::<Result<Vec<_>>>()?
                } else {
                    operand.sequence()?
                };
                let width = if unit == ws_save::checksum::Unit::Byte {
                    1
                } else {
                    2
                };
                let mut data = vec![
                    0;
                    checked_count(
                        values
                            .len()
                            .checked_mul(width)
                            .ok_or_else(|| bad("checksum extent"))?
                    )?
                ];
                for (i, value) in values.iter().enumerate() {
                    if width == 1 {
                        U8Field::new(i).write(
                            &mut data,
                            u8::try_from(value.integer()?).map_err(|_| bad("checksum byte"))?,
                        )?;
                    } else {
                        U16Field::new(i * 2, Endian::Little).write(
                            &mut data,
                            u16::try_from(value.integer()?).map_err(|_| bad("checksum word"))?,
                        )?;
                    }
                }
                let fold = match p["fold"].as_str() {
                    Some("sum") => ws_save::checksum::Fold::Sum,
                    Some("xor") => ws_save::checksum::Fold::Xor,
                    _ => return Err(bad("checksum fold")),
                };
                Ok(Value::Int(
                    ws_save::checksum::Rule {
                        unit,
                        fold,
                        bits: u32::try_from(number("bits")?).map_err(|_| bad("checksum width"))?,
                        seed: number("seed")?,
                        xor_out: number("xor_out")?,
                        offset_base: number("offset_base")?,
                    }
                    .compute(&data)? as i128,
                ))
            }
            "definition_primitive" => {
                let binding = self
                    .primitives
                    .get(&arg(&a, 0)?.text())
                    .ok_or_else(|| bad("unknown definition primitive"))?;
                let value = u64::try_from(arg(&a, 1)?.integer()?)
                    .map_err(|_| bad("scrambled word value"))?;
                let n = arg(&a, 2)?.index()?;
                if binding["algorithm"] != "xor-rotate-feedback" {
                    return Err(bad("unknown transform"));
                }
                Ok(Value::Int(
                    crate::transform::word(value, n, &binding["parameters"])? as i128,
                ))
            }
            "input_table" => {
                let key = arg(&a, 0)?.text();
                let bytes = self
                    .context
                    .get(&key)
                    .ok_or_else(|| bad(format!("external semantic table required: {key}")))?;
                Ok(Value::bytes(bytes.clone()))
            }
            "isinstance" => {
                let v = arg(&a, 0)?;
                let types = if matches!(a.get(1), Some(Value::List(_))) {
                    a[1].sequence()?
                } else {
                    vec![arg(&a, 1)?.clone()]
                };
                let yes = types.iter().any(|t| match t.text().as_str() {
                    "int" => matches!(v, Value::Int(_) | Value::Big(_) | Value::Bool(_)),
                    "str" => matches!(v, Value::Text(_)),
                    "dict" => matches!(v, Value::Map(_)),
                    "bytes" | "bytearray" => matches!(v, Value::Bytes(_)),
                    "list" | "tuple" => matches!(v, Value::List(_)),
                    "set" => matches!(v, Value::Set(_)),
                    _ => false,
                });
                Ok(Value::Bool(yes))
            }
            "len" => Ok(Value::Int(length(arg(&a, 0)?)? as i128)),
            "bool" => Ok(Value::Bool(arg(&a, 0)?.truth())),
            "str" => Ok(Value::Text(arg(&a, 0)?.text())),
            "int" => {
                let v = arg(&a, 0)?;
                Ok(Value::Int(match v {
                    Value::Text(t) => {
                        let radix = a.get(1).map(Value::integer).transpose()?.unwrap_or(10);
                        i128::from_str_radix(
                            t,
                            u32::try_from(radix)
                                .ok()
                                .filter(|r| (2..=36).contains(r))
                                .ok_or_else(|| bad("integer radix"))?,
                        )
                        .map_err(|_| bad("integer text"))?
                    }
                    Value::Float(n) => *n as i128,
                    _ => v.integer()?,
                }))
            }
            "list" | "tuple" | "set" | "iter" => {
                let mut seq = if a.is_empty() {
                    vec![]
                } else {
                    arg(&a, 0)?.sequence()?
                };
                if name == "set" {
                    let mut unique = vec![];
                    for v in seq {
                        if Value::find_equal(&unique, &v)?.is_none() {
                            unique.push(v);
                        }
                    }
                    seq = unique;
                    return Ok(Value::set(seq));
                }
                Ok(Value::list(seq))
            }
            "dict" => {
                let mut pairs = vec![];
                if let Some(v) = a.first() {
                    match v {
                        Value::Map(m) => pairs = m.borrow().clone(),
                        _ => {
                            for v in v.sequence()? {
                                let p = v.sequence()?;
                                if p.len() != 2 {
                                    return Err(bad("dictionary pair"));
                                }
                                pairs.push((p[0].clone(), p[1].clone()));
                            }
                        }
                    }
                }
                for (k, v) in kw {
                    pairs.push((Value::Text(k), v));
                }
                Ok(Value::map(pairs))
            }
            "bytes" | "bytearray" => {
                let bytes = if let Some(v) = a.first() {
                    if matches!(v, Value::Int(_)) {
                        vec![0; checked_count(v.index()?)?]
                    } else {
                        v.raw()?
                    }
                } else {
                    vec![]
                };
                Ok(Value::bytes(bytes))
            }
            "range" => {
                let (start, end, step) = match a.len() {
                    1 => (0, a[0].integer()?, 1),
                    2 => (a[0].integer()?, a[1].integer()?, 1),
                    3 => (a[0].integer()?, a[1].integer()?, a[2].integer()?),
                    _ => return Err(bad("range arguments")),
                };
                if step == 0 {
                    return Err(bad("range zero step"));
                }
                let mut out = vec![];
                let mut x = start;
                while if step > 0 { x < end } else { x > end } {
                    if out.len() >= MAX_ITEMS {
                        return Err(bad("range limit"));
                    }
                    out.push(Value::Int(x));
                    x = x.checked_add(step).ok_or_else(|| bad("range overflow"))?;
                }
                Ok(Value::list(out))
            }
            "enumerate" => {
                let start = a.get(1).map(Value::integer).transpose()?.unwrap_or(0);
                Ok(Value::list(
                    arg(&a, 0)?
                        .sequence()?
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let index = start
                                .checked_add(i as i128)
                                .ok_or_else(|| bad("enumerate overflow"))?;
                            Ok(Value::list(vec![Value::Int(index), v]))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ))
            }
            "zip" => {
                let seqs = a.iter().map(Value::sequence).collect::<Result<Vec<_>>>()?;
                let n = seqs.iter().map(Vec::len).min().unwrap_or(0);
                Ok(Value::list(
                    (0..n)
                        .map(|i| Value::list(seqs.iter().map(|v| v[i].clone()).collect()))
                        .collect(),
                ))
            }
            "sum" => {
                if let Value::Bytes(bytes) = arg(&a, 0)? {
                    let rule = ws_save::checksum::Rule {
                        unit: ws_save::checksum::Unit::Byte,
                        fold: ws_save::checksum::Fold::Sum,
                        bits: 64,
                        seed: 0,
                        xor_out: 0,
                        offset_base: 0,
                    };
                    let mut sum = rule.compute(&bytes.borrow())? as i128;
                    if let Some(v) = a.get(1) {
                        sum = sum
                            .checked_add(v.integer()?)
                            .ok_or_else(|| bad("sum overflow"))?;
                    }
                    return Ok(Value::Int(sum));
                }
                let mut sum = a.get(1).cloned().unwrap_or(Value::Int(0));
                for v in arg(&a, 0)?.sequence()? {
                    sum = self.binary("Add", sum, v)?;
                }
                Ok(sum)
            }
            "all" | "any" => {
                let seq = arg(&a, 0)?.sequence()?;
                Ok(Value::Bool(if name == "all" {
                    seq.iter().all(Value::truth)
                } else {
                    seq.iter().any(Value::truth)
                }))
            }
            "min" | "max" => {
                let seq = if a.len() == 1 { a[0].sequence()? } else { a };
                let mut best = seq.first().cloned().ok_or_else(|| bad("empty min/max"))?;
                for v in &seq[1..] {
                    if Self::compare(if name == "min" { "Lt" } else { "Gt" }, v, &best)? {
                        best = v.clone();
                    }
                }
                Ok(best)
            }
            "reversed" => {
                let mut seq = arg(&a, 0)?.sequence()?;
                seq.reverse();
                Ok(Value::list(seq))
            }
            "sorted" => {
                let mut values = vec![];
                for v in arg(&a, 0)?.sequence()? {
                    let key = if let Some(fun) = kw.get("key") {
                        self.call(fun.clone(), vec![v.clone()], HashMap::new())?
                    } else {
                        v.clone()
                    };
                    values.push((key, v));
                }
                // Merge with fallible comparisons: mapping errors to Equal can
                // violate the total-order contract required by slice sorting.
                let reverse = kw.get("reverse").is_some_and(Value::truth);
                let mut scratch = values.clone();
                let mut width = 1;
                while width < values.len() {
                    for start in (0..values.len()).step_by(width * 2) {
                        let middle = (start + width).min(values.len());
                        let end = (middle + width).min(values.len());
                        let (mut left, mut right) = (start, middle);
                        for destination in &mut scratch[start..end] {
                            let take_left = if left == middle {
                                false
                            } else if right == end {
                                true
                            } else {
                                !Self::compare(
                                    if reverse { "Lt" } else { "Gt" },
                                    &values[left].0,
                                    &values[right].0,
                                )?
                            };
                            let index = if take_left {
                                let index = left;
                                left += 1;
                                index
                            } else {
                                let index = right;
                                right += 1;
                                index
                            };
                            *destination = values[index].clone();
                        }
                    }
                    std::mem::swap(&mut values, &mut scratch);
                    width *= 2;
                }
                Ok(Value::list(values.into_iter().map(|(_, v)| v).collect()))
            }
            "next" => Ok(arg(&a, 0)?
                .sequence()?
                .first()
                .cloned()
                .or_else(|| a.get(1).cloned())
                .unwrap_or(Value::None)),
            "chr" => Ok(Value::Text(
                char::from_u32(
                    u32::try_from(arg(&a, 0)?.integer()?).map_err(|_| bad("character range"))?,
                )
                .ok_or_else(|| bad("invalid character"))?
                .to_string(),
            )),
            "ord" => {
                let s = arg(&a, 0)?.text();
                let mut c = s.chars();
                let x = c.next().ok_or_else(|| bad("empty character"))?;
                if c.next().is_some() {
                    return Err(bad("multiple characters"));
                }
                Ok(Value::Int(x as i128))
            }
            "hex" => Ok(Value::Text(format!("0x{:x}", arg(&a, 0)?.integer()?))),
            "bin" => Ok(Value::Text(format!("0b{:b}", arg(&a, 0)?.integer()?))),
            "format" => Ok(Value::Text(format_value(arg(&a, 0)?, &arg(&a, 1)?.text())?)),
            "round" => {
                let n = arg(&a, 0)?.number()?;
                if let Some(d) = a.get(1) {
                    let digits = i32::try_from(d.integer()?)
                        .ok()
                        .filter(|d| (-16..=16).contains(d))
                        .ok_or_else(|| bad("round precision"))?;
                    let factor = 10f64.powi(digits);
                    Ok(Value::Float((n * factor).round_ties_even() / factor))
                } else {
                    Ok(Value::Int(n.round_ties_even() as i128))
                }
            }
            "abs" => {
                let v = arg(&a, 0)?;
                Ok(match v {
                    Value::Float(f) => Value::Float(f.abs()),
                    _ => Value::Int(
                        v.integer()?
                            .checked_abs()
                            .ok_or_else(|| bad("abs overflow"))?,
                    ),
                })
            }
            "divmod" => {
                if matches!(a.first(), Some(Value::Float(_)))
                    || matches!(a.get(1), Some(Value::Float(_)))
                {
                    let x = arg(&a, 0)?.number()?;
                    let y = arg(&a, 1)?.number()?;
                    if y == 0.0 {
                        return Err(bad("division by zero"));
                    }
                    return Ok(Value::list(vec![
                        Value::Float((x / y).floor()),
                        Value::Float(x % y),
                    ]));
                }
                let x = arg(&a, 0)?.integer()?;
                let y = arg(&a, 1)?.integer()?;
                Ok(Value::list(vec![
                    Value::Int(x.checked_div_euclid(y).ok_or_else(|| bad("division"))?),
                    Value::Int(x.checked_rem_euclid(y).ok_or_else(|| bad("modulo"))?),
                ]))
            }
            "ValueError" | "SystemExit" | "OSError" => Ok(Value::Text(
                a.first()
                    .map(Value::text)
                    .unwrap_or_else(|| "save rejected".into()),
            )),
            "open" => {
                let path = arg(&a, 0)?.text();
                let mode = a.get(1).map(Value::text).unwrap_or_else(|| "rb".into());
                if mode.contains('w') {
                    self.files.insert(path.clone(), vec![]);
                } else if !self.files.contains_key(&path) {
                    return Err(bad("save grammar requested unavailable external input"));
                }
                Ok(Value::File(path))
            }
            "print" => {
                for v in a {
                    if let Value::Json(value) = v {
                        self.output.push(value);
                    }
                }
                Ok(Value::None)
            }
            "json.dumps" => Ok(Value::Json(arg(&a, 0)?.to_json()?)),
            "json.dump" => {
                let value = arg(&a, 0)?.to_json()?;

                self.output.push(value);
                Ok(Value::None)
            }
            "sys.exit" => {
                let code = a.first().cloned().unwrap_or(Value::Int(0));
                if code.truth() {
                    return Err(bad(format!("save parser exit {}", code.text())));
                }
                Ok(Value::None)
            }
            "sys.stderr.write" | "sys.stdout.write" => {
                if let Some(Value::Json(v)) = a.first() {
                    self.output.push(v.clone());
                }
                Ok(Value::None)
            }
            "os.path.exists" => Ok(Value::Bool(self.files.contains_key(&arg(&a, 0)?.text()))),
            "os.path.splitext" => {
                let t = arg(&a, 0)?.text();
                let (base, ext) = t
                    .rsplit_once('.')
                    .map_or((t.as_str(), String::new()), |(b, e)| (b, format!(".{e}")));
                Ok(Value::list(vec![
                    Value::Text(base.into()),
                    Value::Text(ext),
                ]))
            }
            "os.environ.get" => Ok(a.get(1).cloned().unwrap_or(Value::None)),
            "unicodedata.normalize" => {
                let form = arg(&a, 0)?.text();
                let text = arg(&a, 1)?.text();
                Ok(Value::Text(match form.as_str() {
                    "NFKC" => text.nfkc().collect(),
                    "NFC" => text.nfc().collect(),
                    _ => return Err(bad("text normalization")),
                }))
            }
            "bytes.fromhex" => {
                let s = arg(&a, 0)?.text();
                let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                if !s.len().is_multiple_of(2) {
                    return Err(bad("hex length"));
                }
                let mut out = vec![];
                for pair in s.as_bytes().chunks_exact(2) {
                    let text = std::str::from_utf8(pair).map_err(|_| bad("hex text"))?;
                    out.push(u8::from_str_radix(text, 16).map_err(|_| bad("hex digit"))?);
                }
                Ok(Value::bytes(out))
            }
            "int.from_bytes" => {
                let b = arg(&a, 0)?.raw()?;
                checked_count(b.len())?;
                let little = a.get(1).map(Value::text).unwrap_or_else(|| "big".into()) == "little";
                if b.len() > 16 {
                    let signed = kw.get("signed").is_some_and(Value::truth);
                    return Ok(Value::Big(if signed {
                        if little {
                            num_bigint::BigInt::from_signed_bytes_le(&b)
                        } else {
                            num_bigint::BigInt::from_signed_bytes_be(&b)
                        }
                    } else if little {
                        num_bigint::BigInt::from_bytes_le(num_bigint::Sign::Plus, &b)
                    } else {
                        num_bigint::BigInt::from_bytes_be(num_bigint::Sign::Plus, &b)
                    }));
                }
                let mut v = 0u128;
                for &x in if little {
                    b.iter().rev().collect::<Vec<_>>()
                } else {
                    b.iter().collect::<Vec<_>>()
                } {
                    v = (v << 8) | u128::from(x);
                }
                let signed = kw.get("signed").is_some_and(Value::truth);
                if signed
                    && !b.is_empty()
                    && b[if little { b.len() - 1 } else { 0 }] & 128 != 0
                    && b.len() < 16
                {
                    v |= u128::MAX << (b.len() * 8);
                }
                if !signed && v > i128::MAX as u128 {
                    return Err(bad("unsigned integer exceeds signed 128-bit range"));
                }
                Ok(Value::Int(v as i128))
            }
            "str.maketrans" => {
                if a.len() == 1 {
                    if let Value::Map(m) = &a[0] {
                        let mut pairs = vec![];
                        for (k, v) in m.borrow().iter() {
                            let key = if let Value::Text(t) = k {
                                let mut chars = t.chars();
                                let c = chars.next().ok_or_else(|| bad("translation key"))?;
                                if chars.next().is_some() {
                                    return Err(bad("translation key length"));
                                }
                                Value::Int(c as i128)
                            } else {
                                k.clone()
                            };
                            pairs.push((key, v.clone()));
                        }
                        return Ok(Value::map(pairs));
                    }
                    return Err(bad("translation map"));
                }
                let x = arg(&a, 0)?.text();
                let y = arg(&a, 1)?.text();
                if x.chars().count() != y.chars().count() {
                    return Err(bad("translation table length"));
                }
                Ok(Value::map(
                    x.chars()
                        .zip(y.chars())
                        .map(|(x, y)| (Value::Int(x as i128), Value::Text(y.to_string())))
                        .collect(),
                ))
            }
            "struct.unpack_from" | "struct.unpack" | "struct.iter_unpack" => {
                let fmt = arg(&a, 0)?.text();
                let bytes = arg(&a, 1)?.raw()?;
                let offset = a.get(2).map(Value::index).transpose()?.unwrap_or(0);
                if name == "struct.iter_unpack" {
                    let width = struct_width(&fmt)?;
                    if width == 0 || !bytes.len().is_multiple_of(width) {
                        return Err(bad("record unpack extent"));
                    }
                    Ok(Value::list(
                        (0..bytes.len())
                            .step_by(width)
                            .map(|off| unpack(&fmt, &bytes, off).map(Value::list))
                            .collect::<Result<_>>()?,
                    ))
                } else {
                    if name == "struct.unpack" && struct_width(&fmt)? != bytes.len() {
                        return Err(bad("unpack exact size"));
                    }
                    Ok(Value::list(unpack(&fmt, &bytes, offset)?))
                }
            }
            "struct.pack" | "struct.pack_into" => {
                let fmt = arg(&a, 0)?.text();
                if name == "struct.pack" {
                    Ok(Value::bytes(pack(&fmt, &a[1..])?))
                } else {
                    arg(&a, 2)?;
                    let data = pack(&fmt, &a[3..])?;
                    let offset = arg(&a, 2)?.index()?;
                    if let Value::Bytes(b) = arg(&a, 1)? {
                        patch_bytes(&mut b.borrow_mut(), offset, &data)?;
                        Ok(Value::None)
                    } else {
                        Err(bad("pack destination"))
                    }
                }
            }
            "hashlib.sha256" | "hashlib.sha1" => {
                let data = arg(&a, 0)?.raw()?;
                let bytes = if name.ends_with("sha256") {
                    sha2::Sha256::digest(&data).to_vec()
                } else {
                    sha1::Sha1::digest(&data).to_vec()
                };
                Ok(Value::Bound("hash".into(), Box::new(Value::bytes(bytes))))
            }
            "__import__" => Ok(Value::Native(arg(&a, 0)?.text())),
            "zlib.crc32" => {
                let mut crc = !0u32;
                for b in arg(&a, 0)?.raw()? {
                    crc ^= u32::from(b);
                    for _ in 0..8 {
                        crc = if crc & 1 != 0 {
                            (crc >> 1) ^ 0xedb88320
                        } else {
                            crc >> 1
                        };
                    }
                }
                Ok(Value::Int((!crc) as i128))
            }
            other => Err(bad(format!("unsupported save primitive {other}"))),
        }
    }
    pub fn method(
        &mut self,
        name: &str,
        obj: Value,
        a: Vec<Value>,
        kw: HashMap<String, Value>,
    ) -> Result<Value> {
        if let Value::File(path) = &obj {
            return match name {
                "read" => {
                    let bytes = self
                        .files
                        .get(path)
                        .ok_or_else(|| bad("missing virtual image"))?;
                    let n = a
                        .first()
                        .map(Value::index)
                        .transpose()?
                        .unwrap_or(bytes.len())
                        .min(bytes.len());
                    Ok(Value::bytes(bytes_at(bytes, 0, n)?.to_vec()))
                }
                "write" => {
                    let data = arg(&a, 0)?.raw()?;
                    checked_count(data.len())?;
                    let len = data.len();
                    self.files.insert(path.clone(), data);
                    Ok(Value::Int(len as i128))
                }
                _ => Err(bad("unsupported image operation")),
            };
        }
        match name {
            "hexdigest" => {
                if let Value::Bound(_, bytes) = obj {
                    return Ok(Value::Text(
                        bytes.raw()?.iter().map(|b| format!("{b:02x}")).collect(),
                    ));
                }
                Err(bad("digest target"))
            }
            "hex" => {
                let sep = a.first().map(Value::text).unwrap_or_default();
                let bytes = obj.raw()?;
                Ok(Value::Text(
                    bytes
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(&sep),
                ))
            }
            "decode" => {
                let raw = obj.raw()?;
                let codec = a.first().map(Value::text).unwrap_or_else(|| "utf-8".into());
                let errors = a
                    .get(1)
                    .map(Value::text)
                    .or_else(|| kw.get("errors").map(Value::text))
                    .unwrap_or_else(|| "strict".into());
                let text = if codec == "shift_jis" || codec == "sjis" {
                    crate::jis0208::decode(&raw, &errors)?
                } else if codec == "cp932" {
                    crate::cp932::decode(&raw, &errors)?
                } else if codec == "ascii" {
                    if errors == "strict" && raw.iter().any(|b| *b > 127) {
                        return Err(bad("ASCII text"));
                    }
                    raw.iter()
                        .filter_map(|&b| {
                            if b < 128 {
                                Some(char::from(b))
                            } else if errors == "ignore" {
                                None
                            } else {
                                Some('\u{fffd}')
                            }
                        })
                        .collect()
                } else if codec == "latin1" || codec == "latin-1" {
                    raw.iter().map(|&b| char::from(b)).collect()
                } else {
                    let label = match codec.as_str() {
                        "euc_jp" => "euc-jp",
                        "utf-16le" | "utf-16-le" => "utf-16le",
                        x => x,
                    };
                    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
                        .ok_or_else(|| bad(format!("unsupported save encoding {codec}")))?;
                    let (text, error) = encoding.decode_without_bom_handling(&raw);
                    if error && errors == "strict" {
                        return Err(bad("invalid encoded text"));
                    }
                    if errors == "ignore" {
                        text.replace('\u{fffd}', "")
                    } else {
                        text.into_owned()
                    }
                };
                Ok(Value::Text(text))
            }
            "get" => {
                let key = arg(&a, 0)?;
                if let Value::Map(m) = obj {
                    let m = m.borrow();
                    Ok(Value::map_index(&m, key)?
                        .map(|index| m[index].1.clone())
                        .or_else(|| a.get(1).cloned())
                        .unwrap_or(Value::None))
                } else {
                    Err(bad("get target"))
                }
            }
            "keys" | "values" | "items" => {
                if let Value::Map(m) = obj {
                    Ok(Value::list(
                        m.borrow()
                            .iter()
                            .map(|(k, v)| match name {
                                "keys" => k.clone(),
                                "values" => v.clone(),
                                _ => Value::list(vec![k.clone(), v.clone()]),
                            })
                            .collect(),
                    ))
                } else {
                    Err(bad("dictionary view target"))
                }
            }
            "update" => {
                if let Value::Set(l) = &obj {
                    let seq = arg(&a, 0)?.sequence()?;
                    let mut l = l.borrow_mut();
                    for v in seq {
                        if Value::find_equal(&l, &v)?.is_none() {
                            l.push(v);
                            checked_count(l.len())?;
                        }
                    }
                    return Ok(Value::None);
                }
                if let Value::Map(m) = &obj {
                    let incoming = match a.first().unwrap_or(&Value::None) {
                        Value::None => vec![],
                        Value::Map(v) => v.borrow().clone(),
                        v => {
                            let mut out = vec![];
                            for pair in v.sequence()? {
                                let p = pair.sequence()?;
                                if p.len() != 2 {
                                    return Err(bad("update pair"));
                                }
                                out.push((p[0].clone(), p[1].clone()));
                            }
                            out
                        }
                    };
                    let _ = m;
                    for (k, v) in incoming {
                        Self::set(&obj, k, v)?;
                    }
                    for (k, v) in kw {
                        Self::set(&obj, Value::Text(k), v)?;
                    }
                    Ok(Value::None)
                } else {
                    Err(bad("update target"))
                }
            }
            "setdefault" => {
                let k = arg(&a, 0)?.clone();
                match Self::get(&obj, &k) {
                    Ok(v) => Ok(v),
                    Err(_) => {
                        let v = a.get(1).cloned().unwrap_or(Value::None);
                        Self::set(&obj, k, v.clone())?;
                        Ok(v)
                    }
                }
            }
            "append" | "add" => {
                if let Value::Bytes(b) = &obj {
                    let byte = u8::try_from(arg(&a, 0)?.integer()?)
                        .map_err(|_| bad("byte append range"))?;
                    let mut b = b.borrow_mut();
                    checked_count(b.len() + 1)?;
                    b.push(byte);
                    return Ok(Value::None);
                }
                if let Value::List(l) | Value::Set(l) = obj {
                    let v = arg(&a, 0)?.clone();
                    let mut l = l.borrow_mut();
                    if l.len() >= MAX_ITEMS {
                        return Err(bad("list append limit"));
                    }

                    if name == "append" || Value::find_equal(&l, &v)?.is_none() {
                        l.push(v);
                    }
                    Ok(Value::None)
                } else {
                    Err(bad("append target"))
                }
            }
            "extend" => {
                if let Value::Bytes(b) = obj {
                    let raw = arg(&a, 0)?.raw()?;
                    let mut b = b.borrow_mut();
                    checked_count(b.len() + raw.len())?;

                    b.extend(raw);
                    Ok(Value::None)
                } else if let Value::List(l) = obj {
                    let seq = arg(&a, 0)?.sequence()?;
                    let mut l = l.borrow_mut();
                    checked_count(l.len() + seq.len())?;

                    l.extend(seq);
                    Ok(Value::None)
                } else {
                    Err(bad("extend target"))
                }
            }
            "pop" => {
                let k = if let Some(v) = a.first() {
                    v.clone()
                } else {
                    Value::Int(-1)
                };
                match self.remove(&obj, &k) {
                    Ok(v) => Ok(v),
                    Err(e) => a.get(1).cloned().ok_or(e),
                }
            }
            "index" | "find" => {
                let needle = arg(&a, 0)?;
                let index = if let (Value::Text(hay), Value::Text(needle)) = (&obj, needle) {
                    hay.find(needle).map(|pos| hay[..pos].chars().count())
                } else if let Value::Bytes(b) = &obj {
                    let needle = if let Value::Int(n) = needle {
                        vec![u8::try_from(*n).map_err(|_| bad("byte search value"))?]
                    } else {
                        needle.raw()?
                    };
                    if needle.is_empty() {
                        Some(0)
                    } else {
                        b.borrow().windows(needle.len()).position(|v| v == needle)
                    }
                } else {
                    Value::find_equal(&obj.sequence()?, needle)?
                };
                match index {
                    Some(i) => Ok(Value::Int(i as i128)),
                    None if name == "find" => Ok(Value::Int(-1)),
                    _ => Err(bad("sequence search")),
                }
            }
            "count" => {
                let needle = arg(&a, 0)?;
                if let Value::Text(t) = obj {
                    Ok(Value::Int(t.matches(&needle.text()).count() as i128))
                } else {
                    Ok(Value::Int(
                        obj.sequence()?
                            .iter()
                            .try_fold(0i128, |count, v| -> Result<i128> {
                                Ok(count + i128::from(v.equal(needle)?))
                            })?,
                    ))
                }
            }
            "join" => {
                let seq = arg(&a, 0)?.sequence()?;
                if let Value::Bytes(sep) = obj {
                    let sep = sep.borrow();
                    let mut out = vec![];
                    for (i, v) in seq.iter().enumerate() {
                        if i != 0 {
                            out.extend(sep.iter());
                        }
                        out.extend(v.raw()?);
                        checked_count(out.len())?;
                    }
                    Ok(Value::bytes(out))
                } else {
                    let text = seq
                        .iter()
                        .map(Value::text)
                        .collect::<Vec<_>>()
                        .join(&obj.text());
                    checked_count(text.len())?;
                    Ok(Value::Text(text))
                }
            }
            "lower" => Ok(Value::Text(obj.text().to_lowercase())),
            "startswith" | "endswith" => {
                let yes = if let Value::Bytes(_) = obj {
                    let raw = obj.raw()?;
                    let needle = arg(&a, 0)?.raw()?;
                    if name == "startswith" {
                        raw.starts_with(&needle)
                    } else {
                        raw.ends_with(&needle)
                    }
                } else {
                    let text = obj.text();
                    let needle = arg(&a, 0)?.text();
                    if name == "startswith" {
                        text.starts_with(&needle)
                    } else {
                        text.ends_with(&needle)
                    }
                };
                Ok(Value::Bool(yes))
            }
            "strip" | "rstrip" | "lstrip" => {
                if let Value::Bytes(_) = obj {
                    let raw = obj.raw()?;
                    let chars = a
                        .first()
                        .map(Value::raw)
                        .transpose()?
                        .unwrap_or_else(|| b" \t\r\n".to_vec());
                    let mut start = 0;
                    let mut end = raw.len();
                    if name != "rstrip" {
                        while start < end && chars.contains(&raw[start]) {
                            start += 1;
                        }
                    }
                    if name != "lstrip" {
                        while end > start && chars.contains(&raw[end - 1]) {
                            end -= 1;
                        }
                    }
                    Ok(Value::bytes(bytes_at(&raw, start, end - start)?.to_vec()))
                } else {
                    let text = obj.text();
                    let chars = a.first().map(Value::text);
                    let pred =
                        |c: char| chars.as_ref().map_or(c.is_whitespace(), |s| s.contains(c));
                    let text = match name {
                        "rstrip" => text.trim_end_matches(pred),
                        "lstrip" => text.trim_start_matches(pred),
                        _ => text.trim_matches(pred),
                    };
                    Ok(Value::Text(text.to_owned()))
                }
            }
            "replace" => {
                if let Value::Bytes(_) = obj {
                    let data = obj.raw()?;
                    let from = arg(&a, 0)?.raw()?;
                    let to = arg(&a, 1)?.raw()?;
                    if from.is_empty() {
                        return Err(bad("empty byte replacement"));
                    }
                    let mut out = vec![];
                    let mut i = 0;
                    while i < data.len() {
                        if data[i..].starts_with(&from) {
                            out.extend(&to);
                            i += from.len();
                        } else {
                            out.push(data[i]);
                            i += 1;
                        }
                        checked_count(out.len())?;
                    }
                    Ok(Value::bytes(out))
                } else {
                    let text = obj.text().replace(&arg(&a, 0)?.text(), &arg(&a, 1)?.text());
                    checked_count(text.len())?;
                    Ok(Value::Text(text))
                }
            }
            "split" | "splitlines" => {
                if let Value::Bytes(_) = obj {
                    let data = obj.raw()?;
                    let sep = arg(&a, 0)?.raw()?;
                    if sep.is_empty() {
                        return Err(bad("empty byte separator"));
                    }
                    let mut parts = vec![];
                    let mut start = 0;
                    let mut i = 0;
                    while i + sep.len() <= data.len() {
                        if data[i..].starts_with(&sep) {
                            parts.push(Value::bytes(data[start..i].to_vec()));
                            i += sep.len();
                            start = i;
                        } else {
                            i += 1;
                        }
                    }
                    parts.push(Value::bytes(data[start..].to_vec()));
                    Ok(Value::list(parts))
                } else {
                    let t = obj.text();
                    let parts = if name == "splitlines" {
                        t.lines().map(str::to_owned).collect::<Vec<_>>()
                    } else if let Some(sep) = a.first() {
                        t.split(&sep.text()).map(str::to_owned).collect()
                    } else {
                        t.split_whitespace().map(str::to_owned).collect()
                    };
                    Ok(Value::list(parts.into_iter().map(Value::Text).collect()))
                }
            }
            "ljust" => {
                let n = checked_count(arg(&a, 0)?.index()?)?;
                if let Value::Bytes(_) = obj {
                    let mut raw = obj.raw()?;
                    let fill = a
                        .get(1)
                        .map(Value::raw)
                        .transpose()?
                        .unwrap_or_else(|| vec![32]);
                    if fill.len() != 1 {
                        return Err(bad("padding byte"));
                    }
                    raw.resize(n.max(raw.len()), fill[0]);
                    Ok(Value::bytes(raw))
                } else {
                    let mut t = obj.text();
                    let pad = a.get(1).map(Value::text).unwrap_or_else(|| " ".into());
                    if pad.chars().count() != 1 {
                        return Err(bad("padding char"));
                    }
                    let count = n.saturating_sub(t.chars().count());
                    t.push_str(&pad.repeat(count));
                    Ok(Value::Text(t))
                }
            }
            "to_bytes" => {
                let n = checked_count(arg(&a, 0)?.index()?)?;
                if n > 16 {
                    return Err(bad("integer byte width"));
                }
                let v = obj.integer()?;
                let signed = kw.get("signed").is_some_and(Value::truth);
                if !signed && v < 0 {
                    return Err(bad("unsigned integer bytes"));
                }
                let little = a.get(1).map(Value::text).unwrap_or_else(|| "big".into()) == "little";
                let raw = if little {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                };
                Ok(Value::bytes(if little {
                    raw[..n].to_vec()
                } else {
                    raw[16 - n..].to_vec()
                }))
            }
            "translate" => {
                let table = arg(&a, 0)?;
                let mut out = String::new();
                for c in obj.text().chars() {
                    match Self::get(table, &Value::Int(c as i128)) {
                        Ok(Value::None) => {}
                        Ok(Value::Int(n)) => out.push(
                            char::from_u32(
                                u32::try_from(n).map_err(|_| bad("translation character"))?,
                            )
                            .ok_or_else(|| bad("translation character"))?,
                        ),
                        Ok(v) => out.push_str(&v.text()),
                        Err(_) => out.push(c),
                    }
                }
                Ok(Value::Text(out))
            }
            other => Err(bad(format!("unsupported save method {other}"))),
        }
    }
}
fn format_items(fmt: &str) -> Result<Vec<(char, usize)>> {
    let mut out = vec![];
    let mut n = 0usize;
    for c in fmt.chars() {
        if "<>!=@ ".contains(c) {
            continue;
        }
        if c.is_ascii_digit() {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(c.to_digit(10)? as usize))
                .ok_or_else(|| bad("struct count overflow"))?;
            checked_count(n)?;
        } else {
            if !"bBhHiIqQsx".contains(c) {
                return Err(bad(format!("struct scalar {c}")));
            }
            out.push((c, if n == 0 { 1 } else { n }));
            n = 0;
        }
    }
    Ok(out)
}
fn scalar_width(c: char) -> usize {
    match c {
        'h' | 'H' => 2,
        'i' | 'I' => 4,
        'q' | 'Q' => 8,
        _ => 1,
    }
}
fn struct_width(fmt: &str) -> Result<usize> {
    let mut n = 0;
    for (c, count) in format_items(fmt)? {
        n += scalar_width(c) * count;
        checked_count(n)?;
    }
    Ok(n)
}
fn unpack(fmt: &str, bytes: &[u8], offset: usize) -> Result<Vec<Value>> {
    let width = struct_width(fmt)?;
    bytes_at(bytes, offset, width)?;
    let endian = if fmt.starts_with('>') || fmt.starts_with('!') {
        Endian::Big
    } else {
        Endian::Little
    };
    let mut p = offset;
    let mut out = vec![];
    for (c, count) in format_items(fmt)? {
        if c == 's' {
            out.push(Value::bytes(bytes_at(bytes, p, count)?.to_vec()));
            p += count;
            continue;
        }
        for _ in 0..count {
            let v = match c {
                'B' => U8Field::new(p).read(bytes)? as i128,
                'b' => I8Field::new(p).read(bytes)? as i128,
                'H' => U16Field::new(p, endian).read(bytes)? as i128,
                'h' => I16Field::new(p, endian).read(bytes)? as i128,
                'I' => U32Field::new(p, endian).read(bytes)? as i128,
                'i' => I32Field::new(p, endian).read(bytes)? as i128,
                'Q' => U64Field::new(p, endian).read(bytes)? as i128,
                'q' => U64Field::new(p, endian).read(bytes)? as i64 as i128,
                'x' => {
                    p += 1;
                    continue;
                }
                _ => return Err(bad("unpack scalar")),
            };
            out.push(Value::Int(v));
            p += scalar_width(c);
        }
    }
    Ok(out)
}
fn pack(fmt: &str, values: &[Value]) -> Result<Vec<u8>> {
    let mut out = vec![0; struct_width(fmt)?];
    let endian = if fmt.starts_with('>') || fmt.starts_with('!') {
        Endian::Big
    } else {
        Endian::Little
    };
    let mut p = 0;
    let mut i = 0;
    for (c, count) in format_items(fmt)? {
        if c == 's' {
            let raw = arg(values, i)?.raw()?;
            i += 1;
            let n = count.min(raw.len());
            patch_bytes(&mut out, p, &raw[..n])?;
            p += count;
            continue;
        }
        for _ in 0..count {
            if c == 'x' {
                p += 1;
                continue;
            }
            let v = arg(values, i)?.integer()?;
            i += 1;
            let err = || bad("packed scalar out of range");
            match c {
                'B' => U8Field::new(p).write(&mut out, u8::try_from(v).map_err(|_| err())?)?,
                'b' => I8Field::new(p).write(&mut out, i8::try_from(v).map_err(|_| err())?)?,
                'H' => U16Field::new(p, endian)
                    .write(&mut out, u16::try_from(v).map_err(|_| err())?)?,
                'h' => I16Field::new(p, endian)
                    .write(&mut out, i16::try_from(v).map_err(|_| err())?)?,
                'I' => U32Field::new(p, endian)
                    .write(&mut out, u32::try_from(v).map_err(|_| err())?)?,
                'i' => I32Field::new(p, endian)
                    .write(&mut out, i32::try_from(v).map_err(|_| err())?)?,
                'Q' => U64Field::new(p, endian)
                    .write(&mut out, u64::try_from(v).map_err(|_| err())?)?,
                'q' => U64Field::new(p, endian)
                    .write(&mut out, i64::try_from(v).map_err(|_| err())? as u64)?,
                _ => return Err(bad("pack scalar")),
            }
            p += scalar_width(c);
        }
    }
    if i != values.len() {
        return Err(bad("pack arity"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_arithmetic_overflow_returns_errors() {
        let mut e = Engine::new(&[], "synthetic").unwrap();
        assert!(e
            .native(
                "sum",
                vec![Value::bytes(vec![1]), Value::Int(i128::MAX)],
                HashMap::new()
            )
            .is_err());
        assert!(e
            .native(
                "enumerate",
                vec![Value::bytes(vec![1, 2]), Value::Int(i128::MAX)],
                HashMap::new()
            )
            .is_err());
    }
    #[test]
    fn word_checksum_bindings_keep_high_bytes_and_check_ranges() {
        let mut e = Engine::new(&[], "synthetic").unwrap();
        e.checksums.insert(
            "sum".into(),
            serde_json::json!({"parameters": {
                "unit":"word-le", "fold":"sum", "bits":16,
                "seed":0, "xor_out":0, "offset_base":0
            }}),
        );
        let words = Value::list(vec![Value::Int(0xffff), Value::Int(0x1234)]);
        let result = e
            .native(
                "definition_checksum",
                vec![Value::Text("sum".into()), words],
                HashMap::new(),
            )
            .unwrap();
        assert_eq!(result.integer().unwrap(), 0x1233);
        assert!(e
            .native(
                "definition_checksum",
                vec![
                    Value::Text("sum".into()),
                    Value::list(vec![Value::Int(65536)])
                ],
                HashMap::new()
            )
            .is_err());
        assert!(e
            .native(
                "struct.unpack_from",
                vec![
                    Value::Text("<H".into()),
                    Value::bytes(vec![]),
                    Value::Int(4)
                ],
                HashMap::new()
            )
            .is_err());
    }
    #[test]
    fn semantic_primitives_preserve_sets_unicode_and_wide_flags() {
        let mut e = Engine::new(&[], "synthetic").unwrap();
        let empty = HashMap::new;
        let set = e
            .native(
                "set",
                vec![Value::list(vec![Value::Int(1), Value::Int(1)])],
                empty(),
            )
            .unwrap();
        assert_eq!(set.len().unwrap(), 1);
        let union = e
            .binary("BitOr", set, Value::set(vec![Value::Int(2)]))
            .unwrap();
        assert_eq!(union.len().unwrap(), 2);
        let n = e
            .native(
                "int.from_bytes",
                vec![Value::bytes(vec![255; 32]), Value::Text("little".into())],
                empty(),
            )
            .unwrap();
        let bit = e.binary("RShift", n, Value::Int(255)).unwrap();
        assert_eq!(bit.integer().unwrap(), 1);
        let text = e
            .method(
                "decode",
                Value::bytes(vec![0xa4, 0xa2]),
                vec![Value::Text("euc_jp".into())],
                empty(),
            )
            .unwrap();
        assert_eq!(text.text(), "あ");
        assert_eq!(
            e.method(
                "hex",
                Value::bytes(vec![1, 2]),
                vec![Value::Text(" ".into())],
                empty()
            )
            .unwrap()
            .text(),
            "01 02"
        );
        assert!(e
            .method(
                "index",
                Value::bytes(vec![1, 255]),
                vec![Value::Int(255)],
                empty()
            )
            .unwrap()
            .equal(&Value::Int(1))
            .unwrap());
        assert_eq!(format_value(&Value::Float(2.5), "05.2f").unwrap(), "02.50");
    }
}
