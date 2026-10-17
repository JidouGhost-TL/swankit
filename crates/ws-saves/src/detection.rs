//! Content candidates from the definition's program and excluded working regions.
use crate::{DecodeContext, Format};
use formatkit_core::{Error, Result};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ValidityEvidence {
    /// Positive semantic flags outside explicitly invalid or excluded regions.
    pub usable_flags: usize,
    /// All passing checks in included regions, including unusable records.
    pub checks_passed: usize,
    /// All failing checks in included regions, including unusable records.
    pub checks_failed: usize,
}
#[derive(Default)]
struct ExplicitDecisions {
    present: bool,
    usable: bool,
}
// Return whether any included semantic validity flag was present, even if false.
// An invalid region cannot contribute usable flags from itself or its children;
// a separate valid record can still establish a usable image.
fn evidence(
    value: &Value,
    exclude: &HashSet<&str>,
    eligible: bool,
    out: &mut ValidityEvidence,
    decisions: &mut ExplicitDecisions,
) -> bool {
    let mut semantic_flags = false;
    match value {
        Value::Object(map) => {
            let eligible = eligible
                && !["valid", "global_valid"].iter().any(|key| {
                    !exclude.contains(key) && map.get(*key) == Some(&Value::Bool(false))
                });
            for (key, value) in map {
                if exclude.contains(key.as_str()) {
                    continue;
                }
                if let Some(flag) = value.as_bool() {
                    if matches!(key.as_str(), "valid" | "global_valid") {
                        decisions.present = true;
                        decisions.usable |= flag && eligible;
                    }
                    if matches!(key.as_str(), "valid" | "global_valid" | "signature_valid") {
                        semantic_flags = true;
                        if flag && eligible {
                            out.usable_flags += 1;
                        }
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
                semantic_flags |= evidence(value, exclude, eligible, out, decisions);
            }
        }
        Value::Array(values) => {
            for v in values {
                semantic_flags |= evidence(v, exclude, eligible, out, decisions);
            }
        }
        _ => {}
    }
    semantic_flags
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
    /// Cartridge images are primary when present. Internal EEPROM is a
    /// companion in those definitions; standalone console definitions may
    /// recognize it directly. Physical size declarations remain available via
    /// accepts_size for explicit field operations.
    pub fn accepts_primary_size(&self, length: usize) -> bool {
        let Some(media) = self.definition()["media"].as_array() else {
            return false;
        };
        let cartridge = media.iter().any(|m| {
            matches!(
                m["kind"].as_str(),
                Some("cartridge-sram" | "cartridge-eeprom")
            )
        });
        media
            .iter()
            .filter(|m| !cartridge || m["kind"] != "internal-eeprom")
            .any(|m| {
                m["sizes"]
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().any(|n| n.as_u64() == Some(length as u64)))
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
        if !self.accepts_primary_size(bytes.len()) {
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
        let mut decisions = ExplicitDecisions::default();
        let semantic_flags = evidence(&value, &excludes, true, &mut out, &mut decisions);
        // Included explicit program decisions take precedence over signatures
        // anywhere in the tree. Keep diagnostic counts independent of admission.
        let usable = if decisions.present {
            decisions.usable
        } else {
            // Signature/checksum-only programs retain their existing fallback.
            out.usable_flags > 0 || (!semantic_flags && out.checks_passed > 0)
        };
        Ok(usable.then_some(out))
    }
}
