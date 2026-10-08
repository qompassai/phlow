//! The `FilePatch` applier: the loop's missing apply step.
//!
//! [`crate::changeset::validate_change_set`] decides whether a
//! change-set *may* touch the worktree; this module is what actually
//! touches it, and only in the shapes the change-set type carries:
//!
//! - [`ChangeKind::TrainlabConfig`] — the payload must parse as a
//!   JSON object (a trainlab run-config delta). Applying it changes
//!   no files; the delta is consumed by the operator's bundle
//!   production, and this module's job is to fail closed on a delta
//!   that is not a JSON object before anyone downstream trusts it.
//! - [`ChangeKind::FilePatch`] — the payload must be a unified diff
//!   in the restricted subset below, against exactly the paths the
//!   change-set lists.
//!
//! # Supported unified-diff subset
//!
//! - Optional `diff --git a/<path> b/<path>` and `index ...` header
//!   lines (ignored beyond shape).
//! - One `---`/`+++` pair per file section: `a/<path>`, `b/<path>`,
//!   or `/dev/null` on the `---` side for file creation. Renames
//!   (a-path != b-path), deletions (`+++ /dev/null`), mode changes,
//!   and `\ No newline at end of file` markers are **not** supported
//!   and fail closed as [`FailureClass::ProposalInvalid`].
//! - Hunks `@@ -start[,count] +start[,count] @@` whose context and
//!   removal lines must match the current file content exactly, and
//!   whose line counts must match the header counts.
//!
//! Files are treated as UTF-8 text lines; written results end with a
//! trailing newline when non-empty. Every file's new content is
//! computed and verified **before** any file is written, so a
//! change-set that fails partway writes nothing. Writes are atomic
//! per file (temp file in the same directory, then rename).
//!
//! Containment is never re-decided here: [`apply_change_set`] runs
//! the crate's existing [`validate_change_set`] first, and diff
//! header paths must equal the already-validated change-set paths,
//! so a forbidden surface (`.git`, root manifests,
//! `crates/phlow-trainlab`, `crates/phlow-experiment`, gate state,
//! the ledger) is a [`FailureClass::GateViolation`] out of the
//! validator, exactly as the loop records it.

use std::path::{Path, PathBuf};

use crate::changeset::{ChangeKind, ChangeSet, SurfaceError, validate_change_set};
use crate::error::FailureClass;

/// Maximum files one change-set application may write. Mirrors
/// `CHANGE_SET_PATHS_MAX`; restated so this module's bound is local.
const APPLY_FILES_MAX: usize = 32;

/// Apply one validated change-set inside `worktree_root`.
///
/// Returns the worktree-relative paths actually written (empty for
/// a `TrainlabConfig` change-set, whose application is validation
/// only). On any error, no file has been written.
///
/// # Errors
/// The [`SurfaceError`] from containment validation (a
/// [`FailureClass::GateViolation`] for forbidden surfaces), or a
/// `ProposalInvalid` error for payloads outside the supported
/// shapes, context mismatches, and I/O failures.
pub fn apply_change_set(
    change_set: &ChangeSet,
    worktree_root: &Path,
    ledger_dir: &Path,
) -> Result<Vec<String>, SurfaceError> {
    validate_change_set(change_set, worktree_root, ledger_dir)?;
    match change_set.kind {
        ChangeKind::TrainlabConfig => {
            let parsed: serde_json::Value = serde_json::from_str(&change_set.payload)
                .map_err(|err| invalid(format!("config delta is not valid JSON: {err}")))?;
            if !parsed.is_object() {
                return Err(invalid("config delta must be a JSON object"));
            }
            Ok(Vec::new())
        }
        ChangeKind::FilePatch => apply_file_patch(change_set, worktree_root),
    }
}

/// A `ProposalInvalid` apply error with the given detail.
fn invalid(message: impl Into<String>) -> SurfaceError {
    SurfaceError {
        class: FailureClass::ProposalInvalid,
        message: message.into(),
    }
}

