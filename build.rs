//! Commit-based versioning. The runtime version string is
//! `git describe --tags --always --dirty` for the checkout this binary is
//! built from; Cargo.toml carries a placeholder so crates can still
//! parameterize package metadata. Fallbacks, in order:
//!   MIMAE_VERSION env var (release builds), "unknown".

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-env-changed=MIMAE_VERSION");
    if let Some(forced) = std::env::var_os("MIMAE_VERSION") {
        println!("cargo:rustc-env=MIMAE_VERSION={}", forced.to_string_lossy());
        return;
    }
    let described = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=MIMAE_VERSION={described}");
}
