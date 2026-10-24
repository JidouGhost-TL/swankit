use crate::{args, io, Args, Result, Toolkit, MAX_FILE};
use serde_json::{json, Value};
use swankit::{extract, saves};

pub fn run(mut a: Args, json_mode: bool) -> Result<()> {
    let action = a
        .positional
        .get(1)
        .cloned()
        .ok_or("save requires decode, set or fix")?;
    if !matches!(action.as_str(), "decode" | "set" | "fix") {
        return Err("unknown save action".into());
    }
    let selection = a
        .positional
        .get(2)
        .cloned()
        .ok_or("save requires a definition or auto")?;
    let source = a
        .positional
        .get(3)
        .cloned()
        .ok_or("save requires an input image")?;
    let bytes = io::read(&source, MAX_FILE)?;
    let name = a.value("--name").unwrap_or_else(|| source.clone());
    let context = if let Some(path) = a.value("--context") {
        swankit::save::context_from_json(&io::read(&path, extract::MAX_DEFINITION_BYTES)?)?
    } else {
        saves::DecodeContext::default()
    };
    let toolkit = Toolkit::new()?;
    let external;
    let definition = if selection == "auto" {
        toolkit.auto_save(&bytes, &name, &context)?
    } else if std::path::Path::new(&selection).is_file() {
        external = saves::Format::from_json(std::str::from_utf8(&io::read(
            &selection,
            extract::MAX_DEFINITION_BYTES,
        )?)?)?;
        &external
    } else {
        saves::format(&selection)?
    };
    match action.as_str() {
        "decode" => {
            a.finish(4)?;
            let value = toolkit.decode_save(definition, &bytes, &name, &context)?;
            let plain = serde_json::to_string_pretty(&value)?;
            crate::emit(&value, &plain, json_mode, None, false)
        }
        "set" | "fix" => {
            let output = a.require("--output")?;
            let force = a.flag("--force");
            let mut validation_skipped = false;
            let updated = if action == "set" {
                let index = args::number(&a.require("--field")?)?;
                let value: Value = serde_json::from_str(&a.require("--value")?)?;
                let all_copies = a.flag("--all-copies");
                let skip_validation = a.flag("--skip-validation");
                validation_skipped = skip_validation;
                let repair = !a.flag("--no-repair");
                a.finish(4)?;
                io::ensure_distinct_path(&output, &source)?;
                toolkit.set_save_with_options(
                    definition,
                    &bytes,
                    &name,
                    &context,
                    index,
                    &value,
                    swankit::SaveEditOptions {
                        all_copies,
                        skip_validation,
                        repair,
                    },
                )?
            } else {
                a.finish(4)?;
                io::ensure_distinct_path(&output, &source)?;
                toolkit.fix_save(definition, &bytes, &name, &context)?
            };
            let details = if validation_skipped {
                json!({"definition":definition.id,"validation":"skipped"})
            } else {
                json!({"definition":definition.id,"checks":definition.verify_with_context(&updated,&name,&context)?})
            };
            crate::artifact(&output, &updated, force, json_mode, details)
        }
        _ => Err("unknown save action".into()),
    }
}
