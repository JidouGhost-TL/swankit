// SPDX-License-Identifier: MIT OR Apache-2.0
//! Discover data definitions without a second catalog of title identities.
use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=data");
    let mut paths: Vec<_> = fs::read_dir("data")
        .expect("definition directory")
        .map(|entry| entry.expect("definition path").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path.file_name().is_some_and(|name| name != "services.json")
        })
        .collect();
    paths.sort();
    let mut rust = String::from("const DEFINITIONS: &[&str] = &[\n");
    for path in paths {
        rust.push_str(&format!(
            "include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {:?})),\n",
            format!("/{}", path.display())
        ));
    }
    rust.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("build output")).join("definitions.rs"),
        rust,
    )
    .expect("embedded definitions");
}
