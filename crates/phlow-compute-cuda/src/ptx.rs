//! Validated PTX module text.
//!
//! A [`PtxModule`] is a structural check, not an assembler: the text must
//! fit the byte bound and carry a `.version` directive inside the supported
//! ISA range plus a `.target` directive. Entry-point discovery scans for
//! `.entry` declarations with a small documented heuristic.

use crate::arch::PtxVersion;
use crate::error::CudaError;

/// Largest PTX source text accepted, in bytes: 1 MiB.
///
/// Real kernels are kilobytes; a megabyte already signals generated or
/// hostile input.
pub const PTX_BYTES_MAX: usize = 1 << 20;

/// Most entry points enumerated from one module.
///
/// Bounds [`PtxModule::entry_names`] so a hostile file cannot force an
/// unbounded allocation through thousands of fake declarations.
pub const PTX_ENTRY_COUNT_MAX: usize = 1024;

/// Longest entry name reported, in characters.
const ENTRY_NAME_CHARS_MAX: usize = 256;

/// Validated PTX module text.
///
/// The invariant: `text.len() <= PTX_BYTES_MAX`, the text carries a `.version`
/// directive in the supported range and a `.target` directive. Only
/// [`PtxModule::from_text`] constructs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxModule {
    text: String,
    version: PtxVersion,
    target: String,
}

impl PtxModule {
    /// Validates PTX source text.
    ///
    /// # Contract
    /// - Accepts: text within [`PTX_BYTES_MAX`] bytes carrying a `.version`
    ///   directive in range and a `.target` directive. Directives are
    ///   recognized at line starts after trimming ASCII whitespace; `//`
    ///   comments are skipped.
    /// - Rejects: oversized text, a missing or malformed `.version`, an
    ///   out-of-range version, or a missing `.target` — with a typed error.
    ///   The text itself never appears in the error.
    pub fn from_text(text: &str) -> Result<Self, CudaError> {
        if text.len() > PTX_BYTES_MAX {
            return Err(CudaError::PtxTooLarge { bytes: text.len() });
        }
        let mut version: Option<PtxVersion> = None;
        let mut target: Option<String> = None;
        for line in text.lines() {
            let stripped = strip_comment(line).trim_start();
            // First directive wins; later ones are not even parsed.
            if version.is_none()
                && let Some(payload) = stripped.strip_prefix(".version")
            {
                version = Some(PtxVersion::parse(payload.trim())?);
            } else if target.is_none()
                && let Some(payload) = stripped.strip_prefix(".target")
            {
                let name = payload.trim();
                if !name.is_empty() {
                    target = Some(name.to_string());
                }
            }
            if version.is_some() && target.is_some() {
                break;
            }
        }
        let version = version.ok_or(CudaError::PtxMissingVersion)?;
        let target = target.ok_or(CudaError::PtxMissingTarget)?;
        Ok(Self {
            text: text.to_string(),
            version,
            target,
        })
    }

    /// The validated module text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The ISA version from the `.version` directive.
    pub fn version(&self) -> PtxVersion {
        self.version
    }

    /// The `.target` directive payload, e.g. `sm_80`.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Whether the module declares an entry point named `name`.
    ///
    /// The name is validated first (charset and length); then lines are
    /// scanned for `.entry <name>(` or `.visible .entry <name>(` after
    /// comment stripping. This is a heuristic for descriptor tooling, not a
    /// substitute for the driver's symbol lookup.
    pub fn has_entry(&self, name: &str) -> Result<bool, CudaError> {
        validate_entry_name(name)?;
        Ok(self.entry_names().iter().any(|found| found == name))
    }

    /// Every entry-point name declared in the module, in source order.
    ///
    /// At most [`PTX_ENTRY_COUNT_MAX`] names are collected; further
    /// declarations are ignored so a hostile file cannot grow the vector
    /// without bound.
    pub fn entry_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for line in self.text.lines() {
            if names.len() >= PTX_ENTRY_COUNT_MAX {
                break;
            }
            let stripped = strip_comment(line).trim_start();
            let rest = stripped
                .strip_prefix(".visible .entry")
                .or_else(|| stripped.strip_prefix(".entry"));
            let Some(rest) = rest else { continue };
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '$')
                .take(ENTRY_NAME_CHARS_MAX)
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
        names
    }
}

/// Validates an entry-point name: 1..=256 bytes of
/// `[A-Za-z0-9_.$]`, starting with a letter, `_`, `.`, or `$`.
fn validate_entry_name(name: &str) -> Result<(), CudaError> {
    let bytes = name.len();
    if bytes == 0 || bytes > ENTRY_NAME_CHARS_MAX {
        return Err(CudaError::invalid_name(name));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or(' ');
    let first_ok = first.is_ascii_alphabetic() || first == '_' || first == '.' || first == '$';
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$');
    if !first_ok || !rest_ok {
        return Err(CudaError::invalid_name(name));
    }
    Ok(())
}

/// Removes a trailing `//` comment. PTX has no nested or block comments in
/// the subset this scanner reads; string literals cannot appear in the
/// directive lines being scanned.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}
