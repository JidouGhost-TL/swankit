//! Content candidates from the definition's program and excluded working regions.
use crate::{DecodeContext, Format};
use formatkit_core::{Error, Result};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ValidityEvidence {
    pub usable_flags: usize,
    pub checks_passed: usize,
    pub checks_failed: usize,
}
fn evidence(value: &Value, exclude: &HashSet<&str>, out: &mut ValidityEvidence) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if exclude.contains(key.as_str()) {
                    continue;
                }
                if let Some(flag) = value.as_bool() {
                    if matches!(key.as_str(), "valid" | "global_valid" | "signature_valid") && flag
                    {
                        out.usable_flags += 1;
                    }
                    if matches!(
                        key.as_str(),
                        "valid"
                            | "global_valid"
                            | "signature_valid"
                            | "checksum_ok"
                            | "checksum_valid"
                            | "crc_ok"
                            | "ok"
                    ) {
                        if flag {
                            out.checks_passed += 1;
                        } else {
                            out.checks_failed += 1;
                        }
                    }
                }
                evidence(value, exclude, out);
            }
        }
        Value::Array(values) => {
            for v in values {
                evidence(v, exclude, out);
            }
        }
        _ => {}
    }
}
impl Format {
    pub fn accepts_size(&self, length: usize) -> bool {
        self.definition()["media"].as_array().is_some_and(|media| {
            media.iter().any(|m| {
                m["sizes"]
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().any(|n| n.as_u64() == Some(length as u64)))
            })
        })
    }
    /// A match requires positive semantic evidence, not just a successful parse.
    /// Empty/invalid redundant records may coexist with a usable stored record.
    /// This is a candidate predicate, never a unique title identity guarantee.
    pub fn recognize_with_context(
        &self,
        bytes: &[u8],
        name: &str,
        context: &DecodeContext,
    ) -> Result<Option<ValidityEvidence>> {
        if !self.accepts_size(bytes.len()) {
            return Ok(None);
        }
        let policy = &self.definition()["validity"];
        if policy["policy"].as_str() != Some("program") {
            return Err(crate::value::bad("unsupported definition validity policy"));
        }
        let names = policy["exclude"]
            .as_array()
            .ok_or_else(|| crate::value::bad("validity exclusions must be an array"))?;
        if names.len() > crate::value::MAX_ITEMS {
            return Err(Error::ResourceLimit {
                resource: "validity exclusion names",
                requested: names.len() as u64,
                limit: crate::value::MAX_ITEMS as u64,
            });
        }
        let excludes: HashSet<&str> = names
            .iter()
            .map(|n| {
                n.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                    crate::value::bad("validity exclusions must be nonempty strings")
                })
            })
            .collect::<Result<_>>()?;
        let value = self.decode_with_context(bytes, name, context)?;
        if value.get("error").is_some_and(|e| !e.is_null())
            || ["valid", "global_valid"]
                .iter()
                .any(|key| value.get(key) == Some(&Value::Bool(false)))
        {
            return Ok(None);
        }
        let mut out = ValidityEvidence {
            usable_flags: 0,
            checks_passed: 0,
            checks_failed: 0,
        };
        evidence(&value, &excludes, &mut out);
        if out.usable_flags == 0 && out.checks_passed == 0 {
            Ok(None)
        } else {
            Ok(Some(out))
        }
    }
}