/// One file section of a parsed diff: target path, whether it is a
/// creation, and the hunks in order.
struct FileSection {
    path: String,
    creation: bool,
    hunks: Vec<Hunk>,
}

/// One hunk: 1-based starts, declared counts, and body lines.
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

/// One hunk body line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Context,
    Remove,
    Add,
}

/// One hunk body line with its text (no terminator).
struct DiffLine {
    kind: LineKind,
    text: String,
}

/// Parse the restricted unified-diff subset.
fn parse_diff(payload: &str) -> Result<Vec<FileSection>, SurfaceError> {
    let mut sections: Vec<FileSection> = Vec::new();
    let mut lines = payload.lines().peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() || line.starts_with("index ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let mut parts = rest.splitn(2, ' ');
            let a = parts.next().unwrap_or_default();
            let b = parts.next().unwrap_or_default();
            if !a.starts_with("a/") || !b.starts_with("b/") || a[2..] != b[2..] {
                return Err(invalid(format!("unsupported diff --git header: {line}")));
            }
            continue;
        }
        if let Some(old_spec) = line.strip_prefix("--- ") {
            let new_line = lines
                .next()
                .ok_or_else(|| invalid("diff section missing +++ line"))?;
            let new_spec = new_line
                .strip_prefix("+++ ")
                .ok_or_else(|| invalid(format!("diff section malformed +++ line: {new_line}")))?;
            if new_spec == "/dev/null" {
                return Err(invalid(
                    "file deletion diffs are not a supported change-set shape",
                ));
            }
            let path = new_spec
                .strip_prefix("b/")
                .ok_or_else(|| invalid(format!("+++ path is not a b/ path: {new_spec}")))?
                .to_string();
            let creation = if old_spec == "/dev/null" {
                true
            } else {
                let old_path = old_spec
                    .strip_prefix("a/")
                    .ok_or_else(|| invalid(format!("--- path is not an a/ path: {old_spec}")))?;
                if old_path != path {
                    return Err(invalid(format!(
                        "rename diffs are not supported: {old_path} -> {path}"
                    )));
                }
                false
            };
            let mut section = FileSection {
                path,
                creation,
                hunks: Vec::new(),
            };
            // Hunks follow until the next section header or EOF; the
            // outer loop needs the peeked line, so parse inline here.
            while let Some(next) = lines.peek() {
                if next.starts_with("@@ ") {
                    let header = lines.next().unwrap_or_default().to_string();
                    section.hunks.push(parse_hunk(&header, &mut lines)?);
                } else {
                    break;
                }
            }
            if section.hunks.is_empty() {
                return Err(invalid(format!(
                    "diff section for {} carries no hunks",
                    section.path
                )));
            }
            sections.push(section);
            if sections.len() > APPLY_FILES_MAX {
                return Err(invalid("diff carries too many file sections"));
            }
            continue;
        }
        return Err(invalid(format!(
            "unsupported line in unified diff payload: {line}"
        )));
    }
    if sections.is_empty() {
        return Err(invalid("file_patch payload carries no diff sections"));
    }
    Ok(sections)
}

/// Parse one hunk header and its body lines from the line stream.
fn parse_hunk(
    header: &str,
    lines: &mut std::iter::Peekable<std::str::Lines<'_>>,
) -> Result<Hunk, SurfaceError> {
    let inner = header
        .strip_prefix("@@ ")
        .and_then(|rest| rest.split(" @@").next())
        .ok_or_else(|| invalid(format!("malformed hunk header: {header}")))?;
    let mut parts = inner.split(' ');
    let old_part = parts.next().unwrap_or_default();
    let new_part = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return Err(invalid(format!("malformed hunk header: {header}")));
    }
    let (old_start, old_count) = parse_range(old_part, '-')?;
    let (new_start, new_count) = parse_range(new_part, '+')?;
    let mut body: Vec<DiffLine> = Vec::new();
    let (mut old_seen, mut new_seen) = (0_usize, 0_usize);
    while old_seen < old_count || new_seen < new_count {
        let Some(line) = lines.next() else {
            return Err(invalid("hunk body truncated before its declared counts"));
        };
        let (kind, text) = match line.chars().next() {
            Some(' ') => (LineKind::Context, &line[1..]),
            Some('-') => (LineKind::Remove, &line[1..]),
            Some('+') => (LineKind::Add, &line[1..]),
            Some('\\') => {
                return Err(invalid(
                    "'\\ No newline' markers are not a supported change-set shape",
                ));
            }
            _ => return Err(invalid(format!("malformed hunk body line: {line}"))),
        };
        match kind {
            LineKind::Context => {
                old_seen += 1;
                new_seen += 1;
            }
            LineKind::Remove => old_seen += 1,
            LineKind::Add => new_seen += 1,
        }
        if old_seen > old_count || new_seen > new_count {
            return Err(invalid("hunk body exceeds its declared counts"));
        }
        body.push(DiffLine {
            kind,
            text: text.to_string(),
        });
    }
    Ok(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        lines: body,
    })
}

