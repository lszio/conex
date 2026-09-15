use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "proto").unwrap_or(false))
            .collect();
        paths.sort();
        out.extend(paths);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest.join("..").join("..").canonicalize()?;

    let mut absolute = Vec::new();
    collect_protos(&repo_root.join("conformance/schema"), &mut absolute);
    collect_protos(&repo_root.join("schema/conex/v1"), &mut absolute);
    if absolute.is_empty() {
        panic!("no .proto files found under conformance/schema or schema/conex/v1");
    }
    // Use repo-relative paths so generated descriptors are machine independent.
    let protos: Vec<PathBuf> = absolute
        .iter()
        .map(|p| p.strip_prefix(&repo_root).unwrap().to_path_buf())
        .collect();

    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join("conformance/schema").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join("schema/conex/v1").display()
    );

    let v1_root = repo_root.join("schema/conex/v1");
    let has_v1 = absolute.iter().any(|p| p.starts_with(&v1_root));
    println!("cargo:rustc-check-cfg=cfg(has_conex_v1)");
    if has_v1 {
        println!("cargo:rustc-cfg=has_conex_v1");
    }

    let out = PathBuf::from(env::var("OUT_DIR")?);
    let descriptor = out.join("conex.bin");

    let mut config = prost_build::Config::new();
    config
        .file_descriptor_set_path(&descriptor)
        .compile_well_known_types()
        .extern_path(".google.protobuf", "::pbjson_types")
        .bytes(["."]);
    config.compile_protos(&protos, std::slice::from_ref(&repo_root))?;

    let bytes = fs::read(&descriptor)?;
    let mut packages = vec![".conex.test.v1"];
    if has_v1 {
        packages.push(".conex.v1");
    }
    pbjson_build::Builder::new()
        .register_descriptors(&bytes)?
        .build(&packages)?;
    Ok(())
}
