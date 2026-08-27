use std::env;
use std::process::Command;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let revision = git(&manifest, &["rev-parse", "--short=12", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = git(
        &manifest,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .map(|status| !status.is_empty())
    .unwrap_or(false);
    let suffix = if dirty { ".dirty" } else { "" };
    println!("cargo:rustc-env=AGS_PRODUCT_VERSION=v{version}");
    println!("cargo:rustc-env=AGS_BUILD_ID={revision}{suffix}");
    println!("cargo:rustc-env=AGS_BUILD_DISPLAY=v{version} (build {revision}{suffix})");

    if let Some(git_dir) = git(&manifest, &["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        if let Some(reference) = git(&manifest, &["symbolic-ref", "HEAD"]) {
            println!("cargo:rerun-if-changed={git_dir}/{reference}");
        }
    }
}

fn git(root: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
