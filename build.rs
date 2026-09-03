use std::process::Command;

/// Stamps the build with a version string, replacing the Go build's
/// `-ldflags "-X main.version=..."`.
///
/// Precedence: an explicit `SWITCH_VERSION` (used by CI and the Makefile),
/// then `git describe --tags --always --dirty`, then the crate version.
fn main() {
    println!("cargo:rerun-if-env-changed=SWITCH_VERSION");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let version = std::env::var("SWITCH_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(git_describe)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    println!("cargo:rustc-env=SWITCH_VERSION={version}");
}

fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}
