//! Minimal JSON Schema (2020-12 subset) validator.
//!
//! This exists so the Rust and TypeScript boundaries can consume the *same*
//! descriptor-derived schema document. It supports exactly the keywords our
//! emitter produces: type, properties, required, additionalProperties, items,
//! enum, pattern, format (uint64/int64), minimum/maximum, $ref, anyOf, oneOf,
//! not, allOf. It is intentionally not a general-purpose JSON Schema engine.
//!
//! The nested `if let` form is kept deliberately: each clause is an independent
//! schema keyword, and collapsing them obscures which keyword rejected a value.
#![allow(clippy::collapsible_if)]

use serde_json::Value;

pub fn validate(schema: &Value, instance: &Value) -> Result<(), String> {
    let defs = schema.get("$defs").cloned().unwrap_or(Value::Null);
    let mut errors = Vec::new();
    validate_node(schema, instance, &defs, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn validate_silent(schema: &Value, instance: &Value, defs: &Value) -> bool {
    let mut errors = Vec::new();
    validate_node(schema, instance, defs, "$", &mut errors);
    errors.is_empty()
}

fn validate_node(
    schema: &Value,
    instance: &Value,
    defs: &Value,
    path: &str,
    errors: &mut Vec<String>,
) -> bool {
    match schema {
        Value::Bool(true) => return true,
        Value::Bool(false) => {
            errors.push(format!("{path}: schema is false"));
            return false;
        }
        _ => {}
    }
    let obj = match schema.as_object() {
        Some(o) => o,
        None => return true,
    };

    if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
        return match resolve_ref(r, defs) {
            Some(target) => validate_node(&target, instance, defs, path, errors),
            None => {
                errors.push(format!("{path}: unresolved $ref {r}"));
                false
            }
        };
    }

    let mut ok = true;
    if let Some(t) = obj.get("type") {
        if !type_matches(t, instance) {
            errors.push(format!("{path}: expected {t}, got {}", json_type(instance)));
            ok = false;
        }
    }
    if let Some(variants) = obj.get("enum").and_then(Value::as_array) {
        if !variants.iter().any(|v| v == instance) {
            errors.push(format!("{path}: value not among enum"));
            ok = false;
        }
    }
    if let Some(s) = instance.as_str() {
        if let Some(pattern) = obj.get("pattern").and_then(Value::as_str) {
            if !simple_pattern(pattern, s) {
                errors.push(format!("{path}: does not match pattern {pattern}"));
                ok = false;
            }
        }
        if let Some(format) = obj.get("format").and_then(Value::as_str) {
            if !format_ok(format, s) {
                errors.push(format!("{path}: invalid {format}"));
                ok = false;
            }
        }
    }
    if let Some(n) = instance.as_i64() {
        if let Some(min) = obj.get("minimum").and_then(Value::as_i64) {
            if n < min {
                errors.push(format!("{path}: below minimum"));
                ok = false;
            }
        }
        if let Some(max) = obj.get("maximum").and_then(Value::as_i64) {
            if n > max {
                errors.push(format!("{path}: above maximum"));
                ok = false;
            }
        }
    }
    if let Some(map) = instance.as_object() {
        if let Some(required) = obj.get("required").and_then(Value::as_array) {
            for r in required {
                if let Some(name) = r.as_str() {
                    if !map.contains_key(name) {
                        errors.push(format!("{path}: missing required property {name}"));
                        ok = false;
                    }
                }
            }
        }
        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            for (k, v) in map {
                if let Some(sub) = props.get(k) {
                    if !validate_node(sub, v, defs, &format!("{path}.{k}"), errors) {
                        ok = false;
                    }
                } else if let Some(ap) = obj.get("additionalProperties") {
                    match ap {
                        Value::Bool(false) => {
                            errors.push(format!("{path}: unknown property {k}"));
                            ok = false;
                        }
                        Value::Bool(true) => {}
                        sub => {
                            if !validate_node(sub, v, defs, &format!("{path}.{k}"), errors) {
                                ok = false;
                            }
                        }
                    }
                }
            }
        }
    } else if let Some(arr) = instance.as_array() {
        if let Some(items) = obj.get("items") {
            for (i, v) in arr.iter().enumerate() {
                if !validate_node(items, v, defs, &format!("{path}[{i}]"), errors) {
                    ok = false;
                }
            }
        }
    }
    if let Some(any) = obj.get("anyOf").and_then(Value::as_array) {
        if !any.iter().any(|s| validate_silent(s, instance, defs)) {
            errors.push(format!("{path}: no anyOf branch matched"));
            ok = false;
        }
    }
    if let Some(one) = obj.get("oneOf").and_then(Value::as_array) {
        let matched = one
            .iter()
            .filter(|s| validate_silent(s, instance, defs))
            .count();
        if matched != 1 {
            errors.push(format!("{path}: oneOf matched {matched} branches"));
            ok = false;
        }
    }
    if let Some(not) = obj.get("not") {
        if validate_silent(not, instance, defs) {
            errors.push(format!("{path}: matched not schema"));
            ok = false;
        }
    }
    if let Some(all) = obj.get("allOf").and_then(Value::as_array) {
        for s in all {
            if !validate_node(s, instance, defs, path, errors) {
                ok = false;
            }
        }
    }
    ok
}

fn resolve_ref(reference: &str, defs: &Value) -> Option<Value> {
    let name = reference.strip_prefix("#/$defs/")?;
    defs.get(name).cloned()
}

fn type_matches(t: &Value, v: &Value) -> bool {
    if let Some(s) = t.as_str() {
        match s {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => true,
        }
    } else if let Some(arr) = t.as_array() {
        arr.iter().any(|x| type_matches(x, v))
    } else {
        true
    }
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn simple_pattern(pattern: &str, s: &str) -> bool {
    match pattern {
        "^[0-9]+$" => !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        "^-?[0-9]+$" => {
            let digits = s.strip_prefix('-').unwrap_or(s);
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }
        _ => true,
    }
}

fn format_ok(format: &str, s: &str) -> bool {
    match format {
        "uint64" => s.parse::<u64>().is_ok(),
        "int64" => s.parse::<i64>().is_ok(),
        _ => true,
    }
}
