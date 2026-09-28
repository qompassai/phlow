//! Experiment harness: the safety boundary around the self-editing loop.
//!
//! The loop runs **only** here, on experiment-state skill documents,
//! inside one scratch directory per task:
//!
//! - [`Sandbox`] owns a root dir. Skill files are written under it;
//!   names containing `/` or `..` are rejected (no path traversal).
//! - [`Sandbox::export_best`] writes `best_skill.md` only with an
//!   [`ApprovalRecord`] whose SHA-256 binds the exact bytes being
//!   exported. No approval, no export. No auto-promotion.
//! - Nothing here touches diver config, phlow production skills or
//!   source, deployed skills, or any repo's real tree: the module has
//!   no API that takes a production path, and the learner refuses
//!   non-experiment documents.

use super::doc::SkillDoc;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

/// Harness failures: boundary violations, never silent.
#[derive(Debug, Clone)]
pub enum HarnessError {
    /// A write/export was attempted outside the sandbox root.
    PathEscape {
        /// The offending name.
        name: String,
    },
    /// Export attempted without a matching operator approval.
    ApprovalMissingOrMismatch {
        /// What was wrong.
        detail: String,
    },
    /// Filesystem failure creating or writing under the sandbox.
    Io {
        /// What failed.
        detail: String,
    },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathEscape { name } => {
                write!(f, "harness: path escapes the sandbox: {name:?}")
            }
            Self::ApprovalMissingOrMismatch { detail } => {
                write!(f, "harness: export refused ({detail})")
            }
            Self::Io { detail } => write!(f, "harness: I/O failed: {detail}"),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Operator approval binding the exact exported bytes. In real use a
/// human signs; tests use [`ApprovalRecord::test_fixture_for`], which is
/// labeled as a fixture, not a human.
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    /// Who approved (or `"test-fixture"`).
    pub approver: String,
    /// Hex SHA-256 over the exact exported bytes.
    pub skill_sha256: String,
    /// Human-readable decision, e.g. `"approve"`.
    pub decision: String,
}

impl ApprovalRecord {
    /// Build a fixture approval for `skill`. The approver is literally
    /// `"test-fixture"`: it can never be mistaken for a human.
    pub fn test_fixture_for(skill: &SkillDoc) -> Self {
        ApprovalRecord {
            approver: "test-fixture".to_string(),
            skill_sha256: sha256_hex(skill.render_for_export().as_bytes()),
            decision: "approve".to_string(),
        }
    }

    /// Check the record against the exact bytes it must cover.
    pub fn verifies(&self, bytes: &[u8]) -> bool {
        self.skill_sha256 == sha256_hex(bytes)
    }
}

/// Hex SHA-256 over bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        out.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    out
}

/// The experiment sandbox: one root dir per task run.
#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Create the sandbox root `<parent>/<task_id>`. Fails if the
    /// directory cannot be created.
    pub fn new(parent: &Path, task_id: &str) -> Result<Self, HarnessError> {
        if task_id.contains('/') || task_id.contains("..") {
            return Err(HarnessError::PathEscape {
                name: task_id.to_string(),
            });
        }
        let root = parent.join(task_id);
        std::fs::create_dir_all(&root).map_err(|e| HarnessError::Io {
            detail: format!("cannot create sandbox {}: {e}", root.display()),
        })?;
        Ok(Sandbox { root })
    }

    /// The sandbox root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write a skill snapshot under the sandbox. `name` is a bare file
    /// stem (no directories); anything else is a [`HarnessError::PathEscape`].
    pub fn write_skill(&self, name: &str, skill: &SkillDoc) -> Result<PathBuf, HarnessError> {
        if name.contains('/') || name.contains('\\') || name.contains("..") || name.is_empty() {
            return Err(HarnessError::PathEscape {
                name: name.to_string(),
            });
        }
        let path = self.root.join(format!("{name}.md"));
        std::fs::write(&path, skill.render_for_export()).map_err(|e| HarnessError::Io {
            detail: format!("cannot write {}: {e}", path.display()),
        })?;
        Ok(path)
    }

    /// Export the best skill. Refused unless `approval` verifies against
    /// the exact bytes written. This is the only route anything takes
    /// out of the experiment dir.
    pub fn export_best(
        &self,
        skill: &SkillDoc,
        approval: &ApprovalRecord,
    ) -> Result<PathBuf, HarnessError> {
        let bytes = skill.render_for_export();
        if !approval.verifies(bytes.as_bytes()) {
            return Err(HarnessError::ApprovalMissingOrMismatch {
                detail: "approval does not bind these exact bytes".to_string(),
            });
        }
        let path = self.root.join("best_skill.md");
        std::fs::write(&path, &bytes).map_err(|e| HarnessError::Io {
            detail: format!("cannot write {}: {e}", path.display()),
        })?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalRecord, HarnessError, Sandbox};
    use crate::skillopt::doc::SkillDoc;

    fn sandbox(name: &str) -> Sandbox {
        let parent = std::env::temp_dir().join("skillopt-harness-tests");
        Sandbox::new(&parent, name).unwrap()
    }

    /// Validation: approval-bound export writes the exact bytes.
    #[test]
    fn export_with_binding_approval() {
        let sb = sandbox("export-ok");
        let skill = SkillDoc::experiment("ORDER[0]: fetch parse validate emit");
        let approval = ApprovalRecord::test_fixture_for(&skill);
        let path = sb.export_best(&skill, &approval).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, skill.render_for_export().as_bytes());
        assert!(approval.verifies(&bytes));
        assert_eq!(approval.approver, "test-fixture");
    }

    /// Adversarial: export without a matching approval is refused.
    #[test]
    fn export_without_approval_refused() {
        let sb = sandbox("export-no-approval");
        let skill = SkillDoc::experiment("something");
        let other = SkillDoc::experiment("something else");
        let wrong = ApprovalRecord::test_fixture_for(&other);
        let r = sb.export_best(&skill, &wrong);
        assert!(
            matches!(r, Err(HarnessError::ApprovalMissingOrMismatch { .. })),
            "mismatched approval must refuse export"
        );
        assert!(!sb.root().join("best_skill.md").exists());
    }

    /// Adversarial: path traversal in sandbox/file names is refused.
    #[test]
    fn path_traversal_refused() {
        let parent = std::env::temp_dir().join("skillopt-harness-tests");
        assert!(Sandbox::new(&parent, "../escape").is_err());
        let sb = sandbox("traversal-names");
        let skill = SkillDoc::experiment("x");
        for bad in ["../evil", "sub/dir", "", "..\\win"] {
            assert!(
                sb.write_skill(bad, &skill).is_err(),
                "name {bad:?} must be refused"
            );
        }
    }
}
