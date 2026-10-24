//! Generated WonderSwan game save grammars, with bounded semantic interpretation.
#![forbid(unsafe_code)]
mod cp932;
mod engine;
mod jis0208;
mod native;
#[path = "generated/records.rs"]
mod records;
mod transform;
mod value;

use formatkit_core::{bytes_at, Endian, Result};
use formatkit_layout::{patch_bytes, U16Field, U32Field, U8Field};
use serde_json::Value as Json;
use std::{collections::HashMap, sync::OnceLock};
use value::{bad, Environment, Value};

/// A generated fixed field; symbolic/conditional fields live in the semantic program.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub medium: String,
    pub offset: usize,
    pub size: usize,
    pub encoding: String,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Constraint {
    UnsignedRange {
        minimum: u64,
        maximum: u64,
    },
    Enum {
        values: Vec<u64>,
    },
    PackedBcd {
        digits: u8,
        byte_order: String,
        minimum: u64,
        maximum: u64,
    },
}
impl Field {
    pub fn read(&self, bytes: &[u8]) -> Result<Json> {
        Ok(match (self.size, self.encoding.as_str()) {
            (1, "u8") => serde_json::json!(U8Field::new(self.offset).read(bytes)?),
            (2, "u16le") => {
                serde_json::json!(U16Field::new(self.offset, Endian::Little).read(bytes)?)
            }
            (4, "u32le") => {
                serde_json::json!(U32Field::new(self.offset, Endian::Little).read(bytes)?)
            }
            _ => serde_json::json!(bytes_at(bytes, self.offset, self.size)?),
        })
    }
    /// Edit the stored representation without changing surrounding bytes.
    pub fn write(&self, bytes: &mut [u8], value: &Json) -> Result<()> {
        let n = || {
            value
                .as_u64()
                .ok_or_else(|| bad("field requires unsigned scalar"))
        };
        match (self.size, self.encoding.as_str()) {
            (1, "u8") => U8Field::new(self.offset).write(
                bytes,
                u8::try_from(n()?).map_err(|_| bad("u8 field overflow"))?,
            ),
            (2, "u16le") => U16Field::new(self.offset, Endian::Little).write(
                bytes,
                u16::try_from(n()?).map_err(|_| bad("u16 field overflow"))?,
            ),
            (4, "u32le") => U32Field::new(self.offset, Endian::Little).write(
                bytes,
                u32::try_from(n()?).map_err(|_| bad("u32 field overflow"))?,
            ),
            _ => {
                let v = value
                    .as_array()
                    .ok_or_else(|| bad("field requires byte array"))?;
                if v.len() != self.size {
                    return Err(bad("field edit extent"));
                }
                let data = v
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| u8::try_from(n).ok())
                            .ok_or_else(|| bad("byte edit range"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                patch_bytes(bytes, self.offset, &data)
            }
        }
    }
}
/// Game identity and its generated grammar.
#[derive(Debug)]
pub struct Format {
    pub id: String,
    pub fields: Vec<Field>,
    pub records: Vec<(String, usize)>,
    constraints: Vec<Option<Constraint>>,
    copy_groups: Vec<CopyGroup>,
    entry: Json,
    program: Json,
    definition: Json,
}
#[derive(Debug, Clone)]
pub struct CopyGroup {
    pub name: String,
    pub fields: Vec<usize>,
}
include!(concat!(env!("OUT_DIR"), "/definitions.rs"));
fn models() -> &'static Result<Vec<Format>> {
    static MODELS: OnceLock<Result<Vec<Format>>> = OnceLock::new();
    MODELS.get_or_init(|| {
        DEFINITIONS
            .iter()
            .map(|json| Format::from_json(json))
            .collect()
    })
}
pub fn formats() -> Result<&'static [Format]> {
    models().as_deref().map_err(|e| bad(e.to_string()))
}
pub fn format(id: &str) -> Result<&'static Format> {
    formats()?
        .iter()
        .find(|f| f.id == id)
        .ok_or_else(|| bad("unknown save format"))
}