/// Parse `-start[,count]` / `+start[,count]`; an omitted count is 1.
fn parse_range(part: &str, sign: char) -> Result<(usize, usize), SurfaceError> {
    let body = part
        .strip_prefix(sign)
        .ok_or_else(|| invalid(format!("hunk range lacks {sign}: {part}")))?;
    match body.split_once(',') {
        Some((start, count)) => Ok((
            start
                .parse()
                .map_err(|_| invalid(format!("bad hunk start: {part}")))?,
            count
                .parse()
                .map_err(|_| invalid(format!("bad hunk count: {part}")))?,
        )),
        None => Ok((
            body.parse()
                .map_err(|_| invalid(format!("bad hunk start: {part}")))?,
            1,
        )),
    }
}

/// Compute one file's new lines from its old lines and hunks,
/// verifying every context/removal line against the old content.
fn apply_hunks(
    path: &str,
    old_lines: &[String],
    hunks: &[Hunk],
) -> Result<Vec<String>, SurfaceError> {
    let mut result: Vec<String> = Vec::new();
    let mut cursor = 0_usize; // next unconsumed old line index
    for hunk in hunks {
        // Hunk old_start is 1-based; a zero-count hunk inserts after
        // line old_start, so its index is old_start itself.
        let start_index = if hunk.old_count == 0 {
            hunk.old_start
        } else {
            hunk.old_start
                .checked_sub(1)
                .ok_or_else(|| invalid(format!("hunk for {path} starts at line 0")))?
        };
        if start_index < cursor || start_index > old_lines.len() {
            return Err(invalid(format!(
                "hunk for {path} starts outside the file or overlaps a prior hunk"
            )));
        }
        result.extend_from_slice(&old_lines[cursor..start_index]);
        cursor = start_index;
        let mut produced = 0_usize;
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context | LineKind::Remove => {
                    let actual = old_lines.get(cursor).ok_or_else(|| {
                        invalid(format!("hunk for {path} runs past the end of the file"))
                    })?;
                    if actual != &line.text {
                        return Err(invalid(format!(
                            "context mismatch in {path} at line {}: diff expects {:?}, file has {:?}",
                            cursor + 1,
                            line.text,
                            actual
                        )));
                    }
                    cursor += 1;
                    if line.kind == LineKind::Context {
                        result.push(line.text.clone());
                        produced += 1;
                    }
                }
                LineKind::Add => {
                    result.push(line.text.clone());
                    produced += 1;
                }
            }
        }
        if produced != hunk.new_count {
            return Err(invalid(format!(
                "hunk for {path} produced {produced} lines, header declares {}",
                hunk.new_count
            )));
        }
        // new_start is informational once old-side verification has
        // pinned placement; a wildly inconsistent value still fails.
        if hunk.new_count > 0 && hunk.new_start == 0 {
            return Err(invalid(format!("hunk for {path} has new_start 0")));
        }
    }
    result.extend_from_slice(&old_lines[cursor..]);
    Ok(result)
}

