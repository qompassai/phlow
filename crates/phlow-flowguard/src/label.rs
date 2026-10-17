//! The label algebra: [`Label`] = audience × trust, the join, and the
//! two ways a label contribution is written down (a policy [`LabelDelta`]
//! in audience tokens, or a concrete [`LabelSnapshot`] carried across a
//! handoff).
//!
//! Plain words: an audience is a set of readers. `self ⊆ internal ⊆
//! public`; `@group` names a declared set of readers; any other string
//! is a literal reader id. Comparison is always by *expanded reader
//! set*, never by the surface tokens, so `@finance` and a finance
//! member's own id compare correctly. Trust is a rank in the policy's
//! chain. The only combination operation is the **join**: intersect
//! the reader sets, take the lower trust. Both components only shrink,
//! which is exactly monotonicity — and because the join is associative
//! and idempotent, the fold of a log is order-insensitive per event
//! kind sequence and replay-stable.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// An expanded set of readers.
///
/// `All` is the top element (the `public` audience): every reader is
/// in it. `Only` is an explicit finite set. Membership and subset
/// checks are total and exact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReaderSet {
    /// Every reader (the expansion of `public`).
    All,
    /// An explicit, finite reader set.
    Only(BTreeSet<String>),
}

impl ReaderSet {
    /// The empty reader set: no one may receive.
    pub fn empty() -> Self {
        ReaderSet::Only(BTreeSet::new())
    }

    /// Is `reader` in this set?
    pub fn contains(&self, reader: &str) -> bool {
        match self {
            ReaderSet::All => true,
            ReaderSet::Only(set) => set.contains(reader),
        }
    }

    /// Is every member of `other` also a member of `self`?
    ///
    /// `self ⊇ other` — the direction the gate checks: the data's
    /// readers must cover the destination's required readers.
    pub fn is_superset_of(&self, other: &ReaderSet) -> bool {
        match (self, other) {
            (ReaderSet::All, _) => true,
            (_, ReaderSet::All) => false,
            (ReaderSet::Only(mine), ReaderSet::Only(theirs)) => {
                theirs.iter().all(|reader| mine.contains(reader))
            }
        }
    }

    /// Set intersection. The audience half of the join.
    pub fn intersect(&self, other: &ReaderSet) -> ReaderSet {
        match (self, other) {
            (ReaderSet::All, _) => other.clone(),
            (_, ReaderSet::All) => self.clone(),
            (ReaderSet::Only(mine), ReaderSet::Only(theirs)) => {
                ReaderSet::Only(mine.intersection(theirs).cloned().collect())
            }
        }
    }

    /// Set union (used when combining several audience tokens into
    /// one expansion, never to combine labels).
    pub fn union(&self, other: &ReaderSet) -> ReaderSet {
        match (self, other) {
            (ReaderSet::All, _) | (_, ReaderSet::All) => ReaderSet::All,
            (ReaderSet::Only(mine), ReaderSet::Only(theirs)) => {
                ReaderSet::Only(mine.union(theirs).cloned().collect())
            }
        }
    }

    /// Sorted member list for diagnostics; `None` for `All`.
    pub fn members(&self) -> Option<Vec<String>> {
        match self {
            ReaderSet::All => None,
            ReaderSet::Only(set) => Some(set.iter().cloned().collect()),
        }
    }
}

/// A security label: the expanded reader set that may receive the
/// trajectory's material, and the trust rank that material carries.
///
/// `trust` is an index into the owning policy's trust chain
/// (0 = least trusted). Labels are only ever combined with
/// [`Label::join`]; there is deliberately no setter, widen, or
/// "reset" operation anywhere in this crate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Label {
    /// Who may receive material labelled this way.
    pub readers: ReaderSet,
    /// Rank index into the policy trust chain.
    pub trust: usize,
}

impl Label {
    /// The least restrictive label for a chain of `chain_len` ranks:
    /// everyone may read, highest trust.
    pub fn top(chain_len: usize) -> Self {
        Label {
            readers: ReaderSet::All,
            trust: chain_len.saturating_sub(1),
        }
    }

    /// The join (⊔): intersect reader sets, take the lower trust.
    ///
    /// This is the *only* way labels combine, and it can only make a
    /// label more restrictive — the mechanism behind the
    /// no-downgrade rule.
    pub fn join(&self, other: &Label) -> Label {
        Label {
            readers: self.readers.intersect(&other.readers),
            trust: self.trust.min(other.trust),
        }
    }