/// External semantic tables, such as ROM-derived password-hash lookup data.
#[derive(Debug, Default)]
pub struct DecodeContext {
    pub tables: HashMap<String, Vec<u8>>,
    pub images: HashMap<String, Vec<u8>>,
}
impl Format {
    /// Load a versioned definition; parsing and semantic limits reject malformed data.
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 4 * value::MAX_ITEMS {
            return Err(bad("definition size limit"));
        }
        let definition: Json =
            serde_json::from_str(json).map_err(|e| bad(format!("save definition: {e}")))?;
        let version = definition["schema"]
            .as_str()
            .ok_or_else(|| bad("save definition schema missing"))?;
        if !matches!(version, "swankit-save-def/1" | "swankit-save-def/2") {
            return Err(bad("unsupported save definition schema"));
        }
        if version == "swankit-save-def/2" {
            const TOP_LEVEL_KEYS: &[&str] = &[
                "schema",
                "id",
                "implementation",
                "media",
                "entry",
                "fields",
                "records",
                "blocks",
                "checksums",
                "validity",
                "primitives",
                "program",
                "copy_groups",
            ];
            let object = definition
                .as_object()
                .ok_or_else(|| bad("save definition must be an object"))?;
            if object
                .keys()
                .any(|key| !TOP_LEVEL_KEYS.contains(&key.as_str()))
            {
                return Err(bad("unknown save definition property"));
            }
            let field_keys = [
                "name",
                "medium",
                "offset",
                "size",
                "encoding",
                "type",
                "meaning",
                "confidence",
                "evidence",
                "array",
                "constraint",
            ];
            for field in definition["fields"]
                .as_array()
                .ok_or_else(|| bad("definition array"))?
            {
                let object = field
                    .as_object()
                    .ok_or_else(|| bad("field definition must be an object"))?;
                if object.keys().any(|key| !field_keys.contains(&key.as_str())) {
                    return Err(bad("unknown field property"));
                }
            }
        }
        if [
            "replica_groups",
            "copy_policy",
            "read_policy",
            "write_policy",
        ]
        .iter()
        .any(|key| definition.get(*key).is_some())
        {
            return Err(bad("unknown copy policy; use copy_groups"));
        }
        let text = |v: &Json| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| bad("definition string"))
        };
        let extent = |v: &Json| {
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n <= value::MAX_ITEMS)
                .ok_or_else(|| bad("definition extent"))
        };
        let array = |v: &Json| v.as_array().cloned().ok_or_else(|| bad("definition array"));
        let media = array(&definition["media"])?;
        if media.is_empty() {
            return Err(bad("definition has no media"));
        }
        for m in &media {
            if !matches!(
                m["kind"].as_str(),
                Some("cartridge-sram" | "cartridge-eeprom" | "internal-eeprom")
            ) {
                return Err(bad("medium kind"));
            }
            let sizes = array(&m["sizes"])?;
            if sizes.is_empty() {
                return Err(bad("medium has no sizes"));
            }
            for size in &sizes {
                if extent(size)? == 0 {
                    return Err(bad("empty medium"));
                }
            }
        }
        for key in ["primitives", "checksums", "validity"] {
            if !definition[key].is_object() {
                return Err(bad("definition policy object"));
            }
        }
        array(&definition["blocks"])?;
        if definition["program"]["t"] != "Module" {
            return Err(bad("definition program root"));
        }
        array(&definition["program"]["body"])?;
        if !matches!(
            definition["entry"]["mode"].as_str(),
            Some("program" | "sys-argv" | "arguments" | "direct")
        ) {
            return Err(bad("definition entry mode"));
        }
        let raw_fields = array(&definition["fields"])?;
        let fields = raw_fields
            .iter()
            .map(|f| {
                let offset = extent(&f["offset"])?;
                let size = extent(&f["size"])?;
                if size == 0
                    || offset
                        .checked_add(size)
                        .is_none_or(|end| end > value::MAX_ITEMS)
                {
                    return Err(bad("field extent"));
                }
                Ok(Field {
                    name: text(&f["name"])?,
                    medium: text(&f["medium"])?,
                    offset,
                    size,
                    encoding: text(&f["encoding"])?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let constraints = raw_fields
            .iter()
            .zip(&fields)
            .map(|(raw, field)| {
                let Some(value) = raw.get("constraint") else {
                    return Ok(None);
                };
                if version != "swankit-save-def/2" {
                    return Err(bad("field constraints require swankit-save-def/2"));
                }
                let policy: Constraint = serde_json::from_value(value.clone())
                    .map_err(|_| bad("malformed field constraint"))?;
                let field_type = raw["type"].as_str();
                let numeric_storage = matches!(
                    (field.size, field.encoding.as_str()),
                    (1, "u8") | (2, "u16le") | (4, "u32le")
                );
                let scalar = numeric_storage && field_type == Some("unsigned");
                let limit = match field.size {
                    1 => u8::MAX as u64,
                    2 => u16::MAX as u64,
                    4 => u32::MAX as u64,
                    _ => 0,
                };
                match &policy {
                    Constraint::UnsignedRange { minimum, maximum } => {
                        if !scalar || minimum > maximum || *maximum > limit {
                            return Err(bad(
                                "invalid unsigned-range constraint for field representation",
                            ));
                        }
                    }
                    Constraint::Enum { values } => {
                        if !scalar
                            || values.is_empty()
                            || values.iter().any(|n| *n > limit)
                            || values
                                .iter()
                                .collect::<std::collections::HashSet<_>>()
                                .len()
                                != values.len()
                        {
                            return Err(bad("invalid enum constraint for field representation"));
                        }
                    }
                    Constraint::PackedBcd {
                        digits,
                        byte_order,
                        minimum,
                        maximum,
                    } => {
                        if numeric_storage
                            || field_type != Some("bytes")
                            || *digits == 0
                            || *digits > 16
                            || usize::from(*digits) > field.size.saturating_mul(2)
                            || !matches!(byte_order.as_str(), "little" | "big")
                            || minimum > maximum
                            || *maximum > 99_999_999_999_999_999u64
                        {
                            return Err(bad(
                                "invalid packed-bcd constraint for field representation",
                            ));
                        }
                        let cap = 10u64
                            .checked_pow(u32::from(*digits))
                            .and_then(|n| n.checked_sub(1))
                            .ok_or_else(|| bad("packed-bcd digit limit"))?;
                        if *maximum > cap {
                            return Err(bad("packed-bcd range exceeds declared digits"));
                        }
                    }
                }
                Ok(Some(policy))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut copy_groups = Vec::new();
        if version == "swankit-save-def/2" {
            if let Some(raw_groups) = definition.get("copy_groups") {
                let groups = raw_groups
                    .as_array()
                    .ok_or_else(|| bad("copy_groups must be an array"))?;
                let mut group_names = std::collections::HashSet::new();
                let mut claimed = std::collections::HashSet::new();
                for group in groups {
                    if !group.is_object()
                        || group
                            .as_object()
                            .is_some_and(|m| m.keys().any(|key| key != "name" && key != "fields"))
                    {
                        return Err(bad("unknown copy group property"));
                    }
                    let name = group["name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| bad("copy group name"))?
                        .to_owned();
                    if !group_names.insert(name.clone()) {
                        return Err(bad("duplicate copy group name"));
                    }
                    let members = group["fields"]
                        .as_array()
                        .ok_or_else(|| bad("copy group fields must be an array"))?;
                    if members.len() < 2 {
                        return Err(bad("copy group requires at least two fields"));
                    }
                    let mut indexes = Vec::new();
                    for member in members {
                        let member = member
                            .as_str()
                            .ok_or_else(|| bad("copy group member must be a field name"))?;
                        let matches: Vec<_> = fields
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| f.name == member)
                            .map(|(i, _)| i)
                            .collect();
                        if matches.is_empty() {
                            return Err(bad(format!("dangling copy group member: {member}")));
                        }
                        if matches.len() != 1 {
                            return Err(bad(format!("ambiguous copy group member: {member}")));
                        }
                        let i = matches[0];
                        if !claimed.insert(i) || indexes.contains(&i) {
                            return Err(bad("duplicate copy group membership"));
                        }
                        for previous in &indexes {
                            let a: &Field = &fields[*previous];
                            let b: &Field = &fields[i];
                            if (a.size, a.encoding.as_str(), a.medium.as_str())
                                != (b.size, b.encoding.as_str(), b.medium.as_str())
                            {
                                return Err(bad("incompatible copy group member representation"));
                            }
                            let a_end = a
                                .offset
                                .checked_add(a.size)
                                .ok_or_else(|| bad("copy group field extent"))?;
                            let b_end = b
                                .offset
                                .checked_add(b.size)
                                .ok_or_else(|| bad("copy group field extent"))?;
                            if a.offset < b_end && b.offset < a_end {
                                return Err(bad("overlapping copy group member extents"));
                            }
                        }
                        indexes.push(i);
                    }
                    copy_groups.push(CopyGroup {
                        name,
                        fields: indexes,
                    });
                }
            }
        } else if definition.get("copy_groups").is_some() {
            return Err(bad("copy groups require swankit-save-def/2"));
        }
        for block in array(&definition["blocks"])? {
            if extent(&block["field"])? >= fields.len() {
                return Err(bad("block field index"));
            }
        }
        let records = array(&definition["records"])?
            .iter()
            .map(|r| {
                let width = extent(&r["width"])?;
                if width == 0 {
                    return Err(bad("empty record"));
                }
                Ok((text(&r["name"])?, width))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            id: text(&definition["id"])?,
            fields,
            records,
            constraints,
            copy_groups,
            entry: definition["entry"].clone(),
            program: definition["program"].clone(),
            definition,
        })
    }
    /// Portable source definition, including symbolic semantics and policy.
    pub fn definition(&self) -> &Json {
        &self.definition
    }
    pub fn copy_group_for_field(&self, field: usize) -> Option<&CopyGroup> {
        self.copy_groups
            .iter()
            .find(|group| group.fields.contains(&field))
    }
    pub fn copy_groups(&self) -> &[CopyGroup] {
        &self.copy_groups
    }
    pub fn validate_field_value(&self, field: usize, value: &Json) -> Result<()> {
        let f = self
            .fields
            .get(field)
            .ok_or_else(|| bad("unknown field index"))?;
        let Some(policy) = self.constraints.get(field).and_then(Option::as_ref) else {
            return Ok(());
        };
        match policy {
            Constraint::UnsignedRange { minimum, maximum } => {
                let n = value
                    .as_u64()
                    .ok_or_else(|| bad("field constraint requires unsigned integer"))?;
                if n < *minimum || n > *maximum {
                    return Err(bad(format!(
                        "value {n} is outside field constraint {minimum}..={maximum}"
                    )));
                }
            }
            Constraint::Enum { values } => {
                let n = value
                    .as_u64()
                    .ok_or_else(|| bad("field constraint requires unsigned integer"))?;
                if !values.contains(&n) {
                    return Err(bad(format!("value {n} is not an allowed field value")));
                }
            }
            Constraint::PackedBcd {
                digits,
                byte_order,
                minimum,
                maximum,
            } => {
                let input = value
                    .as_array()
                    .ok_or_else(|| bad("packed BCD field requires byte array"))?;
                if input.len() != f.size {
                    return Err(bad("field edit extent"));
                }
                let mut bytes = input
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| u8::try_from(n).ok())
                            .ok_or_else(|| bad("byte edit range"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if byte_order == "little" {
                    bytes.reverse();
                }
                let padding = bytes
                    .len()
                    .checked_mul(2)
                    .and_then(|n| n.checked_sub(usize::from(*digits)))
                    .ok_or_else(|| bad("packed BCD digit extent"))?;
                let mut numeric = 0u64;
                for (index, byte) in bytes.into_iter().enumerate() {
                    for (within_byte, digit) in [byte >> 4, byte & 0x0f].into_iter().enumerate() {
                        let nibble = index * 2 + within_byte;
                        if nibble < padding {
                            if digit != 0 {
                                return Err(bad("nonzero packed BCD padding digit"));
                            }
                        } else {
                            if digit > 9 {
                                return Err(bad("invalid packed BCD digit"));
                            }
                            numeric = numeric
                                .checked_mul(10)
                                .and_then(|v| v.checked_add(u64::from(digit)))
                                .ok_or_else(|| bad("packed BCD value overflow"))?;
                        }
                    }
                }
                if numeric < *minimum || numeric > *maximum {
                    return Err(bad(format!("packed BCD value {numeric} is outside field constraint {minimum}..={maximum}")));
                }
            }
        }
        Ok(())
    }
    fn engine(&self, bytes: &[u8], name: &str, context: &DecodeContext) -> Result<engine::Engine> {
        let mut engine = engine::Engine::new(bytes, name)?;
        for data in context.tables.values().chain(context.images.values()) {
            if data.len() > value::MAX_ITEMS {
                return Err(bad("context image limit"));
            }
        }
        engine.context = context.tables.clone();
        engine.files.extend(context.images.clone());
        engine.checksums = serde_json::from_value(self.definition["checksums"].clone())
            .map_err(|_| bad("checksum bindings"))?;
        engine.primitives = serde_json::from_value(self.definition["primitives"].clone())
            .map_err(|_| bad("primitive bindings"))?;
        Ok(engine)
    }
    /// Edit an explicitly selected stored field. Field names need not be unique.
    pub fn set_field(&self, bytes: &mut [u8], field: usize, value: &Json) -> Result<()> {
        self.set_field_with_options(bytes, field, value, false)
    }
    /// Edit a stored field, optionally bypassing only its semantic value policy.
    /// Storage bounds and representation checks always apply.
    pub fn set_field_with_options(
        &self,
        bytes: &mut [u8],
        field: usize,
        value: &Json,
        skip_validation: bool,
    ) -> Result<()> {
        if !skip_validation {
            self.validate_field_value(field, value)?;
        }
        self.fields
            .get(field)
            .ok_or_else(|| bad("unknown field index"))?
            .write(bytes, value)
    }
    /// Collect every decoded validity/checksum flag with its JSON pointer.
    pub fn verify(&self, bytes: &[u8], name: &str) -> Result<Json> {
        self.verify_with_context(bytes, name, &DecodeContext::default())
    }
    pub fn verify_with_context(
        &self,
        bytes: &[u8],
        name: &str,
        context: &DecodeContext,
    ) -> Result<Json> {
        fn flags(v: &Json, path: &str, out: &mut serde_json::Map<String, Json>) {
            match v {
                Json::Object(m) => {
                    for (k, v) in m {
                        let p = format!("{path}/{}", k.replace('~', "~0").replace('/', "~1"));
                        if matches!(
                            k.as_str(),
                            "valid"
                                | "checksum_ok"
                                | "signature_valid"
                                | "ok"
                                | "crc_ok"
                                | "checksum_valid"
                        ) && v.is_boolean()
                        {
                            out.insert(p.clone(), v.clone());
                        }
                        flags(v, &p, out);
                    }
                }
                Json::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        flags(v, &format!("{path}/{i}"), out);
                    }
                }
                _ => {}
            }
        }
        let decoded = self.decode_with_context(bytes, name, context)?;
        let mut result = serde_json::Map::new();
        flags(&decoded, "", &mut result);
        Ok(Json::Object(result))
    }

    /// Decode to the oracle's JSON value tree. Only the transport key `file` is removed.
    pub fn decode(&self, bytes: &[u8], name: &str) -> Result<Json> {
        self.decode_with_context(bytes, name, &DecodeContext::default())
    }
    pub fn decode_with_context(
        &self,
        bytes: &[u8],
        name: &str,
        context: &DecodeContext,
    ) -> Result<Json> {
        let mut engine = self.engine(bytes, name, context)?;
        let scope = engine.initialize(&self.program)?;
        let argv = Value::list(vec![Value::Text("save".into()), Value::Text(name.into())]);
        Environment::set(&scope, "sys_argv", argv.clone())?;
        let result = if self.entry["mode"] == "direct" {
            let fun = Environment::get(
                &scope,
                self.entry["decode"]
                    .as_str()
                    .ok_or_else(|| bad("direct decode name"))?,
            )
            .ok_or_else(|| bad("direct decode function"))?;
            let value = engine.call(
                fun,
                vec![Value::bytes(bytes.to_vec()), Value::Text(name.into())],
                HashMap::new(),
            )?;
            let json = value.to_json()?;
            engine.output.push(json);
            Value::None
        } else {
            let main =
                Environment::get(&scope, "main").ok_or_else(|| bad("save program lacks entry"))?;
            let args = match self.entry["mode"].as_str() {
                Some("sys-argv") => vec![],
                Some("arguments") => vec![Value::list(vec![Value::Text(name.into())])],
                _ => vec![argv],
            };
            engine.call(main, args, HashMap::new())?
        };
        if result.truth() {
            return Err(bad(format!("save parser returned {}", result.text())));
        }
        if engine.output.len() != 1 {
            return Err(bad(format!(
                "save program emitted {} JSON values",
                engine.output.len()
            )));
        }
        let mut out = engine.output.remove(0);
        if let Json::Array(a) = &mut out {
            if a.len() == 1 {
                out = a.remove(0);
            }
        }
        normalize(&mut out);
        Ok(out)
    }
    /// Apply the title's documented repair transactionally, preserving image length.
    pub fn fix(&self, bytes: &mut [u8], name: &str) -> Result<()> {
        self.fix_with_context(bytes, name, &DecodeContext::default())
    }
    pub fn fix_with_context(
        &self,
        bytes: &mut [u8],
        name: &str,
        context: &DecodeContext,
    ) -> Result<()> {
        let mut engine = self.engine(bytes, name, context)?;
        let scope = engine.initialize(&self.program)?;
        let argv = Value::list(vec![
            Value::Text("save".into()),
            Value::Text("--fix".into()),
            Value::Text(name.into()),
            Value::Text("repaired".into()),
        ]);
        Environment::set(&scope, "sys_argv", argv.clone())?;
        if self.entry["mode"] == "direct" {
            let fun = Environment::get(
                &scope,
                self.entry["fix"]
                    .as_str()
                    .ok_or_else(|| bad("direct repair name"))?,
            )
            .ok_or_else(|| bad("direct repair function"))?;
            let fixed = engine
                .call(fun, vec![Value::bytes(bytes.to_vec())], HashMap::new())?
                .raw()?;
            engine.files.insert("repaired".into(), fixed);
        } else {
            let main =
                Environment::get(&scope, "main").ok_or_else(|| bad("save program lacks entry"))?;
            let args = match self.entry["mode"].as_str() {
                Some("sys-argv") => vec![],
                Some("arguments") => vec![Value::list(vec![
                    Value::Text("--fix".into()),
                    Value::Text(name.into()),
                    Value::Text("repaired".into()),
                ])],
                _ => vec![argv],
            };
            let result = engine.call(main, args, HashMap::new())?;
            if result.truth() {
                return Err(bad(format!("save repair returned {}", result.text())));
            }
        }
        let fixed = engine
            .files
            .get("repaired")
            .ok_or_else(|| bad("title provides no repair operation"))?;
        if fixed.len() != bytes.len() {
            return Err(bad("repair changes image size"));
        }
        patch_bytes(bytes, 0, fixed)
    }
    pub fn record<'a>(
        &self,
        bytes: &'a [u8],
        record: &str,
        start: usize,
        index: usize,
    ) -> Result<&'a [u8]> {
        let width = self
            .records
            .iter()
            .find(|(name, _)| name == record)
            .map(|(_, width)| *width)
            .ok_or_else(|| bad("unknown save record"))?;
        records::record(bytes, start, width, index)
    }
}
/// Normalize transport metadata; semantic keys, values, arrays and validity are exact.
pub fn normalize(value: &mut Json) {
    if let Json::Object(map) = value {
        map.remove("file");
    }
}

pub mod catalog;
mod detection;
pub use detection::ValidityEvidence;
