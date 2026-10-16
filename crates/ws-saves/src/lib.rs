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
    entry: Json,
    program: Json,
    definition: Json,
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
        if definition["schema"] != "swankit-save-def/1" {
            return Err(bad("unsupported save definition schema"));
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
        let fields = array(&definition["fields"])?
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
            entry: definition["entry"].clone(),
            program: definition["program"].clone(),
            definition,
        })
    }
    /// Portable source definition, including symbolic semantics and policy.
    pub fn definition(&self) -> &Json {
        &self.definition
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
