//! Typed save operations through the definition engine, with explicit context.
use crate::{saves, Toolkit};
use formatkit_core::{Error, Result};
use serde::Serialize;
use serde_json::Value;
#[derive(Debug, Clone, Serialize)]
pub struct DefinitionCandidate {
    pub id: String,
    pub evidence: saves::ValidityEvidence,
}
impl Toolkit {
    pub fn detect_with_save_context(
        &self,
        bytes: &[u8],
        extension: Option<&str>,
        name: &str,
        context: &saves::DecodeContext,
    ) -> Result<crate::Detection> {
        let mut detection = self.detect(bytes, extension);
        if detection.selected.as_deref() == Some(crate::save_image::catalog::SAVE_IMAGE.as_str()) {
            detection.save_definitions = self.save_candidates(bytes, name, context)?;
        }
        Ok(detection)
    }
    pub fn save_candidates(
        &self,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
    ) -> Result<Vec<DefinitionCandidate>> {
        let mut out = Vec::new();
        for f in saves::formats()? {
            if f.accepts_size(bytes.len()) {
                if let Ok(Some(evidence)) = f.recognize_with_context(bytes, name, context) {
                    out.push(DefinitionCandidate {
                        id: f.id.clone(),
                        evidence,
                    });
                }
            }
        }
        Ok(out)
    }
    pub fn auto_save(
        &self,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
    ) -> Result<&'static saves::Format> {
        let candidates = self.save_candidates(bytes, name, context)?;
        if candidates.len() != 1 {
            return Err(Error::Unsupported(format!(
                "save auto selection found {} candidates; choose an explicit definition{}",
                candidates.len(),
                if candidates.is_empty() {
                    String::new()
                } else {
                    format!(
                        ": {}",
                        candidates
                            .iter()
                            .map(|c| c.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            )));
        }
        saves::format(&candidates[0].id)
    }
    pub fn decode_save(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
    ) -> Result<Value> {
        definition.decode_with_context(bytes, name, context)
    }
    pub fn set_save(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
        field: usize,
        value: &Value,
    ) -> Result<Vec<u8>> {
        if !definition.accepts_size(bytes.len()) {
            return Err(Error::Malformed(
                "image length is not declared by the save definition".into(),
            ));
        }
        let mut candidate = bytes.to_vec();
        definition.set_field(&mut candidate, field, value)?;
        definition.fix_with_context(&mut candidate, name, context)?;
        if definition
            .recognize_with_context(&candidate, name, context)?
            .is_none()
        {
            return Err(Error::Malformed(
                "save has no usable record after repair".into(),
            ));
        }
        Ok(candidate)
    }
    pub fn fix_save(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
    ) -> Result<Vec<u8>> {
        if !definition.accepts_size(bytes.len()) {
            return Err(Error::Malformed(
                "image length is not declared by the save definition".into(),
            ));
        }
        let mut candidate = bytes.to_vec();
        definition.fix_with_context(&mut candidate, name, context)?;
        if definition
            .recognize_with_context(&candidate, name, context)?
            .is_none()
        {
            return Err(Error::Malformed(
                "save has no usable record after repair".into(),
            ));
        }
        Ok(candidate)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextDefinition {
    schema: String,
    #[serde(default)]
    tables: std::collections::HashMap<String, Vec<u8>>,
    #[serde(default)]
    images: std::collections::HashMap<String, Vec<u8>>,
}
pub fn context_from_json(bytes: &[u8]) -> Result<saves::DecodeContext> {
    let value: ContextDefinition = crate::extract::definition(bytes)?;
    if value.schema != "swankit-save-context/1" {
        return Err(Error::Malformed("unknown save context schema".into()));
    }
    Ok(saves::DecodeContext {
        tables: value.tables,
        images: value.images,
    })
}
