// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Skill sync: a canonical skill dir is synced into a target skill dir
//! through a scan → plan → apply pipeline with backups.
//!
//! Adapted concept: Ghostex `packages/agent-sync` (scan → plan → apply;
//! traversal refusal; symlink refusal; backup-before-write; atomic
//! apply). This is an independent Tiger Style re-implementation, not a
//! port of Ghostex code.
//!
//! Contract notes (deliberate divergences from Ghostex):
//! - Comparison is by SHA-256 content hash, never by mtime.
//! - Symlinks are refused with [`SyncError::SymlinkRefused`]: never
//!   followed, never copied, never descended into.
//! - Files land with the executable bits stripped
//!   ([`SYNCED_FILE_MODE`] = `0o644`).
//! - Backups mirror relative paths under the backup dir, plus the
//!   in-progress marker [`BACKUP_MARKER`].
//! - Writes are atomic: temp file in the same directory + rename.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// Hard cap on files indexed by one scan (bounded work).
pub const MAX_SCAN_FILES: usize = 10_000;
/// Files larger than this are refused, never read into memory.
/// 8 MiB — generous for skill markdown and small configs; anything
/// larger is refused, not truncated.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Mode every synced file lands with: exec bits stripped.
pub const SYNCED_FILE_MODE: u32 = 0o644;
/// Marker written into the backup dir before the first write of an
/// apply; removed when the apply completes. A leftover marker means
/// the previous apply died mid-way and the next apply restores the
/// backups first.
pub const BACKUP_MARKER: &str = ".sync-in-progress";

/// Every filesystem access the sync performs, in order. Recorded so
/// tests (and operators) can audit exactly what changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsAccess {
    /// Atomic write completed at this path.
    Write { path: PathBuf },
    /// Directory created (parents for writes/backups).
    Mkdir { path: PathBuf },
    /// Target file deleted.
    Remove { path: PathBuf },
    /// Target file copied to the backup dir.
    Backup { path: PathBuf },
    /// Backup copied back into the target (crash recovery).
    Restore { path: PathBuf },
}

