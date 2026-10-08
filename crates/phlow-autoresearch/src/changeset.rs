//! Change-sets: the loop's only mutable artifact. One experiment is one
//! change-set — a typed, bounded proposal against an allowlisted
//! surface. Validation here is the containment boundary: nothing is
//! applied until a change-set has passed every check in this module,
//! and the checks distinguish an ordinary invalid proposal
//! ([`FailureClass::ProposalInvalid`]) from an attempt on a forbidden
//! surface ([`FailureClass::GateViolation`]), because the loop halts
//! after two of the latter.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::FailureClass;

/// Maximum characters in a change-set identifier.
pub const CHANGE_SET_ID_CHARS_MAX: usize = 128;
/// Maximum paths in one change-set.
pub const CHANGE_SET_PATHS_MAX: usize = 32;
/// Maximum characters in one change-set path.
pub const CHANGE_SET_PATH_CHARS_MAX: usize = 512;
/// Maximum payload size in bytes (a config delta or a unified patch).
pub const CHANGE_SET_PAYLOAD_BYTES_MAX: usize = 16 * 1024;
/// Maximum characters in the human-readable rationale.
pub const CHANGE_SET_RATIONALE_CHARS_MAX: usize = 512;

/// First path components a change-set may never touch: version-control
/// internals. The fixed harness and build manifests are checked
/// separately below because they are paths, not single components.
const FORBIDDEN_FIRST_COMPONENTS: &[&str] = &[".git"];
/// Root-relative paths a change-set may never touch: the workspace
/// build manifests (the loop may not rewrite its own build) and the
/// fixed harness crate (trainlab is the ground truth, not a subject).
const FORBIDDEN_PATH_PREFIXES: &[&[&str]] = &[
    &["Cargo.toml"],
    &["Cargo.lock"],
    &["crates", "phlow-trainlab"],
    &["crates", "phlow-experiment"],
];

/// Which surface a change-set acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A delta to a trainlab run configuration. Carries no paths.
    TrainlabConfig,
    /// A unified patch against files inside the experiment worktree.
    FilePatch,
}

/// One experiment proposal. All fields are untrusted until
/// [`validate_change_set`] has accepted the value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChangeSet {
    /// Proposer-chosen identifier, `[A-Za-z0-9._-]`, bounded.
    pub id: String,
    /// The surface this change-set acts on.
    pub kind: ChangeKind,
    /// Worktree-relative target paths (`FilePatch` only).
    pub paths: Vec<String>,
    /// Config delta JSON or unified patch text, bounded.
    pub payload: String,
    /// One-line rationale, recorded in the ledger.
    pub rationale: String,
}

impl ChangeSet {
    /// SHA-256 of the canonical JSON encoding — the identity the
    /// ledger records. Deterministic: struct field order is fixed by
    /// the derived serialization.
    #[must_use]
    pub fn sha256(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        format!("{:x}", Sha256::digest(canonical.as_bytes()))
    }
}

/// A validation rejection, classified for the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceError {
    /// The failure class the loop records for this rejection.
    pub class: FailureClass,
    /// Bounded human-readable detail.
    pub message: String,
}

impl SurfaceError {
    fn invalid(message: impl Into<String>) -> Self {
        SurfaceError {
            class: FailureClass::ProposalInvalid,
            message: message.into(),
        }
    }

    fn gate_violation(message: impl Into<String>) -> Self {
        SurfaceError {
            class: FailureClass::GateViolation,
            message: message.into(),
        }
    }
}

