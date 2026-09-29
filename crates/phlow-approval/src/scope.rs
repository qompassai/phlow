//! What a tool call touches ([`Scope`]) and admission of untrusted requests.

use std::collections::BTreeSet;

use phlow_json::{JsonError, object_map, opt_str, req_str};
use serde_json::{Map, Value};

use crate::delta::PermissionSet;
use crate::error::Error;

/// Maximum bytes of one tool name, path, endpoint, permission, actor or run ID.
pub const TEXT_BYTES_MAX: usize = 256;
/// Maximum bytes of a model-authored request summary.
pub const SUMMARY_BYTES_MAX: usize = 4096;
/// Maximum entries in one scope list (paths, endpoints, rule tools).
pub const SCOPE_ITEMS_MAX: usize = 256;

/// The closed request schema. `summary` is the only model-authored field
/// admitted; claimed deltas, approval flags and extra authority are rejected.
const REQUEST_FIELDS: [&str; 7] = [
    "tool",
    "risk",
    "paths",
    "endpoints",
    "permissions_before",
    "permissions_after",
    "summary",
];

/// Risk class of a tool call. Unknown classes are rejected, never defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Observe,
    LocalReversible,
    Process,
    Network,
    Irreversible,
}

impl Risk {
    /// Parse the wire name of a risk class.
    pub fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "observe" => Ok(Risk::Observe),
            "local_reversible" => Ok(Risk::LocalReversible),
            "process" => Ok(Risk::Process),
            "network" => Ok(Risk::Network),
            "irreversible" => Ok(Risk::Irreversible),
            _ => Err(Error::InvalidValue {
                field: "risk",
                reason: "unknown risk class",
            }),
        }
    }

    /// The wire name of this risk class.
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Observe => "observe",
            Risk::LocalReversible => "local_reversible",
            Risk::Process => "process",
            Risk::Network => "network",
            Risk::Irreversible => "irreversible",
        }
    }
}

/// One tool, one risk class, and the exact resources it touches.
///
/// Paths and endpoints are sorted sets compared byte-exactly; no glob or
/// prefix semantics exist. Paths must be lexically canonical absolute paths,
/// so `/work/./a` cannot alias `/work/a` around a deny rule. Endpoints are
/// not canonicalized: endpoint deny rules are only sound under a deny default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub(crate) tool: String,
    pub(crate) risk: Risk,
    pub(crate) paths: Vec<String>,
    pub(crate) endpoints: Vec<String>,
}

impl Scope {
    /// The single tool this scope covers.
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The declared risk class.
    pub fn risk(&self) -> Risk {
        self.risk
    }

    /// Requested paths, sorted and deduplicated.
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Requested endpoints, sorted and deduplicated.
    pub fn endpoints(&self) -> &[String] {
        &self.endpoints
    }

    fn from_map(map: &Map<String, Value>) -> Result<Self, Error> {
        let tool = req_str(map, "tool")?;
        check_plain("tool", tool)?;
        Ok(Scope {
            tool: tool.to_owned(),
            risk: Risk::parse(req_str(map, "risk")?)?,
            paths: string_set(map, "paths", SCOPE_ITEMS_MAX, check_path)?
                .into_iter()
                .collect(),
            endpoints: string_set(map, "endpoints", SCOPE_ITEMS_MAX, check_plain)?
                .into_iter()
                .collect(),
        })
    }
}

/// A validated proposal: a scope plus the permission sets before and after.
///
/// Built only by [`Request::from_json`] (untrusted input) or
/// [`crate::Decision::proposal`] (a denial narrowed to its exact scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub(crate) scope: Scope,
    pub(crate) permissions_before: PermissionSet,
    pub(crate) permissions_after: PermissionSet,
    pub(crate) summary: Option<String>,
}

