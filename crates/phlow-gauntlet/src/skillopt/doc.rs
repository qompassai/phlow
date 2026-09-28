//! The skill document: the optimization state of the SkillOpt loop.
//!
//! A [`SkillDoc`] is three regions with different writers:
//!
//! - `body`: the editable text. Step-level edits ([`Edit`]) may read and
//!   write it.
//! - `protected`: delimited by [`SLOW_UPDATE_START`] / [`SLOW_UPDATE_END`].
//!   Only the epoch-end slow update writes it; step-level edits can never
//!   write inside it (paper §II.6). [`SkillDoc::apply`] rejects any edit
//!   touching it.
//! - `meta`: optimizer-private scratch (paper §II.6: "a separate file
//!   visible only to the optimizer, never read by the target model").
//!   Only the meta update writes it; the target never sees it.
//!
//! Safety: a `SkillDoc` carries [`Provenance`]. The learner runs only on
//! [`Provenance::Experiment`] documents — experiment-state skill text
//! created inside the gauntlet harness. [`SkillDoc::import_untrusted`]
//! builds the other variant so the refusal path is testable; the learner
//! rejects it with a typed error.

use std::fmt;

/// Protected-section open marker (the paper's marker).
pub const SLOW_UPDATE_START: &str = "<!-- SLOW_UPDATE_START -->";
/// Protected-section close marker (the paper's marker).
pub const SLOW_UPDATE_END: &str = "<!-- SLOW_UPDATE_END -->";
/// Per-edit payload bound in chars (task-113's territory; enforced here).
/// An edit larger than this is rejected before it touches the document.
/// This is the hard safety floor; the paper's unit is tokens
/// ([`PER_EDIT_TOKENS_MAX`]), enforced alongside it.
pub const PER_EDIT_CHARS_MAX: usize = 400;
/// Per-edit payload bound in tokens (paper §II.4 counts the textual
/// learning rate in tokens; task-113). The harness approximates one
/// token as four chars ([`Edit::tokens`]), so this bound is the
/// token-unit restatement of the 400-char floor: 400 chars ≈ 100
/// tokens. Both bounds are enforced; the token bound is the one the
/// per-edit accounting invariant is stated in.
pub const PER_EDIT_TOKENS_MAX: usize = 100;

/// Where a skill document comes from. The learner only optimizes
/// [`Provenance::Experiment`] documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Experiment-state text created inside the gauntlet harness.
    Experiment,
    /// Anything else (e.g. loaded from disk). The loop refuses these.
    Untrusted,
}

/// One atomic skill edit: the paper's four ops (paper §II.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    /// Append a line to the end of the body.
    Append {
        /// The exact line to append.
        line: String,
    },
    /// Insert a line after the first body line exactly equal to `anchor`.
    InsertAfter {
        /// Existing body line to insert after.
        anchor: String,
        /// The exact line to insert.
        line: String,
    },
    /// Replace the first contiguous run of body lines exactly equal to
    /// `old` with `new`.
    Replace {
        /// Exact span to find (one or more whole lines).
        old: String,
        /// Replacement span.
        new: String,
    },
    /// Delete the first contiguous run of body lines exactly equal to
    /// `line`.
    Delete {
        /// Exact span to delete (one or more whole lines).
        line: String,
    },
}

/// A proposed edit with its rationale and stable direction key.
///
/// `direction` names the proposal's *direction* (e.g.
/// `"order:add:3"`, `"bind:fix"`, `"noop"`). The rejected-edit buffer
/// keys on it: re-proposing a buffered direction is what the buffer
/// suppresses (task-103 measures exactly this).
#[derive(Debug, Clone)]
pub struct Edit {
    /// The operation.
    pub op: EditOp,
    /// Why the optimizer proposed it (free text, for the ledger).
    pub rationale: String,
    /// Stable direction key for the rejected-edit buffer.
    pub direction: String,
}

impl Edit {
    /// Payload size in chars, for churn accounting and the per-edit bound.
    /// Counts Unicode scalar values, not bytes.
    pub fn chars(&self) -> usize {
        match &self.op {
            EditOp::Append { line } => line.chars().count(),
            EditOp::InsertAfter { anchor, line } => anchor.chars().count() + line.chars().count(),
            EditOp::Replace { old, new } => old.chars().count() + new.chars().count(),
            EditOp::Delete { line } => line.chars().count(),
        }
    }