/// Validate a change-set against every bound and the containment
/// contract. `worktree_root` must exist; `ledger_dir` is the loop's own
/// ledger (change-sets may never write into it, wherever it lives).
///
/// Rejected, as [`FailureClass::GateViolation`]: absolute paths, `..`
/// components, paths escaping the worktree through symlinks, paths
/// under the ledger directory, and the forbidden prefixes above.
/// Rejected, as [`FailureClass::ProposalInvalid`]: everything else
/// (sizes, charsets, kind/path mismatches).
pub fn validate_change_set(
    change_set: &ChangeSet,
    worktree_root: &Path,
    ledger_dir: &Path,
) -> Result<(), SurfaceError> {
    validate_shape(change_set)?;
    if change_set.kind == ChangeKind::FilePatch {
        let root_canonical = worktree_root
            .canonicalize()
            .map_err(|err| SurfaceError::invalid(format!("worktree root unreadable: {err}")))?;
        let ledger_canonical = ledger_dir.canonicalize().ok();
        for raw_path in &change_set.paths {
            validate_path(raw_path, &root_canonical, ledger_canonical.as_deref())?;
        }
    }
    Ok(())
}

/// Bounds and shape checks that need no filesystem access.
fn validate_shape(change_set: &ChangeSet) -> Result<(), SurfaceError> {
    let id = &change_set.id;
    if id.is_empty() || id.chars().count() > CHANGE_SET_ID_CHARS_MAX {
        return Err(SurfaceError::invalid("change-set id empty or too long"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(SurfaceError::invalid(
            "change-set id outside [A-Za-z0-9._-]",
        ));
    }
    if change_set.rationale.chars().count() > CHANGE_SET_RATIONALE_CHARS_MAX {
        return Err(SurfaceError::invalid("rationale too long"));
    }
    if change_set.payload.len() > CHANGE_SET_PAYLOAD_BYTES_MAX {
        return Err(SurfaceError::invalid("payload too large"));
    }
    if change_set.paths.len() > CHANGE_SET_PATHS_MAX {
        return Err(SurfaceError::invalid("too many paths"));
    }
    for path in &change_set.paths {
        if path.is_empty() || path.chars().count() > CHANGE_SET_PATH_CHARS_MAX {
            return Err(SurfaceError::invalid("path empty or too long"));
        }
    }
    match change_set.kind {
        ChangeKind::TrainlabConfig if !change_set.paths.is_empty() => Err(SurfaceError::invalid(
            "trainlab_config change-sets carry no paths",
        )),
        ChangeKind::FilePatch if change_set.paths.is_empty() => Err(SurfaceError::invalid(
            "file_patch change-sets need at least one path",
        )),
        _ => Ok(()),
    }
}

/// Containment checks for one worktree-relative path.
fn validate_path(
    raw_path: &str,
    root_canonical: &Path,
    ledger_canonical: Option<&Path>,
) -> Result<(), SurfaceError> {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        return Err(SurfaceError::gate_violation(format!(
            "absolute path refused: {raw_path}"
        )));
    }
    let mut components: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(SurfaceError::gate_violation(format!(
                    "parent traversal refused: {raw_path}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(SurfaceError::gate_violation(format!(
                    "rooted path refused: {raw_path}"
                )));
            }
        }
    }
    if components.is_empty() {
        return Err(SurfaceError::invalid("path has no target component"));
    }
    if FORBIDDEN_FIRST_COMPONENTS.contains(&components[0].as_str()) {
        return Err(SurfaceError::gate_violation(format!(
            "forbidden surface refused: {raw_path}"
        )));
    }
    for prefix in FORBIDDEN_PATH_PREFIXES {
        if components.len() >= prefix.len()
            && components[..prefix.len()]
                .iter()
                .map(String::as_str)
                .eq(prefix.iter().copied())
        {
            return Err(SurfaceError::gate_violation(format!(
                "forbidden surface refused: {raw_path}"
            )));
        }
    }
    // Containment by canonical ancestry, never by string prefix: the
    // deepest existing ancestor of the target must resolve inside the
    // worktree (this is what defeats symlink escapes), and the target
    // must not resolve inside the ledger directory.
    let target = root_canonical.join(path);
    let ancestor = canonical_existing_ancestor(&target).ok_or_else(|| {
        SurfaceError::invalid(format!("no existing ancestor for path: {raw_path}"))
    })?;
    if !ancestor.starts_with(root_canonical) {
        return Err(SurfaceError::gate_violation(format!(
            "path escapes the worktree: {raw_path}"
        )));
    }
    if let Some(ledger) = ledger_canonical
        && ancestor.starts_with(ledger)
    {
        return Err(SurfaceError::gate_violation(format!(
            "path targets the ledger: {raw_path}"
        )));
    }
    Ok(())
}

