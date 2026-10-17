//! Small shared helpers: an RFC 3339 clock and boot-seeded
//! identifier derivation.
//!
//! Identifiers deserve an honest note: task and context ids in this
//! crate are `<kind>-<boot seed>-<counter>` strings. The boot seed
//! comes from the system clock hashed at store construction, so ids
//! are unguessable *in practice* across restarts — but per the
//! program SPEC they are **handles, not capabilities**: possession
//! of a task id is never treated as authorization. The server still
//! authenticates callers by bearer token when tokens are configured,
//! and the flow gate, not the id, decides what data may move.

use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn unix_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The current time as an RFC 3339 UTC string
/// (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn rfc3339_now() -> String {
    rfc3339_from_unix(unix_secs_now())
}

/// Format unix seconds as RFC 3339 UTC. The civil-from-days
/// conversion is Howard Hinnant's well-known algorithm; it is exact
/// for the full u64 range of days this crate will ever see.
pub fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let within = secs % 86_400;
    let (hour, minute, second) = (within / 3600, (within % 3600) / 60, within % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Derive a 64-bit boot seed from the current time and a caller
/// salt. Distinct stores (and server restarts) get distinct seeds.
pub fn boot_seed(salt: &str) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(
        digest[..8]
            .try_into()
            .expect("a sha256 digest always has 8 bytes"),
    )
}

/// Mint one identifier: `<kind>-<seed in hex>-<counter>`.
pub fn mint_id(kind: &str, seed: u64, counter: u64) -> String {
    format!("{kind}-{seed:016x}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339_from_unix(1_798_000_000), "2026-12-23T04:26:40Z");
    }

    #[test]
    fn ids_are_distinct_per_counter_and_seed() {
        assert_ne!(mint_id("task", 7, 1), mint_id("task", 7, 2));
        assert_ne!(mint_id("task", 7, 1), mint_id("task", 8, 1));
        assert!(mint_id("task", 7, 1).starts_with("task-"));
    }
}
