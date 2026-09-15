//! Derive JSON Schema documents from the compiled FileDescriptorSet.
//!
//! The schema is a *derived* artifact: it documents the structural contract and
//! feeds the TS boundary validator. The authoritative strict decoder for the
//! Rust side is MethodContract.prepare (design S5.2), not this file.
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use prost_reflect::{DescriptorPool, FieldDescriptor, Kind, MessageDescriptor};
use serde_json::{Map, Value, json};

pub fn build(descriptor_bytes: &[u8], out_dir: &Path) -> Result<()> {
    let pool = DescriptorPool::decode(descriptor_bytes).context("decode FileDescriptorSet")?;
    let messages: Vec<MessageDescriptor> = pool
        .all_messages()
        .filter(|m| !m.full_name().starts_with("google.protobuf."))
        .collect();
    let mut defs = Map::new();
    for m in &messages {
        defs.insert(m.full_name().to_string(), message_schema(m));
    }
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    for m in &messages {
        let name = m.full_name();
        let doc = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": format!("conex:{name}"),
            "$ref": format!("#/$defs/{name}"),
            "$defs": Value::Object(defs.clone()),
        });
        let path = out_dir.join(format!("{name}.schema.json"));
        fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&doc)?))
            .with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

fn message_schema(m: &MessageDescriptor) -> Value {
    let mut props = Map::new();
    let mut required: Vec<Value> = Vec::new();
    for field in m.fields() {
        let name = field.json_name().to_string();
        let schema = if field.is_map() {
            let entry = match field.kind() {
                Kind::Message(entry) => entry,
                _ => unreachable!("map field kind is the entry message"),
            };
            let value_field = entry.get_field(2).expect("map entry value field");
            json!({"type": "object", "additionalProperties": kind_schema(&value_field)})
        } else if field.is_list() {
            json!({"type": "array", "items": kind_schema(&field)})
        } else {
            kind_schema(&field)
        };
        if !field.is_map() && !field.is_list() && !field.supports_presence() {
            required.push(Value::String(name.clone()));
        }
        props.insert(name, schema);
    }

    let mut out = Map::new();
    out.insert("type".into(), json!("object"));
    out.insert("additionalProperties".into(), json!(false));
    out.insert("properties".into(), Value::Object(props));
    if !required.is_empty() {
        out.insert("required".into(), Value::Array(required));
    }

    let mut oneof_constraints = Vec::new();
    for oneof in m.oneofs() {
        if oneof.is_synthetic() {
            continue;
        }
        let names: Vec<String> = oneof.fields().map(|f| f.json_name().to_string()).collect();
        if names.is_empty() {
            continue;
        }
        let req = |n: &str| json!({ "required": [n] });
        let present_all: Vec<Value> = names.iter().map(|n| req(n)).collect();
        let mut options = vec![json!({ "not": { "anyOf": present_all } })];
        for (i, n) in names.iter().enumerate() {
            let others: Vec<Value> = names
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, o)| req(o))
                .collect();
            options.push(json!({ "required": [n], "not": { "anyOf": others } }));
        }
        oneof_constraints.push(json!({ "anyOf": options }));
    }
    if !oneof_constraints.is_empty() {
        out.insert("allOf".into(), Value::Array(oneof_constraints));
    }
    Value::Object(out)
}

fn kind_schema(field: &FieldDescriptor) -> Value {
    match field.kind() {
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => {
            json!({"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64})
        }
        Kind::Uint32 | Kind::Fixed32 => {
            json!({"type": "integer", "minimum": 0, "maximum": 4294967295u64})
        }
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => {
            json!({"type": "string", "pattern": "^-?[0-9]+$", "format": "int64"})
        }
        Kind::Uint64 | Kind::Fixed64 => {
            json!({"type": "string", "pattern": "^[0-9]+$", "format": "uint64"})
        }
        Kind::Float | Kind::Double => json!({"type": "number"}),
        Kind::String => json!({"type": "string"}),
        Kind::Bytes => json!({"type": "string", "format": "byte"}),
        Kind::Enum(e) => {
            let names: Vec<String> = e.values().map(|v| v.name().to_string()).collect();
            json!({"type": "string", "enum": names})
        }
        Kind::Message(msg) => match msg.full_name() {
            "google.protobuf.Value" => json!({}),
            "google.protobuf.Struct" => json!({"type": "object"}),
            "google.protobuf.ListValue" => json!({"type": "array"}),
            _ => json!({"$ref": format!("#/$defs/{}", msg.full_name())}),
        },
    }
}