/// Canonicalize the deepest existing ancestor of `path` (the path
/// itself when it exists). `None` only when no ancestor exists at all,
/// which cannot happen for paths under an existing root.
fn canonical_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current: Option<&Path> = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return candidate.canonicalize().ok();
        }
        current = candidate.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::{TestDir, test_dir};

    fn config_change_set() -> ChangeSet {
        ChangeSet {
            id: "cfg-1".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "baseline".to_string(),
        }
    }

    fn patch_change_set(paths: Vec<String>) -> ChangeSet {
        ChangeSet {
            id: "patch-1".to_string(),
            kind: ChangeKind::FilePatch,
            paths,
            payload: "diff".to_string(),
            rationale: "try it".to_string(),
        }
    }

    fn setup() -> (TestDir, PathBuf, PathBuf) {
        let root = test_dir("cs-root");
        let worktree = root.path().join("worktree");
        let ledger = root.path().join("ledger");
        std::fs::create_dir_all(worktree.join("src")).expect("mkdir worktree");
        std::fs::create_dir_all(&ledger).expect("mkdir ledger");
        (root, worktree, ledger)
    }

    #[test]
    fn valid_config_change_set_passes() {
        let (_root, worktree, ledger) = setup();
        let cs = config_change_set();
        assert!(validate_change_set(&cs, &worktree, &ledger).is_ok());
        assert_eq!(cs.sha256().len(), 64);
    }

    #[test]
    fn valid_patch_inside_worktree_passes() {
        let (_root, worktree, ledger) = setup();
        let cs = patch_change_set(vec!["src/lib.rs".to_string()]);
        assert!(validate_change_set(&cs, &worktree, &ledger).is_ok());
    }

    #[test]
    fn oversized_payload_is_proposal_invalid() {
        let (_root, worktree, ledger) = setup();
        let mut cs = config_change_set();
        cs.payload = "x".repeat(CHANGE_SET_PAYLOAD_BYTES_MAX + 1);
        let err = validate_change_set(&cs, &worktree, &ledger).expect_err("must reject");
        assert_eq!(err.class, FailureClass::ProposalInvalid);
    }

    #[test]
    fn parent_traversal_is_gate_violation() {
        let (_root, worktree, ledger) = setup();
        let cs = patch_change_set(vec!["../outside.rs".to_string()]);
        let err = validate_change_set(&cs, &worktree, &ledger).expect_err("must reject");
        assert_eq!(err.class, FailureClass::GateViolation);
    }

    #[test]
    fn absolute_path_is_gate_violation() {
        let (_root, worktree, ledger) = setup();
        let cs = patch_change_set(vec!["/etc/passwd".to_string()]);
        let err = validate_change_set(&cs, &worktree, &ledger).expect_err("must reject");
        assert_eq!(err.class, FailureClass::GateViolation);
    }

    #[test]
    fn harness_path_is_gate_violation() {
        let (_root, worktree, ledger) = setup();
        let cs = patch_change_set(vec!["crates/phlow-trainlab/src/lib.rs".to_string()]);
        let err = validate_change_set(&cs, &worktree, &ledger).expect_err("must reject");
        assert_eq!(err.class, FailureClass::GateViolation);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_gate_violation() {
        let (root, worktree, ledger) = setup();
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::os::unix::fs::symlink(&outside, worktree.join("link")).expect("symlink");
        let cs = patch_change_set(vec!["link/evil.rs".to_string()]);
        let err = validate_change_set(&cs, &worktree, &ledger).expect_err("must reject");
        assert_eq!(err.class, FailureClass::GateViolation);
    }
}
