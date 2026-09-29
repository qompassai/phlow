//! The single typed error for every rejected input or refused transition.

use std::fmt;

use phlow_json::JsonError;

use crate::queue::State;

/// Characters of an attacker-supplied field name kept for error context.
const FIELD_NAME_CHARS_MAX: usize = 64;

/// Why an input was rejected or a transition refused.
///
/// Every variant carries bounded context only; rejected work leaves all
/// published state unchanged unless the variant says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The JSON shape was wrong: missing field, wrong type, not an object.
    Json(JsonError),
    /// An object carried a field outside its closed schema. Unknown fields
    /// are never silently discarded.
    UnknownField { object: &'static str, field: String },
    /// The policy `version` is not one this crate implements.
    UnsupportedVersion { version: u64 },
    /// A field had the right type but an unacceptable value.
    InvalidValue {
        field: &'static str,
        reason: &'static str,
    },
    /// A list or log exceeded its named limit.
    TooMany { field: &'static str, max: usize },
    /// No record with this ID was issued by this queue.
    UnknownId,
    /// The actor is missing, not a configured operator, or reserved.
    ActorRefused,
    /// The record is not in the state this transition requires.
    WrongState { state: State },
    /// The request passed its deadline; the record is now `Expired`.
    Expired,
}

impl Error {
    /// Build [`Error::UnknownField`] with the field name truncated.
    pub(crate) fn unknown_field(object: &'static str, field: &str) -> Self {
        Error::UnknownField {
            object,
            field: field.chars().take(FIELD_NAME_CHARS_MAX).collect(),
        }
    }
}

impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        Error::Json(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(error) => write!(f, "{error}"),
            Error::UnknownField { object, field } => {
                write!(f, "unknown field {field:?} in {object}")
            }
            Error::UnsupportedVersion { version } => {
                write!(f, "unsupported policy version {version}")
            }
            Error::InvalidValue { field, reason } => write!(f, "invalid {field}: {reason}"),
            Error::TooMany { field, max } => write!(f, "{field} exceeds {max} entries"),
            Error::UnknownId => write!(f, "unknown approval id"),
            Error::ActorRefused => write!(f, "actor is not a configured human operator"),
            Error::WrongState { state } => write!(f, "record is {}", state.as_str()),
            Error::Expired => write!(f, "request expired before a decision"),
        }
    }
}

impl std::error::Error for Error {}
