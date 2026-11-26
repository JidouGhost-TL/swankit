use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let workspace_toml = Path::new(&manifest_dir).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", workspace_toml.display());

    let formatkit_version =
        extract_formatkit_version(&workspace_toml).unwrap_or_else(|| "0.1.14".to_string());
    println!("cargo:rustc-env=SWANKIT_FORMATKIT_VERSION={formatkit_version}");

    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SWANKIT_TARGET={target}");
    println!("cargo:rustc-env=SWANKIT_PROFILE={profile}");

    if let Some(head_path) = git_output(&manifest_dir, &["rev-parse", "--git-path", "HEAD"]) {
        let p = Path::new(&manifest_dir).join(&head_path);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    if let Some(symref) = git_output(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(ref_path) = git_output(&manifest_dir, &["rev-parse", "--git-path", &symref]) {
            let p = Path::new(&manifest_dir).join(&ref_path);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
    if let Some(index_path) = git_output(&manifest_dir, &["rev-parse", "--git-path", "index"]) {
        let p = Path::new(&manifest_dir).join(&index_path);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }

    let git_commit =
        git_output(&manifest_dir, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
    let git_dirty = git_output(
        &manifest_dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .is_some_and(|s| !s.is_empty());

    println!("cargo:rustc-env=SWANKIT_GIT_COMMIT={git_commit}");
    println!("cargo:rustc-env=SWANKIT_GIT_DIRTY={git_dirty}");
}

fn git_output(dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_formatkit_version(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("formatkit-core") {
            let after_eq = rest.trim_start().strip_prefix('=')?;
            let quoted = after_eq.trim().trim_matches('"');
            let version = quoted.trim_start_matches(['=', '^', '~']);
            if !version.is_empty() {
                return Some(version.to_string());
            }
        }
    }
    None
}
