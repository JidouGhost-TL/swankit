use crate::{
    limit, malformed, CharacterMap, CharacterMapDefinition, DecodedText, FontDefinition, TextLimits,
};
use formatkit_core::{bytes_at, bytes_at_mut, Endian, Result};
use formatkit_layout::{U16Field, U32Field};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PointerEndian {
    #[default]
    Little,
    Big,
}
impl PointerEndian {
    fn value(self) -> Endian {
        match self {
            Self::Little => Endian::Little,
            Self::Big => Endian::Big,
        }
    }
}

fn one() -> usize {
    1
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentWindow {
    pub minimum: u16,
    pub maximum: u16,
    pub base: usize,
    /// Add the containing pointer record's 64 KiB bank.
    #[serde(default)]
    pub record_bank: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PointerDefinition {
    Integer {
        width: u8,
        #[serde(default)]
        endian: PointerEndian,
        #[serde(default)]
        base: usize,
        #[serde(default = "one")]
        scale: usize,
    },
    /// Off16/seg16 far pointers translated by explicit segment windows.
    Far {
        #[serde(default)]
        endian: PointerEndian,
        windows: Vec<SegmentWindow>,
        #[serde(default)]
        maximum_offset: Option<u16>,
    },
}
impl PointerDefinition {
    fn width(&self) -> Result<usize> {
        match self {
            Self::Integer { width, .. } if *width == 2 || *width == 4 => Ok(*width as usize),
            Self::Far { .. } => Ok(4),
            _ => Err(malformed("pointer width must be 2 or 4")),
        }
    }
    pub fn target(&self, bytes: &[u8], at: usize) -> Result<usize> {
        match self {
            Self::Integer {
                width,
                endian,
                base,
                scale,
            } => {
                let value = match width {
                    2 => u32::from(U16Field::new(at, endian.value()).read(bytes)?),
                    4 => U32Field::new(at, endian.value()).read(bytes)?,
                    _ => return Err(malformed("pointer width must be 2 or 4")),
                };
                if *scale == 0 {
                    return Err(malformed("zero pointer scale"));
                }
                (value as usize)
                    .checked_mul(*scale)
                    .and_then(|n| base.checked_add(n))
                    .ok_or_else(|| malformed("pointer target overflow"))
            }
            Self::Far {
                endian,
                windows,
                maximum_offset,
            } => {
                let off = U16Field::new(at, endian.value()).read(bytes)?;
                let seg_at = at
                    .checked_add(2)
                    .ok_or_else(|| malformed("far pointer offset overflow"))?;
                let seg = U16Field::new(seg_at, endian.value()).read(bytes)?;
                if maximum_offset.is_some_and(|max| off > max) {
                    return Err(malformed("far pointer offset outside policy"));
                }
                let window = windows
                    .iter()
                    .find(|w| (w.minimum..=w.maximum).contains(&seg))
                    .ok_or_else(|| malformed("far pointer segment outside definition"))?;
                let bank = if window.record_bank { at & !65535 } else { 0 };
                bank.checked_add(window.base)
                    .and_then(|n| n.checked_add(usize::from(seg - window.minimum) * 16))
                    .and_then(|n| n.checked_add(off as usize))
                    .ok_or_else(|| malformed("far pointer target overflow"))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StringTableDefinition {
    Offsets {
        offsets: Vec<usize>,
        maximum_bytes: usize,
        #[serde(default)]
        require_end: bool,
    },
    Fixed {
        offset: usize,
        count: usize,
        stride: usize,
        length: usize,
        #[serde(default)]
        require_end: bool,
    },
    Pointers {
        offset: usize,
        count: usize,
        stride: usize,
        #[serde(default)]
        pointer_offset: usize,
        pointer: PointerDefinition,
        maximum_bytes: usize,
        #[serde(default)]
        require_end: bool,
        /// Skip a pool marker only if marker + following byte match this rule.
        #[serde(default)]
        skip_marker: Option<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpedString {
    pub offset: usize,
    pub decoded: DecodedText,
}

impl StringTableDefinition {
    pub fn dump(
        &self,
        bytes: &[u8],
        map: &CharacterMap,
        limits: TextLimits,
        maximum_strings: usize,
    ) -> Result<Vec<DumpedString>> {
        let (count, offset, stride) = match self {
            Self::Offsets { offsets, .. } => (offsets.len(), 0, 0),
            Self::Fixed {
                offset,
                count,
                stride,
                ..
            }
            | Self::Pointers {
                offset,
                count,
                stride,
                ..
            } => (*count, *offset, *stride),
        };
        limit("text table strings", count, maximum_strings)?;
        if let Self::Fixed { length, .. } = self {
            if *length > stride || *length == 0 {
                return Err(malformed("invalid fixed string record"));
            }
        }
        if let Self::Pointers {
            pointer,
            pointer_offset,
            ..
        } = self
        {
            if pointer_offset
                .checked_add(pointer.width()?)
                .is_none_or(|n| n > stride)
            {
                return Err(malformed("pointer exceeds table stride"));
            }
        }
        if !matches!(self, Self::Offsets { .. }) {
            let length = count
                .checked_mul(stride)
                .ok_or_else(|| malformed("text table extent overflow"))?;
            bytes_at(bytes, offset, length)?;
        }
        let mut out = Vec::new();
        let mut total_output = 0usize;
        let mut total_tokens = 0usize;
        for i in 0..count {
            let at = i
                .checked_mul(stride)
                .and_then(|n| n.checked_add(offset))
                .ok_or_else(|| malformed("text table offset overflow"))?;
            let (start, maximum, require_end) = match self {
                Self::Offsets {
                    offsets,
                    maximum_bytes,
                    require_end,
                } => (offsets[i], *maximum_bytes, *require_end),
                Self::Fixed {
                    length,
                    require_end,
                    ..
                } => (at, *length, *require_end),
                Self::Pointers {
                    pointer_offset,
                    pointer,
                    maximum_bytes,
                    require_end,
                    skip_marker,
                    ..
                } => {
                    let p = at
                        .checked_add(*pointer_offset)
                        .ok_or_else(|| malformed("pointer record overflow"))?;
                    let mut start = pointer.target(bytes, p)?;
                    if let Some(marker) = skip_marker {
                        let pair = bytes_at(bytes, start, 2)?;
                        if pair[0] == *marker && pair[1] != *marker {
                            start = start
                                .checked_add(1)
                                .ok_or_else(|| malformed("pool marker overflow"))?;
                        }
                    }
                    (start, *maximum_bytes, *require_end)
                }
            };
            if maximum == 0 {
                return Err(malformed("zero string extent"));
            }
            let available = bytes
                .len()
                .checked_sub(start)
                .ok_or_else(|| malformed("string target outside source"))?;
            let input = bytes_at(bytes, start, maximum.min(available))?;
            let decoded = map.decode(input, limits)?;
            if require_end && !decoded.terminated {
                return Err(malformed(format!("unterminated string at {start:#x}")));
            }
            total_output = total_output
                .checked_add(decoded.text.len())
                .ok_or_else(|| malformed("dump output overflow"))?;
            total_tokens = total_tokens
                .checked_add(decoded.tokens.len())
                .ok_or_else(|| malformed("dump token overflow"))?;
            limit("text dump output bytes", total_output, limits.output_bytes)?;
            limit("text dump tokens", total_tokens, limits.tokens)?;
            out.push(DumpedString {
                offset: start,
                decoded,
            });
        }
        Ok(out)
    }
}

/// Encode and fit completely before modifying the resident slot. The caller
/// owns pointer relocation/checksums; this primitive never relocates a table.
pub fn insert_text(
    bytes: &mut [u8],
    offset: usize,
    slot_bytes: usize,
    map: &CharacterMap,
    text: &str,
    padding: u8,
) -> Result<usize> {
    bytes_at(bytes, offset, slot_bytes)?;
    let encoded = map.encode(text, slot_bytes)?;
    let target = bytes_at_mut(bytes, offset, slot_bytes)?;
    target.fill(padding);
    target[..encoded.len()].copy_from_slice(&encoded);
    Ok(encoded.len())
}

/// Per-engine metadata is data; real strings and expectations stay external.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextDefinition {
    pub schema: String,
    pub id: String,
    pub map: CharacterMapDefinition,
    #[serde(default)]
    pub fonts: BTreeMap<String, FontDefinition>,
    #[serde(default)]
    pub tables: BTreeMap<String, StringTableDefinition>,
}
impl TextDefinition {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        limit("text definition JSON bytes", bytes.len(), 4 * 1024 * 1024)?;
        let value: Self = serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
        if value.schema != "swankit-text-def/1" {
            return Err(malformed("unknown text definition schema"));
        }
        CharacterMap::new(value.map.clone())?;
        Ok(value)
    }
    pub fn character_map(&self) -> Result<CharacterMap> {
        CharacterMap::new(self.map.clone())
    }
}

/// Curated definitions use neutral IDs so consumer code remains title-independent.
pub fn builtin_definition(id: &str) -> Result<TextDefinition> {
    let json = match id {
        "byte-pages-1" => include_bytes!("../data/byte-pages-1.json").as_slice(),
        "word-dialog-1" => include_bytes!("../data/word-dialog-1.json").as_slice(),
        "word-dialog-2" => include_bytes!("../data/word-dialog-2.json").as_slice(),
        "word-mask-1" => include_bytes!("../data/word-mask-1.json").as_slice(),
        "byte-prefix-1" => include_bytes!("../data/byte-prefix-1.json").as_slice(),
        _ => return Err(malformed("unknown builtin text definition")),
    };
    TextDefinition::from_json(json)
}
