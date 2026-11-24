//! Typed save operations through the definition engine, with explicit context.
use crate::{saves, Toolkit};
use formatkit_core::{Error, Result};
use serde::Serialize;
use serde_json::Value;
#[derive(Debug, Clone, Copy)]
pub struct SaveEditOptions {
    pub all_copies: bool,
    pub skip_validation: bool,
    pub repair: bool,
}
impl SaveEditOptions {
    pub const DEFAULT: Self = Self {
        all_copies: false,
        skip_validation: false,
        repair: true,
    };
}
impl Default for SaveEditOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}
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
            if f.accepts_primary_size(bytes.len()) {
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
        self.set_save_with_options(
            definition,
            bytes,
            name,
            context,
            field,
            value,
            SaveEditOptions::DEFAULT,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn set_save_with_options(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
        field: usize,
        value: &Value,
        options: SaveEditOptions,
    ) -> Result<Vec<u8>> {
        self.edit_save(
            definition, bytes, name, context, field, value, options, true,
        )
    }
    /// Legacy-style un-repaired edit, retaining storage, input-usability and
    /// field-policy checks but not the post-edit usability check.
    #[allow(clippy::too_many_arguments)]
    pub fn set_save_unrepaired(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
        field: usize,
        value: &Value,
        options: SaveEditOptions,
    ) -> Result<Vec<u8>> {
        self.edit_save(
            definition,
            bytes,
            name,
            context,
            field,
            value,
            SaveEditOptions {
                repair: false,
                ..options
            },
            false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn edit_save(
        &self,
        definition: &saves::Format,
        bytes: &[u8],
        name: &str,
        context: &saves::DecodeContext,
        field: usize,
        value: &Value,
        options: SaveEditOptions,
        check_usable: bool,
    ) -> Result<Vec<u8>> {
        if !definition.accepts_size(bytes.len()) {
            return Err(Error::Malformed(
                "image length is not declared by the save definition".into(),
            ));
        }
        require_usable_input(definition, bytes, name, context)?;
        let fields: Vec<usize> = if options.all_copies {
            definition
                .copy_group_for_field(field)
                .ok_or_else(|| {
                    Error::Malformed("selected field has no declared copy group".into())
                })?
                .fields
                .clone()
        } else {
            vec![field]
        };
        if !options.skip_validation {
            for index in &fields {
                definition.validate_field_value(*index, value)?;
            }
        }
        let mut candidate = bytes.to_vec();
        for index in fields {
            definition.set_field_with_options(
                &mut candidate,
                index,
                value,
                options.skip_validation,
            )?;
        }
        if options.repair {
            definition.fix_with_context(&mut candidate, name, context)?;
        }
        if check_usable
            && !options.skip_validation
            && definition
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

/// Refuse an edit unless the unmodified input already holds a record the
/// selected definition's own validity program accepts. This is the same
/// predicate as candidate recognition and the post-edit usability check; it
/// adds no heuristics. It runs before any field write or repair, so a
/// same-size image of another medium cannot be rewritten into a plausible
/// record by checksum repair.
fn require_usable_input(
    definition: &saves::Format,
    bytes: &[u8],
    name: &str,
    context: &saves::DecodeContext,
) -> Result<()> {
    if definition
        .recognize_with_context(bytes, name, context)?
        .is_some()
    {
        return Ok(());
    }
    let reason = if !definition.accepts_primary_size(bytes.len()) {
        "its length is declared only for a companion medium, not as a primary save image"
            .to_string()
    } else {
        let tree = definition.decode_with_context(bytes, name, context)?;
        match tree.get("error") {
            Some(Value::String(message)) => format!("the definition reports: {message}"),
            Some(e) if !e.is_null() => format!("the definition reports: {e}"),
            _ if ["valid", "global_valid"]
                .iter()
                .any(|key| tree.get(*key) == Some(&Value::Bool(false))) =>
            {
                "the definition's validity check marks the image invalid".to_string()
            }
            _ => "no record passes the definition's validity checks".to_string(),
        }
    };
    Err(Error::Malformed(format!(
        "input save has no usable record under definition {}: {reason}; refusing to edit \
         (check that the image is this game's save for this medium)",
        definition.id
    )))
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