    /// Payload size in tokens, for the per-edit accounting invariant
    /// (task-113): every step's edits must satisfy
    /// `tokens ≤ PER_EDIT_TOKENS_MAX`. The estimate is deliberately
    /// crude — one token per four chars, rounding up — and documented
    /// as such: it is an accounting bound, not a tokenizer. Anchors
    /// count: a whole-document anchor is a whole-document payload.
    pub fn tokens(&self) -> usize {
        self.chars().div_ceil(4)
    }

    /// The op category, for meta-update bookkeeping.
    pub fn category(&self) -> &'static str {
        match &self.op {
            EditOp::Append { .. } => "append",
            EditOp::InsertAfter { .. } => "insert_after",
            EditOp::Replace { .. } => "replace",
            EditOp::Delete { .. } => "delete",
        }
    }

    /// One-line rendering, used in the rejected-edit buffer and ledgers.
    pub fn render(&self) -> String {
        match &self.op {
            EditOp::Append { line } => format!("append: {line}"),
            EditOp::InsertAfter { anchor, line } => {
                format!("insert_after: {anchor} -> {line}")
            }
            EditOp::Replace { old, new } => format!("replace: {old} -> {new}"),
            EditOp::Delete { line } => format!("delete: {line}"),
        }
    }
}

/// Failures of [`SkillDoc::apply`]: the driver mis-proposed, not the
/// document. Every variant names the rejected edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// Payload larger than [`PER_EDIT_CHARS_MAX`] chars.
    TooLarge {
        /// Measured payload chars.
        chars: usize,
    },
    /// Payload larger than [`PER_EDIT_TOKENS_MAX`] tokens
    /// (task-113: the per-edit accounting bound). This is the error a
    /// newline-joined multi-edit smuggle hits: several logical edits in
    /// one payload still count as one payload.
    PayloadTooLarge {
        /// Measured payload tokens.
        tokens: usize,
    },
    /// An edit text field contains a newline: one [`Edit`] is one line.
    /// A newline-joined "payload" of several logical edits is an edit-
    /// budget evasion (task-113 attack 3), not one edit.
    MultiLinePayload {
        /// Which field carried the newline (e.g. "append line").
        what: String,
        /// How many lines the field held.
        lines: usize,
    },
    /// Anchor/span not found in the body.
    NotFound {
        /// What was searched for (truncated).
        what: String,
    },
    /// The anchor (or replaced span) is empty: an empty anchor game —
    /// matching "nothing" is not a location.
    EmptySpan {
        /// Which op carried the empty span (truncated).
        what: String,
    },
    /// The insert anchor matches more than one body line: the edit does
    /// not name a unique location, so it is refused rather than applied
    /// to the first match (task-113: duplicate-anchor games).
    AnchorAmbiguous {
        /// The ambiguous anchor (truncated).
        anchor: String,
        /// How many body lines it matched.
        matches: usize,
    },
    /// The edit touches the protected section, or tries to forge the
    /// protected markers into the body. Step-level edits can never do
    /// either.
    ProtectedRegion,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { chars } => write!(
                f,
                "edit payload {chars} chars exceeds PER_EDIT_CHARS_MAX={PER_EDIT_CHARS_MAX}"
            ),
            Self::PayloadTooLarge { tokens } => write!(
                f,
                "edit payload {tokens} tokens exceeds PER_EDIT_TOKENS_MAX={PER_EDIT_TOKENS_MAX}"
            ),
            Self::MultiLinePayload { what, lines } => write!(
                f,
                "edit field {what} holds {lines} lines: one edit is one line (newline-joined smuggle rejected)"
            ),
            Self::NotFound { what } => write!(f, "edit span not found in skill body: {what}"),
            Self::EmptySpan { what } => {
                write!(f, "edit span is empty (empty anchor/span game): {what}")
            }
            Self::AnchorAmbiguous { anchor, matches } => write!(
                f,
                "anchor matches {matches} body lines, refusing ambiguous insert: {anchor}"
            ),
            Self::ProtectedRegion => write!(
                f,
                "edit touches the protected slow-update section or forges its markers"
            ),
        }
    }
}

impl std::error::Error for EditError {}

/// The skill document under optimization.
#[derive(Debug, Clone)]
pub struct SkillDoc {
    body: String,
    protected: String,
    meta: String,
    provenance: Provenance,
}

impl SkillDoc {
    /// New experiment-state document. The only constructor the learner
    /// accepts.
    pub fn experiment(body: &str) -> Self {
        SkillDoc {
            body: body.to_string(),
            protected: String::new(),
            meta: String::new(),
            provenance: Provenance::Experiment,
        }
    }

