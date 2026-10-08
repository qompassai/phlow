//! Build script: compile the Mojo scoring kernels into a shared library.
//!
//! The kernels (`kernels/scoring.mojo`) are compiled with the pixi-pinned
//! MAX 26.5 / Mojo 1.0 toolchain declared in this crate's `pixi.toml` /
//! `pixi.lock`. The resulting `libphlow_scoring.so` is placed in OUT_DIR
//! and linked; rpaths are emitted for OUT_DIR and for the pixi env's lib
//! directory (the Mojo runtime libraries the .so depends on), so the
//! built binaries run without LD_LIBRARY_PATH — at the documented cost
//! that they only run on a machine holding this pixi env.
//!
//! Failure policy: if pixi or the installed env is missing, the build
//! panics with the remediation (`pixi install` in this crate directory).
//! A stale .so is never linked silently: the kernel source, manifest,
//! and lockfile are all declared rerun-if-changed inputs.
//!
//! Pixi resolution is deterministic, never bare-PATH: primo carries two
//! pixis (system 0.81.0 in /usr/bin, a stale 0.59.0 in ~/.pixi/bin and
//! ~/.local/bin), and cargo's build-script PATH can order them either
//! way. The 0.59 binary cannot read the v7 lockfile the 0.81 binary
//! wrote and tries to re-solve mid-build. Resolution order here:
//! $PHLOW_MOJO_PIXI, /usr/bin/pixi, ~/.pixi/bin/pixi, then PATH. The
//! run also passes --locked, so a build never re-solves the env.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default());
    let kernel = manifest_dir.join("kernels").join("scoring.mojo");
    let env_lib = manifest_dir
        .join(".pixi")
        .join("envs")
        .join("default")
        .join("lib");
    println!("cargo:rerun-if-changed={}", kernel.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("pixi.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("pixi.lock").display()
    );
    assert!(
        env_lib.is_dir(),
        "pixi env not installed for phlow-trainer-mojo: run `pixi install` in {} first",
        manifest_dir.display()
    );
    let lib_path = out_dir.join("libphlow_scoring.so");
    let status = Command::new(resolve_pixi())
        .args([
            "run",
            "--locked",
            "mojo",
            "build",
            "--emit",
            "shared-lib",
            kernel.to_str().unwrap_or_default(),
            "-o",
            lib_path.to_str().unwrap_or_default(),
        ])
        .current_dir(&manifest_dir)
        .status()
        .expect("failed to spawn `pixi run mojo build` (is pixi on PATH?)");
    assert!(
        status.success(),
        "mojo kernel compilation failed with status {status}"
    );
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=dylib=phlow_scoring");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", out_dir.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", env_lib.display());
}

/// Resolve the pixi binary deterministically (see the module docs for
/// why bare PATH lookup is a trap on primo). The returned path is used
/// for every pixi invocation in this build.
fn resolve_pixi() -> PathBuf {
    if let Some(explicit) = env::var_os("PHLOW_MOJO_PIXI") {
        return PathBuf::from(explicit);
    }
    let system = PathBuf::from("/usr/bin/pixi");
    if system.is_file() {
        return system;
    }
    if let Some(home) = env::var_os("HOME") {
        let self_installed = PathBuf::from(home).join(".pixi").join("bin").join("pixi");
        if self_installed.is_file() {
            return self_installed;
        }
    }
    PathBuf::from("pixi")
}
