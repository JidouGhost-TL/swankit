// SPDX-License-Identifier: MIT OR Apache-2.0
//! Validator for the closed JSON Schema vocabulary used by definition v1.
//! The schema is repository-owned; this is not a general JSON Schema service.
use crate::malformed;
use formatkit_core::Result;
use serde_json::Value;
use std::sync::OnceLock;
pub const SCHEMA: &str = include_str!("../../../docs/wondergate-title-definition.schema.json");

pub fn validate(value: &Value) -> Result<()> {
    static ROOT: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
    let root = ROOT.get_or_init(|| serde_json::from_str(SCHEMA).map_err(|e| e.to_string()));
    let root = root.as_ref().map_err(|e| malformed(e))?;
    check(value, root, root, "$", 0)
}
fn check(value: &Value, schema: &Value, root: &Value, path: &str, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(malformed("schema nesting limit"));
    }
    let fail = |why: &str| malformed(&format!("{path}: {why}"));
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = reference
            .strip_prefix('#')
            .and_then(|p| root.pointer(p))
            .ok_or_else(|| fail("invalid schema reference"))?;
        return check(value, target, root, path, depth + 1);
    }
    if let Some(choices) = schema.get("oneOf").and_then(Value::as_array) {
        if choices
            .iter()
            .filter(|s| check(value, s, root, path, depth + 1).is_ok())
            .count()
            != 1
        {
            return Err(fail("expected exactly one schema alternative"));
        }
    }
    if schema.get("const").is_some_and(|c| c != value) {
        return Err(fail("unexpected constant"));
    }
    if schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|choices| !choices.contains(value))
    {
        return Err(fail("unknown enumeration value"));
    }
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let valid = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(fail("wrong JSON type"));
        }
    }
    if let Some(object) = value.as_object() {
        let props = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(fail(&format!("missing {key}")));
                }
            }
        }
        size(object.len(), schema, "minProperties", "maxProperties", path)?;
        for (key, member) in object {
            if let Some(rule) = props.and_then(|p| p.get(key)) {
                check(member, rule, root, &format!("{path}/{key}"), depth + 1)?;
            } else if let Some(extra) = schema.get("additionalProperties") {
                if extra == &Value::Bool(false) {
                    return Err(fail(&format!("unknown property {key}")));
                }
                if extra.is_object() {
                    check(member, extra, root, &format!("{path}/{key}"), depth + 1)?;
                }
            }
        }
    }
    if let Some(array) = value.as_array() {
        size(array.len(), schema, "minItems", "maxItems", path)?;
        if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
            for (i, member) in array.iter().enumerate() {
                if array[..i].contains(member) {
                    return Err(fail("duplicate array entry"));
                }
            }
        }
        if let Some(items) = schema.get("items") {
            for (i, member) in array.iter().enumerate() {
                check(member, items, root, &format!("{path}/{i}"), depth + 1)?;
            }
        }
    }
    if let Some(string) = value.as_str() {
        size(
            string.chars().count(),
            schema,
            "minLength",
            "maxLength",
            path,
        )?;
    }
    if value.is_number() {
        let number = value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from))
            .ok_or_else(|| fail("integer required"))?;
        for (keyword, below) in [("minimum", true), ("maximum", false)] {
            if let Some(bound) = schema.get(keyword) {
                let bound = bound
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| bound.as_u64().map(i128::from))
                    .ok_or_else(|| fail("invalid numeric schema"))?;
                if (below && number < bound) || (!below && number > bound) {
                    return Err(fail("number outside schema bounds"));
                }
            }
        }
    }
    Ok(())
}
fn size(n: usize, schema: &Value, min: &str, max: &str, path: &str) -> Result<()> {
    if schema
        .get(min)
        .and_then(Value::as_u64)
        .is_some_and(|v| (n as u64) < v)
        || schema
            .get(max)
            .and_then(Value::as_u64)
            .is_some_and(|v| (n as u64) > v)
    {
        return Err(malformed(&format!("{path}: size outside schema bounds")));
    }
    Ok(())
}
