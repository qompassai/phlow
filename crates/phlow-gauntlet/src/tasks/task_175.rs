// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 175 — forged code rejection (rust, A).
//!
//! Everything that is not a genuine code minted by *this* daemon dies
//! before the secret store is ever consulted: a corpus of bit-flipped
//! genuine codes returns only `Malformed` or `Authenticity` (never
//! `Mismatch` — the forgery must not reach the compare), a code
//! minted by a *different* daemon instance is refused because the
//! HMAC key is per-instance, and a `ghostex-ec1:`-prefixed code is
//! refused at the prefix gate. The hash-comparison counter proves no
//! forgery reached the secret comparison.
//!
//! The deterministic PRNG below is a 32-bit PCG (inline, no new
//! dependency): the corpus is reproducible from a fixed seed.
//! [`ManualClock`] (MOCK) drives time. Per-instance HMAC keying
//! adapts Ghostex's server-side secret discipline in
//! `server/src/remote_access/pairing_code.rs`; the prefix gate and
//! the no-compare proof are ours.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{Daemon, IssuedCode, PairingError, base64url_decode};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-175";
/// Task name.
pub const NAME: &str = "forged code rejection";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["fuzz_corpus_rejected", "foreign_instance_and_prefix"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;
/// Bit-flip corpus size (deterministic from [`PCG_SEED`]).
pub const FUZZ_CORPUS: usize = 200;
/// PCG32 seed: the corpus is identical on every run.
pub const PCG_SEED: u64 = 0x0175_F026;

/// 32-bit PCG: `state = state * MUL + INC`, output is the
/// xorshifted-high-bits rotation. Inline so the corpus needs no RNG
/// dependency.
struct Pcg32 {
    state: u64,
}

impl Pcg32 {
    fn new(seed: u64) -> Self {
        let mut rng = Pcg32 { state: 0 };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        Pcg32 { state: rng.state }
    }

    fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 58) as u32;
        xorshifted.rotate_right(rot)
    }
}

fn daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

fn issue(
    daemon: &Daemon,
    label: &str,
    now: u64,
) -> Result<crate::pairing::IssuedCode, TaskDriverError> {
    daemon.issue(label, now).map_err(|e| TaskDriverError::Arm {
        arm: "issue".to_string(),
        detail: format!("{e:?}"),
    })
}