/// Errors for the skill-sync pipeline. Every variant names the
/// relative path involved where one exists.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncError {
    /// A relative path escapes its root (absolute path, `..`,
    /// trailing separator, or empty).
    Traversal { rel: String },
    /// A symlink was encountered where a regular file was required.
    /// Symlinks are never followed or copied.
    SymlinkRefused { rel: String },
    /// File larger than [`MAX_FILE_BYTES`].
    TooLarge { rel: String, bytes: u64 },
    /// The target changed between scan and apply (hash mismatch or
    /// deletion). Nothing was written.
    ConcurrentModification { rel: String },
    /// Test-only kill switch fired (see
    /// [`ApplyOptions::kill_after_ops`]).
    Killed,
    /// Any underlying I/O failure, with context.
    Io { what: String, detail: String },
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Traversal { rel } => write!(f, "path escapes root: {rel}"),
            SyncError::SymlinkRefused { rel } => write!(f, "symlink refused: {rel}"),
            SyncError::TooLarge { rel, bytes } => {
                write!(f, "file too large ({bytes} bytes): {rel}")
            }
            SyncError::ConcurrentModification { rel } => {
                write!(f, "target changed during sync: {rel}")
            }
            SyncError::Killed => write!(f, "killed mid-apply (test hook)"),
            SyncError::Io { what, detail } => write!(f, "io error at {what}: {detail}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// Build an [`SyncError::Io`] from an [`io::Error`].
fn io_err(what: &str, e: io::Error) -> SyncError {
    SyncError::Io {
        what: what.to_string(),
        detail: e.to_string(),
    }
}

/// One indexed file: its SHA-256 and length, plus whether the dir
/// entry was a symlink (symlinks are indexed, never followed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillFile {
    /// SHA-256 of the file bytes (for symlinks: of the link target
    /// text, used only for comparison, never followed).
    pub hash: [u8; 32],
    /// Byte length (0 for symlinks).
    pub len: u64,
    /// True when the dir entry was a symlink.
    pub is_symlink: bool,
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Lowercase hex of the SHA-256 of `bytes` (for test assertions).
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Join `rel` onto `root`, refusing anything that escapes `root`.
/// Lexical check on components — no canonicalization of a path that
/// may not exist yet, and no reliance on the target of a symlink.
pub fn contained_path(root: &Path, rel: &str) -> Result<PathBuf, SyncError> {
    if rel.is_empty() {
        return Err(SyncError::Traversal {
            rel: rel.to_string(),
        });
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(SyncError::Traversal {
            rel: rel.to_string(),
        });
    }
    for component in p.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(SyncError::Traversal {
                    rel: rel.to_string(),
                });
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    if p.file_name().is_none() {
        return Err(SyncError::Traversal {
            rel: rel.to_string(),
        });
    }
    Ok(root.join(p))
}

/// Index every regular file under `root` (iterative walk, no
/// recursion). Symlinks are indexed, never followed or descended
/// into.
pub fn scan(root: &Path) -> Result<BTreeMap<String, SkillFile>, SyncError> {
    let mut files: BTreeMap<String, SkillFile> = BTreeMap::new();
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| io_err(&dir.display().to_string(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_err(&dir.display().to_string(), e))?;
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .map_err(|e| SyncError::Io {
                    what: path.display().to_string(),
                    detail: e.to_string(),
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let meta =
                fs::symlink_metadata(&path).map_err(|e| io_err(&path.display().to_string(), e))?;
            if meta.file_type().is_symlink() {
                let target =
                    fs::read_link(&path).map_err(|e| io_err(&path.display().to_string(), e))?;
                files.insert(
                    rel,
                    SkillFile {
                        hash: sha256(target.to_string_lossy().as_bytes()),
                        len: 0,
                        is_symlink: true,
                    },
                );
            } else if meta.is_dir() {
                dirs.push(path);
            } else if meta.is_file() {
                if meta.len() > MAX_FILE_BYTES {
                    return Err(SyncError::TooLarge {
                        rel,
                        bytes: meta.len(),
                    });
                }
                let bytes = fs::read(&path).map_err(|e| io_err(&path.display().to_string(), e))?;
                files.insert(
                    rel,
                    SkillFile {
                        hash: sha256(&bytes),
                        len: bytes.len() as u64,
                        is_symlink: false,
                    },
                );
            }
            if files.len() > MAX_SCAN_FILES {
                return Err(SyncError::Io {
                    what: root.display().to_string(),
                    detail: format!("more than {MAX_SCAN_FILES} files"),
                });
            }
        }
    }
    Ok(files)
}

/// One planned filesystem operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanVerb {
    /// In canonical, missing in target: copy in.
    Add,
    /// In both, hashes differ: replace (old version backed up).
    Update,
    /// In target, missing in canonical: remove (backed up first).
    Remove,
}

/// One planned operation. `expected_hash` is the target hash seen at
/// plan time (Some for Update/Remove) — apply re-hashes and aborts on
/// mismatch, which is the concurrent-modification check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanOp {
    pub verb: PlanVerb,
    pub rel: String,
    pub expected_hash: Option<[u8; 32]>,
}

/// The ordered, deterministic plan: ops sorted by relative path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPlan {
    pub ops: Vec<PlanOp>,
}

impl SyncPlan {
    /// True when the sync is a no-op.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Human-readable lines, one per op — what dry-run prints.
    pub fn describe(&self) -> Vec<String> {
        self.ops
            .iter()
            .map(|op| {
                let verb = match op.verb {
                    PlanVerb::Add => "add",
                    PlanVerb::Update => "update",
                    PlanVerb::Remove => "remove",
                };
                format!("{verb} {}", op.rel)
            })
            .collect()
    }
}

