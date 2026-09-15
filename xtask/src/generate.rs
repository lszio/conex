use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

use crate::schema;

const TS_PROTO_OPT: &str = "forceLong=string,outputEncodeMethods=false,outputJsonMethods=true,useOptionals=all,outputServices=none,esModuleInterop=true";

pub fn run(check: bool) -> Result<()> {
    let root = repo_root()?;
    let protos = collect_protos(&root)?;
    let real_schema = root.join("schema/generated");
    let real_ts = root.join("sdk/typescript/src/generated");

    if check {
        let tmp = tempfile::tempdir().context("create temp dir")?;
        let tmp_schema = tmp.path().join("schema");
        let tmp_ts = tmp.path().join("ts");
        generate(&root, &tmp_schema, &tmp_ts, &protos)?;
        compare_trees(&real_schema, &tmp_schema)?;
        compare_trees(&real_ts, &tmp_ts)?;
        println!("xtask generate --check: generated artifacts are up to date");
    } else {
        generate(&root, &real_schema, &real_ts, &protos)?;
        println!("generated schema/generated/ and sdk/typescript/src/generated/");
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.join("..").canonicalize()?)
}

fn collect_protos(root: &Path) -> Result<Vec<PathBuf>> {
    let mut protos = Vec::new();
    for dir in ["conformance/schema", "schema/conex/v1"] {
        let path = root.join(dir);
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.extension().map(|x| x == "proto").unwrap_or(false) {
                    protos.push(p);
                }
            }
        }
    }
    protos.sort();
    if protos.is_empty() {
        bail!("no .proto files found under conformance/schema or schema/conex/v1");
    }
    Ok(protos)
}

fn generate(root: &Path, schema_dir: &Path, ts_dir: &Path, protos: &[PathBuf]) -> Result<()> {
    let rust_dir = schema_dir.join("rust");
    fs::create_dir_all(&rust_dir)?;
    let descriptor = schema_dir.join("conex.bin");

    let rel: Vec<PathBuf> = protos
        .iter()
        .map(|p| p.strip_prefix(root).unwrap().to_path_buf())
        .collect();

    let include = [root.to_path_buf()];
    let mut config = prost_build::Config::new();
    config
        .out_dir(&rust_dir)
        .file_descriptor_set_path(&descriptor)
        .compile_well_known_types()
        .extern_path(".google.protobuf", "::pbjson_types")
        .bytes(["."]);
    config.compile_protos(&rel, &include)?;

    let bytes = fs::read(&descriptor)?;
    let packages = packages_from_descriptor(&bytes)?;
    let pkg_refs: Vec<&str> = packages.iter().map(String::as_str).collect();
    pbjson_build::Builder::new()
        .out_dir(&rust_dir)
        .register_descriptors(&bytes)?
        .build(&pkg_refs)?;

    schema::build(&bytes, &schema_dir.join("jsonschema"))?;
    generate_ts(root, ts_dir, &rel)?;
    Ok(())
}

fn packages_from_descriptor(bytes: &[u8]) -> Result<Vec<String>> {
    let pool = prost_reflect::DescriptorPool::decode(bytes)?;
    let mut packages: Vec<String> = pool
        .files()
        .map(|f| format!(".{}", f.package_name()))
        .filter(|p| p != ".")
        .collect();
    packages.sort();
    packages.dedup();
    Ok(packages)
}

fn generate_ts(root: &Path, out_dir: &Path, rel_protos: &[PathBuf]) -> Result<()> {
    let plugin = find_ts_proto_plugin(root)
        .context("locate protoc-gen-ts_proto; run bun install at the repo root")?;
    fs::create_dir_all(out_dir)?;
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());
    let mut cmd = Command::new(&protoc);
    cmd.arg(format!("--plugin=protoc-gen-ts_proto={}", plugin.display()));
    cmd.arg(format!("--ts_proto_out={}", out_dir.display()));
    cmd.arg(format!("--ts_proto_opt={TS_PROTO_OPT}"));
    cmd.arg(format!("-I{}", root.display()));
    for p in rel_protos {
        cmd.arg(p);
    }
    let output = cmd.output().with_context(|| format!("spawn {protoc}"))?;
    if !output.status.success() {
        bail!(
            "ts-proto generation failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn find_ts_proto_plugin(root: &Path) -> Option<PathBuf> {
    [
        root.join("node_modules/.bin/protoc-gen-ts_proto"),
        root.join("sdk/typescript/node_modules/.bin/protoc-gen-ts_proto"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

fn compare_trees(expected: &Path, actual: &Path) -> Result<()> {
    let expected_files = collect_files(expected)?;
    let actual_files = collect_files(actual)?;
    let mut problems = Vec::new();
    for (rel, path) in &expected_files {
        match actual_files.get(rel) {
            None => problems.push(format!("stale or missing after regeneration: {rel}")),
            Some(other) if fs::read(path)? != fs::read(other)? => {
                problems.push(format!("content differs: {rel}"))
            }
            _ => {}
        }
    }
    for rel in actual_files.keys() {
        if !expected_files.contains_key(rel) {
            problems.push(format!("generated but not committed: {rel}"));
        }
    }
    if !problems.is_empty() {
        bail!(
            "generate --check found {} problem(s) (run cargo xtask generate):\n  {}",
            problems.len(),
            problems.join("\n  ")
        );
    }
    Ok(())
}

fn collect_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(dir)?
                .to_string_lossy()
                .to_string();
            out.insert(rel, entry.path().to_path_buf());
        }
    }
    Ok(out)
}
