//! Versioned, fail-closed policy parsing and scope decisions.

use std::collections::BTreeSet;

use phlow_json::{JsonError, object_map, opt_str, req_str, req_u64};
use serde_json::Value;

use crate::error::Error;
use crate::scope::{
    Request, Risk, SCOPE_ITEMS_MAX, Scope, check_path, check_plain, reject_unknown, string_set,
};

/// The only policy schema version this crate implements.
pub const POLICY_VERSION: u64 = 1;
/// Maximum rules in one policy.
pub const RULES_MAX: usize = 256;

const POLICY_FIELDS: [&str; 3] = ["version", "default", "rules"];
const RULE_FIELDS: [&str; 5] = ["risk", "decision", "tools", "paths", "endpoints"];

/// Outcome of a policy decision, ordered from least to most restrictive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// The scope may run without a human decision.
    Allow,
    /// The scope needs an approved request from a human operator.
    Approval,
    /// The scope must not run.
    Deny,
}

impl Verdict {
    fn parse(field: &'static str, text: &str) -> Result<Self, Error> {
        match text {
            "allow" => Ok(Verdict::Allow),
            "approval" => Ok(Verdict::Approval),
            "deny" => Ok(Verdict::Deny),
            _ => Err(Error::InvalidValue {
                field,
                reason: "decision must be allow, approval or deny",
            }),
        }
    }

    /// The wire name of this verdict.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Approval => "approval",
            Verdict::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone)]
struct Rule {
    risk: Risk,
    verdict: Verdict,
    tools: BTreeSet<String>,
    paths: BTreeSet<String>,
    endpoints: BTreeSet<String>,
}

impl Rule {
    /// Allow and approval rules must cover every requested path and endpoint;
    /// a deny rule matches on any overlap, or on the whole tool when it names
    /// no resources. Separate rules never combine into a wider grant.
    fn matches(&self, scope: &Scope) -> bool {
        if self.risk != scope.risk || !self.tools.contains(&scope.tool) {
            return false;
        }
        match self.verdict {
            Verdict::Deny => {
                (self.paths.is_empty() && self.endpoints.is_empty())
                    || scope.paths.iter().any(|path| self.paths.contains(path))
                    || scope
                        .endpoints
                        .iter()
                        .any(|end| self.endpoints.contains(end))
            }
            Verdict::Allow | Verdict::Approval => {
                scope.paths.iter().all(|path| self.paths.contains(path))
                    && scope
                        .endpoints
                        .iter()
                        .all(|end| self.endpoints.contains(end))
            }
        }
    }
}

/// A validated policy. There is no mutating API: a policy changes only by
/// parsing a new one, and parsed state shares nothing with its input.
#[derive(Debug, Clone)]
pub struct Policy {
    version: u64,
    default: Verdict,
    rules: Vec<Rule>,
}

impl Policy {
    /// Parse an untrusted policy document.
    ///
    /// `version` is required and must equal [`POLICY_VERSION`]. `default`
    /// is optional (absent means deny). `rules` is optional but, if present,
    /// must be an array of rule objects; `false`, maps and sparse tables are
    /// rejected. Unknown fields at either level are rejected, including
    /// bypass flags such as `legacy` or `permissive`. Rules require `risk`,
    /// `decision` and a non-empty `tools`.
    pub fn from_json(value: &Value) -> Result<Self, Error> {
        let map = object_map(value)?;
        reject_unknown(map, &POLICY_FIELDS, "policy")?;
        let version = req_u64(map, "version")?;
        if version != POLICY_VERSION {
            return Err(Error::UnsupportedVersion { version });
        }
        let default = match opt_str(map, "default")? {
            Some(text) => Verdict::parse("default", text)?,
            None => Verdict::Deny,
        };
        let rules = match map.get("rules") {
            Some(rules) => parse_rules(rules)?,
            None => Vec::new(),
        };
        Ok(Policy {
            version,
            default,
            rules,
        })
    }

    /// The schema version this policy was parsed with.
    pub fn version(&self) -> u64 {
        self.version
    }
}

fn parse_rules(value: &Value) -> Result<Vec<Rule>, Error> {
    let Value::Array(items) = value else {
        return Err(Error::Json(JsonError::UnexpectedType {
            field: "rules".to_owned(),
            expected: "an array",
        }));
    };
    if items.len() > RULES_MAX {
        return Err(Error::TooMany {
            field: "rules",
            max: RULES_MAX,
        });
    }
    items.iter().map(parse_rule).collect()
}

fn parse_rule(value: &Value) -> Result<Rule, Error> {
    let map = object_map(value)?;
    reject_unknown(map, &RULE_FIELDS, "rule")?;
    let risk = Risk::parse(req_str(map, "risk")?)?;
    let verdict = Verdict::parse("decision", req_str(map, "decision")?)?;
    let tools = string_set(map, "tools", SCOPE_ITEMS_MAX, check_plain)?;
    if tools.is_empty() {
        return Err(Error::InvalidValue {
            field: "tools",
            reason: "a rule must name at least one tool",
        });
    }
    Ok(Rule {
        risk,
        verdict,
        tools,
        paths: string_set(map, "paths", SCOPE_ITEMS_MAX, check_path)?,
        endpoints: string_set(map, "endpoints", SCOPE_ITEMS_MAX, check_plain)?,
    })
}

/// A policy decision that retains the exact scope it judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub verdict: Verdict,
    /// Fixed, non-empty explanation; never model-authored.
    pub reason: &'static str,
    /// The judged tool, risk, paths and endpoints, unchanged.
    pub scope: Scope,
}

impl Decision {
    /// A proposal for exactly the judged scope and nothing more: no extra
    /// tools, resources, permissions or summary.
    pub fn proposal(&self) -> Request {
        Request::from_scope(self.scope.clone())
    }
}

/// Decide `scope` under `policy`. A missing policy (for example after a
/// parse failure) denies everything.
///
/// The most restrictive matching rule wins regardless of rule order
/// (deny > approval > allow); with no match the policy default applies.
pub fn decide(policy: Option<&Policy>, scope: &Scope) -> Decision {
    let decision = |verdict, reason| Decision {
        verdict,
        reason,
        scope: scope.clone(),
    };
    let Some(policy) = policy else {
        return decision(Verdict::Deny, "no valid policy is loaded");
    };
    let strongest = policy
        .rules
        .iter()
        .filter(|rule| rule.matches(scope))
        .map(|rule| rule.verdict)
        .max();
    match strongest {
        Some(Verdict::Deny) => decision(Verdict::Deny, "a deny rule matches the scope"),
        Some(Verdict::Approval) => decision(Verdict::Approval, "an approval rule covers the scope"),
        Some(Verdict::Allow) => decision(Verdict::Allow, "an allow rule covers the full scope"),
        None => decision(
            policy.default,
            "no rule covers the full scope; policy default applies",
        ),
    }
}