    /// Build a document from outside the experiment (e.g. loaded from
    /// disk). Exists so the learner's refusal path is real and testable;
    /// the loop never optimizes these.
    pub fn import_untrusted(body: &str) -> Self {
        SkillDoc {
            body: body.to_string(),
            protected: String::new(),
            meta: String::new(),
            provenance: Provenance::Untrusted,
        }
    }

    /// Document provenance.
    pub fn provenance(&self) -> Provenance {
        self.provenance
    }

    /// The editable body text.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The protected slow-update text (between the markers).
    pub fn protected(&self) -> &str {
        &self.protected
    }

    /// Optimizer-private meta text.
    pub fn meta(&self) -> &str {
        &self.meta
    }

    /// What the target (rollout) sees: body + protected section.
    /// Meta is never shown to the target (paper §II.6).
    pub fn render_for_target(&self) -> String {
        format!(
            "{}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n",
            self.body, self.protected
        )
    }

    /// What the optimizer sees: everything, including meta.
    pub fn render_for_optimizer(&self) -> String {
        format!(
            "{}\n{SLOW_UPDATE_START}\n{}\n{SLOW_UPDATE_END}\n<!-- META_START -->\n{}\n<!-- META_END -->\n",
            self.body, self.protected, self.meta
        )
    }

    /// What gets exported on operator approval: body + protected.
    pub fn render_for_export(&self) -> String {
        self.render_for_target()
    }

    /// Exact-line presence in body or protected.
    pub fn has_line(&self, line: &str) -> bool {
        self.body.lines().any(|l| l == line) || self.protected.lines().any(|l| l == line)
    }

    /// Protected `KEEP: ...` lines (the slow update's longitudinal
    /// memory). The scripted optimizer treats these as do-not-rewrite.
    pub fn keep_lines(&self) -> Vec<String> {
        self.protected
            .lines()
            .filter(|l| l.starts_with("KEEP: "))
            .map(str::to_string)
            .collect()
    }

    /// Epoch-end slow update only: replace the protected section.
    /// `pub(crate)` so step-level code paths cannot reach it.
    pub(crate) fn set_protected(&mut self, text: &str) {
        self.protected = text.to_string();
    }

    /// Meta update only: replace the optimizer-private text.
    /// `pub(crate)` so nothing else can reach it.
    pub(crate) fn set_meta(&mut self, text: &str) {
        self.meta = text.to_string();
    }

