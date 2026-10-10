//! Build script: compile the Mojo kernels and vendor their runtime.
//!
//! The kernels (`kernels/scoring.mojo`) are compiled with the pixi-pinned
//! MAX 26.5 / Mojo 1.0 toolchain declared in this crate's `pixi.toml` /
//! `pixi.lock`. The resulting `libphlow_scoring.so` is placed in OUT_DIR
//! and linked.
//!
//! Runtime vendoring (productization): the kernel library's only
//! non-system dependencies are the six Mojo runtime libraries in
//! [`VENDOR_LIBS`], and those libraries already carry `$ORIGIN` in
//! their own RPATHs. The build therefore copies the six into OUT_DIR
//! next to the kernel library and rewrites the kernel library's RPATH
//! to `$ORIGIN` alone with `patchelf`. Built binaries then need no
//! pixi env at run time — pixi is a build-time toolchain only. The
//! binary additionally gets an `$ORIGIN/../lib` rpath so the
//! installed layout produced by `scripts/install-runtime.sh`
//! (`bin/` + `lib/`) resolves the same way.
//!
//! Failure policy: if pixi, the installed env, a vendored library, or
//! patchelf is missing, the build panics with the remediation. A stale
//! .so is never linked silently: the kernel source, manifest, and
//! lockfile are all declared rerun-if-changed inputs.
//!
//! Pixi resolution is deterministic, never bare-PATH: primo carries two
//! pixis (system 0.81.0 in /usr/bin, a stale 0.59.0 in ~/.pixi/bin and
//! ~/.local/bin), and cargo's build-script PATH can order them either
//! way. The 0.59 binary cannot read the v7 lockfile the 0.81 binary
//! wrote and tries to re-solve mid-build. Resolution order here:
//! $PHLOW_MOJO_PIXI, /usr/bin/pixi, ~/.pixi/bin/pixi, then PATH. The
//! run also passes --locked, so a build never re-solves the env.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Mojo runtime libraries `libphlow_scoring.so` links against
/// (its full non-system DT_NEEDED set, verified with `ldd` against
/// the MAX 26.5 env the lockfile pins). If a MAX upgrade changes this
/// set, the build fails on a missing entry rather than shipping a
/// library that cannot load.
const VENDOR_LIBS: [&str; 6] = [
    "libKGENCompilerRTShared.so",
    "libAsyncRTMojoBindings.so",
    "libMSupportGlobals.so",
    "libAsyncRTRuntimeGlobals.so",
    "libstdc++.so.6",
    "libgcc_s.so.1",
];

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
    println!("cargo:rerun-if-changed={}", file!());
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
    vendor_runtime(&env_lib, &out_dir, &lib_path);
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=dylib=phlow_scoring");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", out_dir.display());
    // Installed layout (scripts/install-runtime.sh): binary in bin/,
    // kernel + vendored libraries together in lib/.
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
}

/// Copy the Mojo runtime libraries next to the kernel library and
/// point the kernel library's RPATH at `$ORIGIN` only, so the built
/// artifacts resolve every Mojo dependency from their own directory.
fn vendor_runtime(env_lib: &Path, out_dir: &Path, lib_path: &Path) {
    for name in VENDOR_LIBS {
        let source = env_lib.join(name);
        assert!(
            source.is_file(),
            "vendored runtime library {name} missing from the pixi env at {} \
             (a MAX upgrade may have changed the runtime set — update VENDOR_LIBS)",
            source.display()
        );
        fs::copy(&source, out_dir.join(name))
            .unwrap_or_else(|error| panic!("failed to vendor {name}: {error}"));
    }
    let patchelf = resolve_patchelf();
    let status = Command::new(&patchelf)
        .args([
            "--set-rpath",
            "$ORIGIN",
            lib_path.to_str().unwrap_or_default(),
        ])
        .status()
        .expect("failed to spawn patchelf (install patchelf to build this crate)");
    assert!(
        status.success(),
        "patchelf --set-rpath failed with status {status}"
    );
}

/// Resolve patchelf deterministically: $PHLOW_MOJO_PATCHELF, the
/// system path, then PATH.
fn resolve_patchelf() -> PathBuf {
    if let Some(explicit) = env::var_os("PHLOW_MOJO_PATCHELF") {
        return PathBuf::from(explicit);
    }
    let system = PathBuf::from("/usr/bin/patchelf");
    if system.is_file() {
        return system;
    }
    PathBuf::from("patchelf")
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
