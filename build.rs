use std::env;
use std::path::PathBuf;
use std::process::Command;

const DUCKDB_TAG: &str = "v1.5.5";

fn main() {
    println!("cargo:rustc-check-cfg=cfg(duckjeu_client_context_bridge)");
    println!("cargo:rerun-if-changed=src/host/client_context.cpp");
    println!("cargo:rerun-if-env-changed=DUCKDB_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=DUCKDB_SOURCE_VERSION");

    let Some(source_dir) = env::var_os("DUCKDB_SOURCE_DIR").map(PathBuf::from) else {
        println!("cargo:warning=DUCKDB_SOURCE_DIR is unset; connection-scoped runtime support is disabled");
        return;
    };
    let include_dir = source_dir.join("src/include");
    let required_header = include_dir.join("duckdb/main/client_context_state.hpp");
    if !required_header.is_file() {
        panic!(
            "DUCKDB_SOURCE_DIR must point to DuckDB {DUCKDB_TAG} source (missing src/include/duckdb/main/client_context_state.hpp)"
        );
    }

    let source_version = Command::new("git")
        .args([
            "-C",
            source_dir.to_str().expect("valid DuckDB source path"),
            "describe",
            "--tags",
            "--exact-match",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .or_else(|| env::var("DUCKDB_SOURCE_VERSION").ok());
    if source_version.as_deref() != Some(DUCKDB_TAG) {
        panic!(
            "DuckJeu's connection bridge requires the exact DuckDB {DUCKDB_TAG} source tag; set DUCKDB_SOURCE_DIR to that checkout (or set DUCKDB_SOURCE_VERSION={DUCKDB_TAG} for a verified source archive)"
        );
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("src/host/client_context.cpp")
        .include(include_dir)
        .flag_if_supported("-std=c++17")
        .warnings(false);
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build.flag_if_supported("-mmacosx-version-min=11.0");
    }
    build.compile("duckjeu_client_context");
    println!("cargo:rustc-cfg=duckjeu_client_context_bridge");
}
