//! Executable identity pins: canonical path plus SHA-256.
//!
//! A check's program is resolved the way its spawn would resolve it —
//! relative to the workspace root when it contains `/`, otherwise through
//! `PATH` — then canonicalized and hashed once at admission. Before every
//! spawn [`crate::CheckRunner`] repeats the resolution; any difference in
//! canonical path or digest is a hard refusal, so a rewritten, replaced,
//! relinked, PATH-shadowed or removed executable never runs.
//!
//! Residual race: the file is hashed, then executed by path. A writer who
//! can replace the pinned file between those two steps is not stopped;
//! closing that window needs `fexecve`, which this safe-only crate cannot
//! call. Only the named executable is pinned — a `#!` interpreter and
//! shared libraries are not.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use phlow_config::CheckConfig;
use sha2::{Digest, Sha256};

/// Largest executable that can be pinned, in bytes. Hashing streams, so
/// this bounds hashing time rather than memory.
pub const PIN_BYTES_MAX: u64 = 1 << 30;
/// Chunk size for streaming an executable through SHA-256, in bytes.
const PIN_READ_CHUNK_BYTES: usize = 64 * 1024;
/// Most `PATH` entries consulted when resolving a bare command name.
const PATH_ENTRIES_MAX: usize = 256;

/// Identity of one executable, taken at admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPin {
    /// Absolute, symlink-free path of the executable.
    pub canonical_path: PathBuf,
    /// Lowercase hex SHA-256 of the executable's bytes.
    pub sha256: String,
}

/// Admission result per check name: its pin, or why it could not be pinned.
pub type CheckPins = BTreeMap<String, Result<BinaryPin, String>>;

/// Pin every check's program as seen from `root`.
pub fn pin_checks(root: &Path, checks: &BTreeMap<String, CheckConfig>) -> CheckPins {
    checks
        .iter()
        .map(|(name, check)| {
            let pin = match check.cmd().first() {
                Some(program) => pin_executable(root, program),
                None => Err("check argv is empty".to_owned()),
            };
            (name.clone(), pin)
        })
        .collect()
}

/// Resolve, canonicalize and hash `program` as a spawn from `root` finds it.
pub fn pin_executable(root: &Path, program: &str) -> Result<BinaryPin, String> {
    let resolved = resolve(root, program)?;
    let canonical_path = std::fs::canonicalize(&resolved)
        .map_err(|error| format!("{}: {error}", resolved.display()))?;
    let sha256 = sha256_file(&canonical_path)?;
    Ok(BinaryPin {
        canonical_path,
        sha256,
    })
}

/// Mirror exec's lookup: a name containing `/` is relative to the child's
/// cwd (`root`); a bare name takes the first executable `PATH` match, with
/// empty and relative entries also relative to `root`.
fn resolve(root: &Path, program: &str) -> Result<PathBuf, String> {
    if program.contains('/') {
        // `join` of an absolute program replaces `root` entirely.
        return Ok(root.join(program));
    }
    let path = std::env::var_os("PATH").ok_or_else(|| format!("{program}: PATH is unset"))?;
    for directory in std::env::split_paths(&path).take(PATH_ENTRIES_MAX) {
        let candidate = root.join(directory).join(program);
        if is_executable_file(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!("{program}: not found on PATH"))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Stream `path` through SHA-256, refusing files over [`PIN_BYTES_MAX`].
fn sha256_file(path: &Path) -> Result<String, String> {
    let describe = |error: std::io::Error| format!("{}: {error}", path.display());
    let file = std::fs::File::open(path).map_err(describe)?;
    // Read one byte past the cap so an oversized (or growing) file is seen.
    let mut limited = file.take(PIN_BYTES_MAX + 1);
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; PIN_READ_CHUNK_BYTES];
    let mut total_bytes: u64 = 0;
    loop {
        let read = limited.read(&mut chunk).map_err(describe)?;
        if read == 0 {
            break;
        }
        total_bytes += read as u64;
        hasher.update(&chunk[..read]);
    }
    if total_bytes > PIN_BYTES_MAX {
        return Err(format!(
            "{}: executable exceeds {PIN_BYTES_MAX} bytes",
            path.display()
        ));
    }
    Ok(format!("{:x}", hasher.finalize()))
}
