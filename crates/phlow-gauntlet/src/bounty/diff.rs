//! Scope diffing between two snapshots, keyed by target id.

use crate::bounty::types::{ScopeDiff, ScopeSnapshot};
use std::collections::{HashMap, HashSet};

/// Compute added / removed / changed between `old` and `new`.
/// A stable id with a changed value is `changed` (one pair), never an
/// add+remove. Runs in O(n) via id-indexed maps.
pub fn diff_scope(old: &ScopeSnapshot, new: &ScopeSnapshot) -> ScopeDiff {
    let old_by_id: HashMap<&str, _> = old.targets.iter().map(|t| (t.id.0.as_str(), t)).collect();
    let new_by_id: HashMap<&str, _> = new.targets.iter().map(|t| (t.id.0.as_str(), t)).collect();
    let old_ids: HashSet<&str> = old_by_id.keys().copied().collect();
    let new_ids: HashSet<&str> = new_by_id.keys().copied().collect();

    let mut diff = ScopeDiff::default();
    for id in new_ids.difference(&old_ids) {
        diff.added.push(new_by_id[id].clone());
    }
    for id in old_ids.difference(&new_ids) {
        diff.removed.push(old_by_id[id].clone());
    }
    for id in old_ids.intersection(&new_ids) {
        let o = old_by_id[id];
        let n = new_by_id[id];
        if o != n {
            diff.changed.push((o.clone(), n.clone()));
        }
    }
    diff.added.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    diff.removed.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    diff
}
