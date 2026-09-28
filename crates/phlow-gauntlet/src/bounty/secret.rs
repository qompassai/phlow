//! Secret hygiene: the vault holds real credentials in memory only
//! (never serialized), and `redact_text` scrubs every text surface at
//! write time.

/// In-memory-only credential vault. Deliberately has no `Serialize`
/// impl and its `Debug` impl prints only the entry count, never
/// values: Debug output routinely lands in logs. The only ways out
/// are per-key `get` for the authenticated submit call, and
/// `secret_values`, which clones values solely to feed the
/// redaction list that scrubs text surfaces.
#[derive(Default)]
pub struct SecretVault {
    secrets: Vec<(String, String)>,
}

impl std::fmt::Debug for SecretVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretVault")
            .field("entries", &self.secrets.len())
            .finish()
    }
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