/// Apply a `FilePatch` change-set: parse, cross-check the sections
/// against the validated path list, compute every file's new
/// content, and only then write (atomically per file).
fn apply_file_patch(
    change_set: &ChangeSet,
    worktree_root: &Path,
) -> Result<Vec<String>, SurfaceError> {
    let sections = parse_diff(&change_set.payload)?;
    let mut section_paths: Vec<&str> = sections.iter().map(|s| s.path.as_str()).collect();
    section_paths.sort_unstable();
    let mut listed_paths: Vec<&str> = change_set.paths.iter().map(String::as_str).collect();
    listed_paths.sort_unstable();
    if section_paths != listed_paths {
        return Err(invalid(format!(
            "diff sections {section_paths:?} do not match the change-set path list {listed_paths:?}"
        )));
    }
    // Compute everything before writing anything.
    let mut writes: Vec<(PathBuf, String, String)> = Vec::new();
    for section in &sections {
        let target = worktree_root.join(&section.path);
        let old_lines: Vec<String> = if section.creation {
            if target.exists() {
                return Err(invalid(format!(
                    "creation diff for {} but the file already exists",
                    section.path
                )));
            }
            Vec::new()
        } else {
            let text = std::fs::read_to_string(&target).map_err(|err| {
                invalid(format!("cannot read {} for patching: {err}", section.path))
            })?;
            text.lines().map(str::to_string).collect()
        };
        let new_lines = apply_hunks(&section.path, &old_lines, &section.hunks)?;
        let mut content = new_lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        writes.push((target, section.path.clone(), content));
    }
    let mut written = Vec::new();
    for (target, relative, content) in writes {
        write_atomic(&target, content.as_bytes())
            .map_err(|err| invalid(format!("cannot write {relative}: {err}")))?;
        written.push(relative);
    }
    Ok(written)
}