/// Diff canonical vs target by content hash (mtime is ignored):
/// canonical-only → Add, hash differs → Update, target-only → Remove.
pub fn build_plan(
    canonical: &BTreeMap<String, SkillFile>,
    target: &BTreeMap<String, SkillFile>,
) -> SyncPlan {
    let mut ops: Vec<PlanOp> = Vec::new();
    for (rel, c) in canonical {
        match target.get(rel) {
            None => ops.push(PlanOp {
                verb: PlanVerb::Add,
                rel: rel.clone(),
                expected_hash: None,
            }),
            Some(t) if t.hash != c.hash => ops.push(PlanOp {
                verb: PlanVerb::Update,
                rel: rel.clone(),
                expected_hash: Some(t.hash),
            }),
            Some(_) => {}
        }
    }
    for (rel, t) in target {
        if !canonical.contains_key(rel) {
            ops.push(PlanOp {
                verb: PlanVerb::Remove,
                rel: rel.clone(),
                expected_hash: Some(t.hash),
            });
        }
    }
    ops.sort_by(|a, b| a.rel.cmp(&b.rel));
    SyncPlan { ops }
}

/// Options for [`apply`].
#[derive(Clone, Debug)]
pub struct ApplyOptions {
    /// When true, validate and describe the plan but perform zero
    /// filesystem writes.
    pub dry_run: bool,
    /// Where replaced/removed files are backed up, mirrored by
    /// relative path, plus the in-progress marker.
    pub backup_dir: PathBuf,
    /// Test-only kill switch: after this many ops complete, return
    /// [`SyncError::Killed`] without removing the in-progress marker,
    /// simulating a crash mid-apply. `None` disables it.
    pub kill_after_ops: Option<usize>,
}

impl ApplyOptions {
    /// Normal (non-dry-run) options with the given backup dir.
    pub fn new(backup_dir: PathBuf) -> Self {
        ApplyOptions {
            dry_run: false,
            backup_dir,
            kill_after_ops: None,
        }
    }
}

/// What one [`apply`] run did.
#[derive(Clone, Debug)]
pub struct ApplyReport {
    /// Ops executed, in order.
    pub applied: Vec<PlanOp>,
    /// Relative paths backed up before writing.
    pub backed_up: Vec<String>,
    /// Relative paths restored from backup (crash recovery only).
    pub restored: Vec<String>,
    /// True when this was a dry run (nothing was written).
    pub dry_run: bool,
}

/// Write `bytes` to `dest` atomically (temp file in the same dir +
/// rename) with the executable bits stripped.
fn write_file_atomic(dest: &Path, bytes: &[u8], log: &mut Vec<FsAccess>) -> Result<(), SyncError> {
    let parent = dest.parent().ok_or_else(|| SyncError::Io {
        what: dest.display().to_string(),
        detail: "no parent directory".to_string(),
    })?;
    if !parent.exists() {
        fs::create_dir_all(parent).map_err(|e| io_err(&parent.display().to_string(), e))?;
        log.push(FsAccess::Mkdir {
            path: parent.to_path_buf(),
        });
    }
    let tmp = parent.join(format!(".sync-tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| io_err(&tmp.display().to_string(), e))?;
    let mut perms = fs::metadata(&tmp)
        .map_err(|e| io_err(&tmp.display().to_string(), e))?
        .permissions();
    perms.set_mode(SYNCED_FILE_MODE);
    fs::set_permissions(&tmp, perms).map_err(|e| io_err(&tmp.display().to_string(), e))?;
    fs::rename(&tmp, dest).map_err(|e| io_err(&dest.display().to_string(), e))?;
    log.push(FsAccess::Write {
        path: dest.to_path_buf(),
    });
    Ok(())
}

/// Validate one op's paths: the rel must stay inside both roots, the
/// canonical source must exist and not be a symlink, and the target
/// path must not currently be a symlink (never follow one).
fn validate_op(
    op: &PlanOp,
    canonical_root: &Path,
    canonical: &BTreeMap<String, SkillFile>,
    target_root: &Path,
) -> Result<(), SyncError> {
    let target_path = contained_path(target_root, &op.rel)?;
    match op.verb {
        PlanVerb::Add | PlanVerb::Update => {
            let _ = contained_path(canonical_root, &op.rel)?;
            let entry = canonical.get(&op.rel).ok_or_else(|| SyncError::Io {
                what: op.rel.clone(),
                detail: "canonical entry missing for planned op".to_string(),
            })?;
            if entry.is_symlink {
                return Err(SyncError::SymlinkRefused {
                    rel: op.rel.clone(),
                });
            }
        }
        PlanVerb::Remove => {}
    }
    if let Ok(meta) = fs::symlink_metadata(&target_path)
        && meta.file_type().is_symlink()
    {
        return Err(SyncError::SymlinkRefused {
            rel: op.rel.clone(),
        });
    }
    Ok(())
}

/// Re-hash the target files named by Update/Remove ops and compare
/// against the plan-time hashes. Any mismatch means the target
/// changed after the scan: abort before any write.
fn reverify_target(plan: &SyncPlan, target_root: &Path) -> Result<(), SyncError> {
    for op in &plan.ops {
        let Some(expected) = op.expected_hash else {
            continue;
        };
        let path = contained_path(target_root, &op.rel)?;
        // A file deleted after the scan is a concurrent modification
        // too — the plan-time state no longer holds.
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SyncError::ConcurrentModification {
                    rel: op.rel.clone(),
                });
            }
            Err(e) => return Err(io_err(&path.display().to_string(), e)),
        };
        if sha256(&bytes) != expected {
            return Err(SyncError::ConcurrentModification {
                rel: op.rel.clone(),
            });
        }
    }
    Ok(())
}

