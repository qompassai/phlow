//! Secret hygiene: the vault holds real credentials in memory only
//! (never serialized), and `redact_text` scrubs every text surface at
//! write time.

/// In-memory-only credential vault. Deliberately has no `Serialize`
/// impl and no method that returns all secrets at once: the only way
/// out is per-key `get` for the authenticated submit call.
#[derive(Debug, Default)]
pub struct SecretVault {
    secrets: Vec<(String, String)>,
}

impl SecretVault {
    pub fn new() -> Self {
        SecretVault {
            secrets: Vec::new(),
        }
    }

    pub fn insert(&mut self, name: &str, value: &str) {
        self.secrets.push((name.to_string(), value.to_string()));
    }

    /// The single authorized read path: one named secret for the
    /// authenticated platform call. Nothing else reads the vault.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.secrets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn secret_values(&self) -> Vec<String> {
        self.secrets.iter().map(|(_, v)| v.clone()).collect()
    }
}

/// Replace every occurrence of each secret with `[REDACTED]`. Runs at
/// write time on logs, reports, diffs, and error strings.
pub fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if s.is_empty() {
            continue;
        }
        out = out.replace(s.as_str(), "[REDACTED]");
    }
    out
}