    /// Apply one edit to the body. Never touches the protected section:
    /// any edit whose span/anchor is protected, or whose payload forges
    /// the markers, is rejected with [`EditError::ProtectedRegion`].
    pub fn apply(&mut self, edit: &Edit) -> Result<(), EditError> {
        // Per-edit accounting bound first (task-113): the paper's unit
        // is tokens; a newline-joined smuggle of several logical edits
        // is still one payload and is measured as one.
        if edit.tokens() > PER_EDIT_TOKENS_MAX {
            return Err(EditError::PayloadTooLarge {
                tokens: edit.tokens(),
            });
        }
        if edit.chars() > PER_EDIT_CHARS_MAX {
            return Err(EditError::TooLarge {
                chars: edit.chars(),
            });
        }
        // Marker forgery: step edits may not introduce the markers.
        let payloads: Vec<(&str, &str)> = match &edit.op {
            EditOp::Append { line } => vec![("append line", line)],
            EditOp::InsertAfter { anchor, line } => {
                vec![("insert_after anchor", anchor), ("insert_after line", line)]
            }
            EditOp::Replace { old, new } => vec![("replace old", old), ("replace new", new)],
            EditOp::Delete { line } => vec![("delete line", line)],
        };
        for (_what, payload) in &payloads {
            if payload.contains(SLOW_UPDATE_START) || payload.contains(SLOW_UPDATE_END) {
                return Err(EditError::ProtectedRegion);
            }
        }
        // One edit is one line (task-113): a newline-joined bundle of
        // several logical edits in a single payload is an edit-budget
        // evasion, rejected before it can dodge the per-step op count.
        for (what, payload) in &payloads {
            let lines = payload.lines().count();
            if lines > 1 {
                return Err(EditError::MultiLinePayload {
                    what: what.to_string(),
                    lines,
                });
            }
        }
        // Span/anchor inside the protected section: reject.
        let protected_lines: Vec<&str> = self.protected.lines().collect();
        let touches_protected = |span: &str| {
            span.lines()
                .any(|sl| protected_lines.iter().any(|pl| pl == &sl))
        };
        match &edit.op {
            EditOp::Append { line } => {
                if touches_protected(line) {
                    return Err(EditError::ProtectedRegion);
                }
                if !self.body.is_empty() && !self.body.ends_with('\n') {
                    self.body.push('\n');
                }
                self.body.push_str(line);
                self.body.push('\n');
                Ok(())
            }
            EditOp::InsertAfter { anchor, line } => {
                if touches_protected(anchor) {
                    return Err(EditError::ProtectedRegion);
                }
                // Empty-anchor game: matching "nothing" is not a location.
                if anchor.is_empty() {
                    return Err(EditError::EmptySpan {
                        what: "insert_after anchor".to_string(),
                    });
                }
                let mut lines: Vec<String> = self.body.lines().map(str::to_string).collect();
                let matches = lines.iter().filter(|l| *l == anchor).count();
                if matches == 0 {
                    return Err(EditError::NotFound {
                        what: truncate(anchor, 80),
                    });
                }
                // Duplicate-anchor game: refuse rather than silently
                // taking the first match (task-113).
                if matches > 1 {
                    return Err(EditError::AnchorAmbiguous {
                        anchor: truncate(anchor, 80),
                        matches,
                    });
                }
                let pos = lines
                    .iter()
                    .position(|l| l == anchor)
                    .expect("anchor counted exactly once above");
                lines.insert(pos + 1, line.clone());
                self.body = join_lines(&lines);
                Ok(())
            }
            EditOp::Replace { old, new } => {
                if touches_protected(old) {
                    return Err(EditError::ProtectedRegion);
                }
                // Empty-span game: replacing "nothing" is not an edit.
                if old.is_empty() {
                    return Err(EditError::EmptySpan {
                        what: "replace old span".to_string(),
                    });
                }
                let lines: Vec<String> = self.body.lines().map(str::to_string).collect();
                let old_lines: Vec<&str> = old.lines().collect();
                let pos = find_span(&lines, &old_lines).ok_or_else(|| EditError::NotFound {
                    what: truncate(old, 80),
                })?;
                let mut out = lines;
                out.splice(pos..pos + old_lines.len(), new.lines().map(str::to_string));
                self.body = join_lines(&out);
                Ok(())
            }
            EditOp::Delete { line } => {
                if touches_protected(line) {
                    return Err(EditError::ProtectedRegion);
                }
                let lines: Vec<String> = self.body.lines().map(str::to_string).collect();
                let span: Vec<&str> = line.lines().collect();
                let pos = find_span(&lines, &span).ok_or_else(|| EditError::NotFound {
                    what: truncate(line, 80),
                })?;
                let mut out = lines;
                out.drain(pos..pos + span.len());
                self.body = join_lines(&out);
                Ok(())
            }
        }
    }

    /// Apply several edits in order; stops at the first failure.
    pub fn apply_all(&mut self, edits: &[Edit]) -> Result<(), EditError> {
        for edit in edits {
            self.apply(edit)?;
        }
        Ok(())
    }
}

/// Find the first contiguous run of `lines` equal to `span`.
fn find_span(lines: &[String], span: &[&str]) -> Option<usize> {
    if span.is_empty() || span.len() > lines.len() {
        return None;
    }
    (0..=lines.len() - span.len())
        .find(|&i| span.iter().enumerate().all(|(k, s)| lines[i + k] == *s))
}

