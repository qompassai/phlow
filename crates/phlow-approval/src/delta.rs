//! Permission sets and the deterministic delta between two of them.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::error::Error;
use crate::scope::{check_plain, string_set};

/// Maximum raw entries in one permission list (before deduplication).
pub const PERMISSIONS_MAX: usize = 4096;

/// A validated, bounded set of permission identities.
///
/// Identity is byte-exact: `read` and `READ` are different permissions, and
/// `fs.read:/work` is not `fs.read:/work/a`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    members: BTreeSet<String>,
}

impl PermissionSet {
    /// Members in ascending byte order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(String::as_str)
    }

    /// Number of distinct members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// True when the set has no members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub(crate) fn from_field(map: &Map<String, Value>, name: &'static str) -> Result<Self, Error> {
        Ok(PermissionSet {
            members: string_set(map, name, PERMISSIONS_MAX, check_plain)?,
        })
    }
}

/// Set difference between a before and an after permission set.
///
/// Both lists are sorted ascending and duplicate-free. The delta is computed
/// only from the two sets; no summary or claimed delta can influence it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionDelta {
    /// In `after` but not in `before`.
    pub added: Vec<String>,
    /// In `before` but not in `after`.
    pub removed: Vec<String>,
}

impl PermissionDelta {
    /// Compute `after - before` and `before - after`.
    pub fn between(before: &PermissionSet, after: &PermissionSet) -> Self {
        PermissionDelta {
            added: after.members.difference(&before.members).cloned().collect(),
            removed: before.members.difference(&after.members).cloned().collect(),
        }
    }

    /// Canonical encoding: `{"added":[...],"removed":[...]}` in that key
    /// order, so identical deltas serialize to identical bytes.
    pub fn to_json(&self) -> Value {
        let strings =
            |items: &[String]| Value::Array(items.iter().cloned().map(Value::String).collect());
        let mut map = Map::with_capacity(2);
        map.insert("added".to_owned(), strings(&self.added));
        map.insert("removed".to_owned(), strings(&self.removed));
        Value::Object(map)
    }
}