/// Write `bytes` to `target` atomically: temp file in the same
/// directory, then rename over the target.
fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let temp = directory.join(format!(".autoresearch-apply-{}", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::{TestDir, test_dir};

    fn setup() -> (TestDir, PathBuf, PathBuf) {
        let root = test_dir("apply");
        let worktree = root.path().join("worktree");
        let ledger = root.path().join("ledger");
        std::fs::create_dir_all(worktree.join("src")).expect("mkdir worktree");
        std::fs::create_dir_all(&ledger).expect("mkdir ledger");
        std::fs::write(worktree.join("src/lib.rs"), "alpha\nbeta\ngamma\n").expect("write lib");
        (root, worktree, ledger)
    }

    fn patch(id: &str, paths: Vec<&str>, payload: &str) -> ChangeSet {
        ChangeSet {
            id: id.to_string(),
            kind: ChangeKind::FilePatch,
            paths: paths.iter().map(|p| p.to_string()).collect(),
            payload: payload.to_string(),
            rationale: "apply test".to_string(),
        }
    }

    const REPLACE_DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\n\
        --- a/src/lib.rs\n+++ b/src/lib.rs\n\
        @@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n";

    #[test]
    fn replacement_hunk_applies() {
        let (_root, worktree, ledger) = setup();
        let cs = patch("p1", vec!["src/lib.rs"], REPLACE_DIFF);
        let written = apply_change_set(&cs, &worktree, &ledger).expect("applies");
        assert_eq!(written, vec!["src/lib.rs".to_string()]);
        let content = std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read back");
        assert_eq!(content, "alpha\nBETA\ngamma\n");
    }

    #[test]
    fn insertion_and_creation_apply() {
        let (_root, worktree, ledger) = setup();
        let insert = "--- a/src/lib.rs\n+++ b/src/lib.rs\n\
            @@ -3,0 +4,1 @@\n+delta\n";
        let cs = patch("p2", vec!["src/lib.rs"], insert);
        apply_change_set(&cs, &worktree, &ledger).expect("insert applies");
        let create = "--- /dev/null\n+++ b/src/new.rs\n\
            @@ -0,0 +1,2 @@\n+one\n+two\n";
        let cs = patch("p3", vec!["src/new.rs"], create);
        apply_change_set(&cs, &worktree, &ledger).expect("create applies");
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/new.rs")).expect("read new"),
            "one\ntwo\n"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nbeta\ngamma\ndelta\n"
        );
    }

    #[test]
    fn trainlab_config_delta_validates_without_writing() {
        let (_root, worktree, ledger) = setup();
        let cs = ChangeSet {
            id: "cfg".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{\"temperature\": 0.5}".to_string(),
            rationale: "config".to_string(),
        };
        let written = apply_change_set(&cs, &worktree, &ledger).expect("config applies");
        assert!(written.is_empty());
    }

    #[test]
    fn context_mismatch_writes_nothing() {
        let (_root, worktree, ledger) = setup();
        // Removal line no longer matches the file content.
        let cs = patch(
            "p4",
            vec!["src/lib.rs"],
            &REPLACE_DIFF.replace("-beta", "-nope"),
        );
        let err = apply_change_set(&cs, &worktree, &ledger).expect_err("must fail");
        assert_eq!(err.class, FailureClass::ProposalInvalid);
        assert!(err.message.contains("context mismatch"), "{}", err.message);
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nbeta\ngamma\n"
        );
    }

    #[test]
    fn forbidden_surface_is_gate_violation() {
        let (_root, worktree, ledger) = setup();
        let diff = "--- a/crates/phlow-trainlab/src/lib.rs\n\
            +++ b/crates/phlow-trainlab/src/lib.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let cs = patch("p5", vec!["crates/phlow-trainlab/src/lib.rs"], diff);
        let err = apply_change_set(&cs, &worktree, &ledger).expect_err("must fail");
        assert_eq!(err.class, FailureClass::GateViolation);
    }

    #[test]
    fn diff_header_outside_path_list_is_rejected() {
        let (_root, worktree, ledger) = setup();
        let cs = patch("p6", vec!["src/other.rs"], REPLACE_DIFF);
        let err = apply_change_set(&cs, &worktree, &ledger).expect_err("must fail");
        assert!(err.message.contains("do not match"), "{}", err.message);
    }

    #[test]
    fn deletion_and_rename_shapes_are_rejected() {
        let (_root, worktree, ledger) = setup();
        let deletion = "--- a/src/lib.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-alpha\n";
        let cs = patch("p7", vec!["src/lib.rs"], deletion);
        assert!(apply_change_set(&cs, &worktree, &ledger).is_err());
        let rename = "--- a/src/lib.rs\n+++ b/src/renamed.rs\n@@ -1,1 +1,1 @@\n alpha\n";
        let cs = patch("p8", vec!["src/lib.rs"], rename);
        assert!(apply_change_set(&cs, &worktree, &ledger).is_err());
    }

    #[test]
    fn malformed_config_and_truncated_hunk_are_rejected() {
        let (_root, worktree, ledger) = setup();
        let cs = ChangeSet {
            id: "cfg-bad".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "[1,2]".to_string(),
            rationale: "config".to_string(),
        };
        assert!(apply_change_set(&cs, &worktree, &ledger).is_err());
        let truncated = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n alpha\n";
        let cs = patch("p9", vec!["src/lib.rs"], truncated);
        assert!(apply_change_set(&cs, &worktree, &ledger).is_err());
    }

    #[test]
    fn multi_file_failure_writes_neither_file() {
        let (root, worktree, ledger) = setup();
        std::fs::write(worktree.join("src/second.rs"), "one\ntwo\n").expect("write second");
        let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-alpha\n+ALPHA\n\
            --- a/src/second.rs\n+++ b/src/second.rs\n@@ -1,2 +1,2 @@\n one\n-WRONG\n+two2\n";
        let cs = patch("p10", vec!["src/lib.rs", "src/second.rs"], diff);
        let err = apply_change_set(&cs, &worktree, &ledger).expect_err("must fail");
        assert_eq!(err.class, FailureClass::ProposalInvalid);
        assert_eq!(
            std::fs::read_to_string(worktree.join("src/lib.rs")).expect("read lib"),
            "alpha\nbeta\ngamma\n",
            "first file must be untouched when the second fails"
        );
        let _ = root;
    }
}