impl Request {
    /// Admit an untrusted JSON request against the closed request schema.
    ///
    /// Rejects unknown fields (including `permissions`, `tools`,
    /// `permission_delta`, `approved`, `state`), unknown risk classes,
    /// non-canonical paths, non-string or map-shaped sets, and anything over
    /// the named limits. The value is copied; later caller edits do not reach
    /// the returned request.
    pub fn from_json(value: &Value) -> Result<Self, Error> {
        let map = object_map(value)?;
        reject_unknown(map, &REQUEST_FIELDS, "request")?;
        let summary = opt_str(map, "summary")?;
        if let Some(text) = summary {
            check_summary(text)?;
        }
        Ok(Request {
            scope: Scope::from_map(map)?,
            permissions_before: PermissionSet::from_field(map, "permissions_before")?,
            permissions_after: PermissionSet::from_field(map, "permissions_after")?,
            summary: summary.map(str::to_owned),
        })
    }

    /// A proposal for exactly `scope`, with no permission change or summary.
    pub fn from_scope(scope: Scope) -> Self {
        Request {
            scope,
            permissions_before: PermissionSet::default(),
            permissions_after: PermissionSet::default(),
            summary: None,
        }
    }

    /// The proposed scope.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

/// Reject any key of `map` outside `allowed`.
pub(crate) fn reject_unknown(
    map: &Map<String, Value>,
    allowed: &[&str],
    object: &'static str,
) -> Result<(), Error> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(Error::unknown_field(object, key)),
        None => Ok(()),
    }
}

/// Non-empty, bounded, free of control characters.
pub(crate) fn check_plain(field: &'static str, text: &str) -> Result<(), Error> {
    if text.is_empty() {
        return Err(Error::InvalidValue {
            field,
            reason: "empty",
        });
    }
    if text.len() > TEXT_BYTES_MAX {
        return Err(Error::InvalidValue {
            field,
            reason: "too long",
        });
    }
    if text.chars().any(char::is_control) {
        return Err(Error::InvalidValue {
            field,
            reason: "control character",
        });
    }
    Ok(())
}

/// A plain value that is also an absolute path with no empty, `.` or `..`
/// component and no trailing slash. Symlinks are not resolved here: paths are
/// requests, and containment stays with the runtime's file operations.
pub(crate) fn check_path(field: &'static str, path: &str) -> Result<(), Error> {
    check_plain(field, path)?;
    let Some(rest) = path.strip_prefix('/') else {
        return Err(Error::InvalidValue {
            field,
            reason: "path is not absolute",
        });
    };
    let canonical = rest.is_empty() || rest.split('/').all(|c| !matches!(c, "" | "." | ".."));
    if !canonical {
        return Err(Error::InvalidValue {
            field,
            reason: "path is not lexically canonical",
        });
    }
    Ok(())
}

/// Summaries may span lines but carry no other control characters, so a
/// model cannot smuggle terminal escapes onto an approval surface.
fn check_summary(text: &str) -> Result<(), Error> {
    if text.len() > SUMMARY_BYTES_MAX {
        return Err(Error::InvalidValue {
            field: "summary",
            reason: "too long",
        });
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(Error::InvalidValue {
            field: "summary",
            reason: "control character",
        });
    }
    Ok(())
}

/// Read an optional array of strings as a sorted set; absent is empty.
///
/// The raw length is checked before any member is copied. Duplicates
/// collapse (set semantics); `null`, booleans, numbers and nested values are
/// rejected, as is an object standing in for an array (a sparse table).
pub(crate) fn string_set(
    map: &Map<String, Value>,
    name: &'static str,
    items_max: usize,
    check: fn(&'static str, &str) -> Result<(), Error>,
) -> Result<BTreeSet<String>, Error> {
    let Some(value) = map.get(name) else {
        return Ok(BTreeSet::new());
    };
    let Value::Array(items) = value else {
        return Err(Error::Json(JsonError::UnexpectedType {
            field: name.to_owned(),
            expected: "an array",
        }));
    };
    if items.len() > items_max {
        return Err(Error::TooMany {
            field: name,
            max: items_max,
        });
    }
    let mut set = BTreeSet::new();
    for item in items {
        let Value::String(text) = item else {
            return Err(Error::InvalidValue {
                field: name,
                reason: "member is not a string",
            });
        };
        check(name, text)?;
        set.insert(text.clone());
    }
    Ok(set)
}