    /// Is `self` at least as restrictive as `other`
    /// (readers ⊆, trust ≤)? Used to state spawn inheritance:
    /// a child's start label must satisfy `child ⊑ parent`.
    pub fn is_at_most(&self, other: &Label) -> bool {
        other.readers.is_superset_of(&self.readers) && self.trust <= other.trust
    }
}

/// A label contribution as written in policy: audience *tokens*
/// (`public`, `internal`, `self`, `@group`, literal reader ids) and
/// an optional trust rank *name*. Either field may be absent, meaning
/// "no constraint from this source".
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LabelDelta {
    /// Audience tokens; empty means no audience constraint.
    #[serde(default)]
    pub audience: Vec<String>,
    /// Trust rank name; `None` means no trust constraint.
    #[serde(default)]
    pub trust: Option<String>,
}

impl LabelDelta {
    /// A delta carrying only an audience constraint.
    pub fn audience(tokens: &[&str]) -> Self {
        LabelDelta {
            audience: tokens.iter().map(|token| (*token).to_string()).collect(),
            trust: None,
        }
    }

    /// A delta carrying only a trust constraint.
    pub fn trust(rank: &str) -> Self {
        LabelDelta {
            audience: Vec::new(),
            trust: Some(rank.to_string()),
        }
    }
}

/// A concrete label as carried across a handoff (in a sub-agent's
/// start event, or in a received message's join spec). The trust
/// rides as its rank *name* so the snapshot stays meaningful under
/// the receiving policy's chain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LabelSnapshot {
    /// The expanded readers at handoff time.
    pub readers: ReaderSet,
    /// The trust rank name at handoff time.
    pub trust: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(names: &[&str]) -> ReaderSet {
        ReaderSet::Only(names.iter().map(|name| (*name).to_string()).collect())
    }

    #[test]
    fn join_intersects_readers_and_takes_lower_trust() {
        let left = Label {
            readers: only(&["a@x", "b@x"]),
            trust: 1,
        };
        let right = Label {
            readers: only(&["b@x", "c@x"]),
            trust: 0,
        };
        let joined = left.join(&right);
        assert_eq!(joined.readers, only(&["b@x"]));
        assert_eq!(joined.trust, 0);
    }

    #[test]
    fn join_with_all_keeps_the_other_side() {
        let top = Label::top(2);
        let narrow = Label {
            readers: only(&["a@x"]),
            trust: 0,
        };
        assert_eq!(top.join(&narrow), narrow);
        assert_eq!(narrow.join(&top), narrow);
    }

    #[test]
    fn join_is_associative_and_idempotent() {
        let a = Label {
            readers: only(&["a@x", "b@x", "c@x"]),
            trust: 2,
        };
        let b = Label {
            readers: only(&["b@x", "c@x"]),
            trust: 1,
        };
        let c = Label {
            readers: only(&["c@x"]),
            trust: 0,
        };
        assert_eq!(a.join(&b).join(&c), a.join(&b.join(&c)));
        assert_eq!(a.join(&a), a);
    }

    #[test]
    fn superset_direction_is_destination_covering() {
        // The gate asks: does the data's audience cover the
        // destination's readers? `All` covers everything; an explicit
        // set never covers `All`.
        assert!(ReaderSet::All.is_superset_of(&only(&["a@x"])));
        assert!(only(&["a@x", "b@x"]).is_superset_of(&only(&["b@x"])));
        assert!(!only(&["a@x"]).is_superset_of(&ReaderSet::All));
        assert!(!only(&["a@x"]).is_superset_of(&only(&["b@x"])));
    }

    #[test]
    fn is_at_most_requires_both_components() {
        let parent = Label {
            readers: only(&["a@x", "b@x"]),
            trust: 1,
        };
        let child_ok = Label {
            readers: only(&["a@x"]),
            trust: 1,
        };
        let child_wider = Label {
            readers: only(&["a@x", "c@x"]),
            trust: 1,
        };
        let child_trustier = Label {
            readers: only(&["a@x"]),
            trust: 2,
        };
        assert!(child_ok.is_at_most(&parent));
        assert!(!child_wider.is_at_most(&parent));
        assert!(!child_trustier.is_at_most(&parent));
    }
}