/// Copy every file under the backup dir (except the marker) back
/// into the target, restoring the pre-sync state after a crash.
/// Returns the restored relative paths.
fn restore_backups(
    backup_dir: &Path,
    target_root: &Path,
    log: &mut Vec<FsAccess>,
) -> Result<Vec<String>, SyncError> {
    let mut restored: Vec<String> = Vec::new();
    let mut dirs: Vec<PathBuf> = vec![backup_dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| io_err(&dir.display().to_string(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_err(&dir.display().to_string(), e))?;
            let path = entry.path();
            if entry.file_name().to_string_lossy() == BACKUP_MARKER
                && path.parent() == Some(backup_dir)
            {
                continue;
            }
            let meta =
                fs::symlink_metadata(&path).map_err(|e| io_err(&path.display().to_string(), e))?;
            if meta.is_dir() {
                dirs.push(path);
            } else if meta.is_file() {
                let rel = path
                    .strip_prefix(backup_dir)
                    .map_err(|e| SyncError::Io {
                        what: path.display().to_string(),
                        detail: e.to_string(),
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");
                let dest = contained_path(target_root, &rel)?;
                let bytes = fs::read(&path).map_err(|e| io_err(&path.display().to_string(), e))?;
                write_file_atomic(&dest, &bytes, log)?;
                log.push(FsAccess::Restore { path: dest.clone() });
                restored.push(rel);
            }
        }
    }
    Ok(restored)
}

/// Back up every target file named by an Update/Remove op, before any
/// write happens. Copies (not renames): the target keeps its content
/// until the op executes.
fn backup_targets(
    plan: &SyncPlan,
    target_root: &Path,
    backup_dir: &Path,
    log: &mut Vec<FsAccess>,
) -> Result<Vec<String>, SyncError> {
    let mut backed_up: Vec<String> = Vec::new();
    for op in &plan.ops {
        match op.verb {
            PlanVerb::Add => {}
            PlanVerb::Update | PlanVerb::Remove => {
                let path = contained_path(target_root, &op.rel)?;
                let meta = match fs::symlink_metadata(&path) {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(io_err(&path.display().to_string(), e)),
                };
                if !meta.is_file() {
                    continue;
                }
                let dest = contained_path(backup_dir, &op.rel)?;
                if let Some(parent) = dest.parent()
                    && !parent.exists()
                {
                    fs::create_dir_all(parent)
                        .map_err(|e| io_err(&parent.display().to_string(), e))?;
                    log.push(FsAccess::Mkdir {
                        path: parent.to_path_buf(),
                    });
                }
                fs::copy(&path, &dest).map_err(|e| io_err(&dest.display().to_string(), e))?;
                log.push(FsAccess::Backup { path: dest.clone() });
                backed_up.push(op.rel.clone());
            }
        }
    }
    Ok(backed_up)
}

/// Execute one planned op. Add/Update copy the canonical bytes
/// through an atomic write (exec bits stripped); Remove deletes the
/// target.
fn execute_op(
    op: &PlanOp,
    canonical_root: &Path,
    canonical: &BTreeMap<String, SkillFile>,
    target_root: &Path,
    log: &mut Vec<FsAccess>,
) -> Result<(), SyncError> {
    match op.verb {
        PlanVerb::Add | PlanVerb::Update => {
            let _entry = canonical.get(&op.rel).ok_or_else(|| SyncError::Io {
                what: op.rel.clone(),
                detail: "canonical entry missing for planned op".to_string(),
            })?;
            let src = contained_path(canonical_root, &op.rel)?;
            if fs::symlink_metadata(&src)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(SyncError::SymlinkRefused {
                    rel: op.rel.clone(),
                });
            }
            let bytes = fs::read(&src).map_err(|e| io_err(&src.display().to_string(), e))?;
            let dest = contained_path(target_root, &op.rel)?;
            write_file_atomic(&dest, &bytes, log)?;
        }
        PlanVerb::Remove => {
            let dest = contained_path(target_root, &op.rel)?;
            fs::remove_file(&dest).map_err(|e| io_err(&dest.display().to_string(), e))?;
            log.push(FsAccess::Remove { path: dest.clone() });
        }
    }
    Ok(())
}

/// Apply a plan: crash recovery, dry-run short-circuit, path
/// validation, concurrent-modification re-verification, backups, then
/// ordered execution. Every op path is traversal-checked before any
/// write; symlinks are refused, never followed.
pub fn apply(
    plan: &SyncPlan,
    canonical_root: &Path,
    canonical: &BTreeMap<String, SkillFile>,
    target_root: &Path,
    opts: &ApplyOptions,
    log: &mut Vec<FsAccess>,
) -> Result<ApplyReport, SyncError> {
    let marker = opts.backup_dir.join(BACKUP_MARKER);
    let mut restored: Vec<String> = Vec::new();
    if marker.exists() {
        // Previous apply died mid-way: restore the pre-sync state
        // from the backups before doing anything else.
        restored = restore_backups(&opts.backup_dir, target_root, log)?;
        fs::remove_file(&marker).map_err(|e| io_err(&marker.display().to_string(), e))?;
    }
    for op in &plan.ops {
        validate_op(op, canonical_root, canonical, target_root)?;
    }
    if opts.dry_run {
        return Ok(ApplyReport {
            applied: Vec::new(),
            backed_up: Vec::new(),
            restored,
            dry_run: true,
        });
    }
    // Concurrent-modification check: re-hash target files and
    // compare against plan-time hashes before any write.
    reverify_target(plan, target_root)?;
    // Back up everything the plan will replace or remove, then
    // record that an apply is in progress.
    let backed_up = backup_targets(plan, target_root, &opts.backup_dir, log)?;
    if !opts.backup_dir.exists() {
        fs::create_dir_all(&opts.backup_dir)
            .map_err(|e| io_err(&opts.backup_dir.display().to_string(), e))?;
        log.push(FsAccess::Mkdir {
            path: opts.backup_dir.clone(),
        });
    }
    fs::write(&marker, b"in-progress").map_err(|e| io_err(&marker.display().to_string(), e))?;
    let mut applied: Vec<PlanOp> = Vec::new();
    for op in &plan.ops {
        execute_op(op, canonical_root, canonical, target_root, log)?;
        applied.push(op.clone());
        if let Some(kill_after) = opts.kill_after_ops
            && applied.len() >= kill_after
        {
            // Test-only crash simulation: the marker stays behind so
            // the next apply restores the backups.
            return Err(SyncError::Killed);
        }
    }
    fs::remove_file(&marker).map_err(|e| io_err(&marker.display().to_string(), e))?;
    Ok(ApplyReport {
        applied,
        backed_up,
        restored,
        dry_run: false,
    })
}

/// Full pipeline: scan both dirs, build the plan, apply it.
pub fn sync(
    canonical_root: &Path,
    target_root: &Path,
    opts: &ApplyOptions,
    log: &mut Vec<FsAccess>,
) -> Result<ApplyReport, SyncError> {
    let canonical = scan(canonical_root)?;
    let target = scan(target_root)?;
    let plan = build_plan(&canonical, &target);
    apply(&plan, canonical_root, &canonical, target_root, opts, log)
}