/// Join lines with `\n`, keeping a trailing newline when non-empty.
fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Truncate a string to `max` chars for error context (char-safe: never
/// splits a UTF-8 sequence).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, EditOp, PER_EDIT_CHARS_MAX, SLOW_UPDATE_END, SLOW_UPDATE_START, SkillDoc};

    fn doc() -> SkillDoc {
        let mut d = SkillDoc::experiment("line one\nline two");
        d.set_protected("KEEP: nothing yet");
        d
    }

    /// Task-114: the protected-region structure is exact. The render
    /// places the protected section between the markers, after the
    /// body; the body and protected accessors return their own
    /// sections, never each other's.
    #[test]
    fn protected_region_structure() {
        let d = doc();
        let rendered = d.render_for_target();
        let start = rendered.find(SLOW_UPDATE_START).unwrap();
        let end = rendered.find(SLOW_UPDATE_END).unwrap();
        assert!(start < end);
        // Body lines are before the start marker.
        assert!(rendered[..start].contains("line one"));
        // Protected lines are between the markers.
        assert!(rendered[start..end].contains("KEEP: nothing yet"));
        // After the end marker: nothing (no trailing protected leak).
        assert!(!rendered[end..].contains("KEEP: nothing yet"));
        // Accessors are disjoint.
        assert!(!d.body().contains("KEEP: nothing yet"));
        assert!(!d.protected().contains("line one"));
    }

    /// Validation: append/replace/delete/insert-after happy paths.
    #[test]
    fn edit_ops_happy_paths() {
        let mut d = doc();
        d.apply(&Edit {
            op: EditOp::Append {
                line: "line three".into(),
            },
            rationale: "t".into(),
            direction: "append".into(),
        })
        .unwrap();
        assert!(d.has_line("line three"));
        d.apply(&Edit {
            op: EditOp::Replace {
                old: "line two".into(),
                new: "line TWO".into(),
            },
            rationale: "t".into(),
            direction: "replace".into(),
        })
        .unwrap();
        assert!(d.has_line("line TWO") && !d.has_line("line two"));
        d.apply(&Edit {
            op: EditOp::Delete {
                line: "line one".into(),
            },
            rationale: "t".into(),
            direction: "delete".into(),
        })
        .unwrap();
        assert!(!d.has_line("line one"));
        d.apply(&Edit {
            op: EditOp::InsertAfter {
                anchor: "line TWO".into(),
                line: "line 2.5".into(),
            },
            rationale: "t".into(),
            direction: "insert".into(),
        })
        .unwrap();
        let body: Vec<&str> = d.body().lines().collect();
        assert_eq!(body, ["line TWO", "line 2.5", "line three"]);
    }

    /// Validation: target sees body + protected, never meta.
    #[test]
    fn target_view_excludes_meta() {
        let mut d = doc();
        d.set_meta("append n=3 mean=+0.10 var=0.01");
        let target_view = d.render_for_target();
        let opt_view = d.render_for_optimizer();
        assert!(target_view.contains("KEEP: nothing yet"));
        assert!(
            !target_view.contains("append n=3"),
            "target must never see meta"
        );
        assert!(opt_view.contains("append n=3"), "optimizer sees meta");
    }

    /// Validation: exactly-400-char edit passes, 401 fails.
    #[test]
    fn per_edit_cap_boundary() {
        let mut d = doc();
        let ok_line = "x".repeat(PER_EDIT_CHARS_MAX);
        let big_line = "x".repeat(PER_EDIT_CHARS_MAX + 1);
        assert!(
            d.apply(&Edit {
                op: EditOp::Append { line: ok_line },
                rationale: "t".into(),
                direction: "cap-ok".into(),
            })
            .is_ok()
        );
        assert!(
            d.apply(&Edit {
                op: EditOp::Append { line: big_line },
                rationale: "t".into(),
                direction: "cap-big".into(),
            })
            .is_err()
        );
    }

    /// Adversarial: step edits touching the protected region are refused.
    #[test]
    fn protected_region_refused() {
        let mut d = doc();
        for op in [
            EditOp::Append {
                line: SLOW_UPDATE_START.into(),
            },
            EditOp::Delete {
                line: "KEEP: nothing yet".into(),
            },
            EditOp::Replace {
                old: "line one".into(),
                new: format!("{SLOW_UPDATE_END} hijack"),
            },
        ] {
            assert!(
                d.apply(&Edit {
                    op,
                    rationale: "evil".into(),
                    direction: "evil".into(),
                })
                .is_err(),
                "protected-region edit must be refused"
            );
        }
        assert!(d.has_line("KEEP: nothing yet"), "protected section intact");
    }

    /// Adversarial: replace/delete of an absent line fails loudly.
    #[test]
    fn absent_line_fails() {
        let mut d = doc();
        let r = d.apply(&Edit {
            op: EditOp::Delete {
                line: "no such line".into(),
            },
            rationale: "t".into(),
            direction: "ghost".into(),
        });
        assert!(matches!(r, Err(super::EditError::NotFound { .. })));
    }

    /// Adversarial: untrusted imports are labeled and stay untrusted.
    #[test]
    fn untrusted_import_labeled() {
        let d = SkillDoc::import_untrusted("whatever");
        assert_eq!(d.provenance(), super::Provenance::Untrusted);
        assert_ne!(d.provenance(), super::Provenance::Experiment);
    }
}