/// Flip one deterministic bit of the base64url body of a genuine
/// code, chosen by the PRNG.
fn bit_flip(code: &str, rng: &mut Pcg32) -> String {
    let prefix_len = crate::pairing::CODE_PREFIX.len();
    let mut bytes = code.as_bytes().to_vec();
    let body_len = bytes.len() - prefix_len;
    let at = prefix_len + (rng.next_u32() as usize % body_len);
    let bit = rng.next_u32() % 8;
    bytes[at] ^= 1 << bit;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A1: 200 bit-flipped genuine codes. Every one must fail as
/// `Malformed` or `Authenticity` — never `Mismatch`, which would mean
/// the forgery reached the secret compare. The comparison counter
/// must not move across the whole corpus.
fn case_fuzz_corpus_rejected() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let issued = issue(&daemon, "pixel-9", clock.now())?;
    let mut failures = Vec::new();
    let mut malformed = 0usize;
    let mut authenticity = 0usize;
    let mut rng = Pcg32::new(PCG_SEED);
    let comparisons_before = daemon.hash_comparisons();

    for flip in 0..FUZZ_CORPUS {
        let forged = bit_flip(&issued.code, &mut rng);
        let mut secret = issued.secret.clone();
        match daemon.verify(&forged, &mut secret, clock.now()) {
            Err(PairingError::Malformed) => malformed += 1,
            Err(PairingError::Authenticity) => authenticity += 1,
            other => failures.push(format!(
                "flip {flip}: {other:?}, want only Malformed or Authenticity"
            )),
        }
    }
    let comparisons_after = daemon.hash_comparisons();
    if comparisons_after != comparisons_before {
        failures.push(format!(
            "forgeries reached the secret compare: \
             comparisons {comparisons_before} -> {comparisons_after}"
        ));
    }
    if daemon.device_count() != 0 {
        failures.push(format!(
            "corpus paired {} devices, want 0",
            daemon.device_count()
        ));
    }
    let evidence = vec![
        format!(
            "bit-flip corpus: {FUZZ_CORPUS} forgeries -> \
             {malformed} Malformed, {authenticity} Authenticity, 0 reached the compare"
        ),
        format!("hash comparisons unchanged: {comparisons_before} -> {comparisons_after}"),
    ];

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "corpus": FUZZ_CORPUS,
            "malformed": malformed,
            "authenticity": authenticity,
            "comparisons_before": comparisons_before,
            "comparisons_after": comparisons_after,
            "pairings": daemon.device_count(),
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// A2: a code minted by a different daemon instance (different HMAC
/// key) is refused as `Authenticity`; a `ghostex-ec1:`-prefixed code
/// — same body, wrong prefix — is refused as `Malformed`. Neither
/// reaches the compare; the genuine code on the home daemon still
/// pairs afterwards, proving the forgeries did not poison the store.
/// All three forgeries must die before the secret compare: the
/// cross-instance code at the MAC gate (`Authenticity`), the wrong
/// prefix and the garbage body at the shape gate (`Malformed`).
/// Returns the comparison counter captured right after the
/// forgeries (before the genuine proof-of-health verify).
fn refuse_forgeries(
    home: &Daemon,
    foreign_issued: &IssuedCode,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> u64 {
    let comparisons_before = home.hash_comparisons();
    // Cross-instance code: valid shape, valid MAC — under the wrong
    // key. Must die at the MAC gate.
    let mut secret = foreign_issued.secret.clone();
    match home.verify(&foreign_issued.code, &mut secret, clock_now) {
        Err(PairingError::Authenticity) => {
            evidence.push("foreign-instance code: Authenticity".to_string());
        }
        other => failures.push(format!(
            "foreign-instance code: {other:?}, want Authenticity"
        )),
    }
    // Wrong prefix, genuine body.
    let body_b64 = foreign_issued
        .code
        .strip_prefix(crate::pairing::CODE_PREFIX)
        .unwrap_or("");
    let ghostex_code = format!("ghostex-ec1:{body_b64}");
    let mut secret2 = foreign_issued.secret.clone();
    match home.verify(&ghostex_code, &mut secret2, clock_now) {
        Err(PairingError::Malformed) => {
            evidence.push("ghostex-ec1: prefixed code: Malformed".to_string());
        }
        other => failures.push(format!(
            "ghostex-ec1: prefixed code: {other:?}, want Malformed"
        )),
    }
    // Truncated garbage: not even base64url-shaped.
    let mut secret3 = foreign_issued.secret.clone();
    match home.verify("phlow-ec1:!!!not-base64url!!!", &mut secret3, clock_now) {
        Err(PairingError::Malformed) => {
            evidence.push("garbage body: Malformed".to_string());
        }
        other => failures.push(format!("garbage body: {other:?}, want Malformed")),
    }
    if home.hash_comparisons() != comparisons_before {
        failures.push(format!(
            "forgeries reached the compare: {} -> {}",
            comparisons_before,
            home.hash_comparisons()
        ));
    } else {
        evidence.push("no forgery reached the secret compare".to_string());
    }
    home.hash_comparisons()
}

/// The store is unpoisoned: the home daemon's own genuine code
/// still pairs (its one compare runs after the forgery counter was
/// captured), and the foreign code's body really was well-formed —
/// the refusal came from the MAC, not the shape.
fn confirm_store_unpoisoned(
    home: &Daemon,
    genuine: &IssuedCode,
    body_b64: &str,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    let mut genuine_secret = genuine.secret.clone();
    match home.verify(&genuine.code, &mut genuine_secret, clock_now) {
        Ok(_) => evidence.push("home daemon's genuine code still pairs".to_string()),
        Err(e) => failures.push(format!("genuine code after forgeries: {e:?}, want Ok")),
    }
    let body_ok = base64url_decode(body_b64)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some();
    if !body_ok {
        failures.push("foreign code body did not decode as JSON (fixture broken)".to_string());
    }
}

fn case_foreign_instance_and_prefix() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let home = daemon()?;
    let foreign = daemon()?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let foreign_issued = issue(&foreign, "pixel-9", clock.now())?;
    let genuine = issue(&home, "pixel-9", clock.now())?;
    let comparisons_before = home.hash_comparisons();
    let body_b64 = foreign_issued
        .code
        .strip_prefix(crate::pairing::CODE_PREFIX)
        .unwrap_or("")
        .to_string();
    let comparisons_after_forgeries = refuse_forgeries(
        &home,
        &foreign_issued,
        clock.now(),
        &mut failures,
        &mut evidence,
    );
    confirm_store_unpoisoned(
        &home,
        &genuine,
        &body_b64,
        clock.now(),
        &mut failures,
        &mut evidence,
    );

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "foreign_refused_as_authenticity": true,
            "ghostex_prefix_refused_as_malformed": true,
            "comparisons_unchanged": comparisons_after_forgeries == comparisons_before,
            "genuine_still_pairs": true,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "fuzz_corpus_rejected" => case_fuzz_corpus_rejected(),
        "foreign_instance_and_prefix" => case_foreign_instance_and_prefix(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-175: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-175".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-175".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
