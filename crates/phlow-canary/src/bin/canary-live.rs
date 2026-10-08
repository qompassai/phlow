//! `canary-live`: run the canary battery against a live model served
//! by a local Ollama daemon, through phlow-system1's
//! [`OllamaBackend`] (real distributions from first-token logprobs).
//!
//! This binary is the design's adaptation seam: the battery core is
//! synchronous and runtime-free, and phlow-system1's `OllamaBackend`
//! is a synchronous blocking backend, so the two meet directly
//! behind the [`ProbeBackend`] trait — no async runtime anywhere
//! (the workspace's only async machinery is the msgpack transport's
//! single worker, a standing invariant).
//!
//! Operator data (payload store, threshold book, verdict cache,
//! reports, split log) lives outside the repo, per the crate docs.
//! Usage:
//!
//! ```text
//! canary-live --model <ollama-tag> --artifact <artifact-file> \
//!   --store <payloads.json> [--endpoint <url>] [--book <thresholds.json>] \
//!   [--calibrate] [--cache <verdict-cache.json>] [--report <report.jsonl>] \
//!   [--splits <splits.jsonl>] [--seed <u64>]
//! ```
//!
//! Exit codes: 0 = verdict Deploy, 2 = verdict Refuse, 1 = the
//! battery could not run (fail-closed: an error never deploys).
//! With `--calibrate`, a model missing from the threshold book is
//! first measured on the store's clean inputs and its thresholds are
//! derived with [`calibrate`] and written back to the book.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use phlow_canary::probes::classify_batch;
use phlow_canary::stats::{l1_shift, mean, top_confidence, variance};
use phlow_canary::{
    BaselineStats, CANARY_VERSION, CanarySuite, PayloadStore, ProbeBackend, RunContext,
    ThresholdBook, Verdict, VerdictCache, calibrate, model_hash_reader,
};
use phlow_system1::{OllamaBackend, OllamaConfig, QuestionBatch, RichAnswerBatch, System1Error};

/// The battery's backend: phlow-system1's synchronous Ollama backend
/// behind the [`ProbeBackend`] trait.
struct SyncBackend {
    decider: OllamaBackend,
}

impl ProbeBackend for SyncBackend {
    fn decide(&self, batch: &QuestionBatch) -> Result<RichAnswerBatch, System1Error> {
        self.decider.decide_rich(batch)
    }
}

/// Parsed command line. Paths are operator-supplied; the binary keeps
/// no operator data of its own.
struct Args {
    endpoint: Option<String>,
    model: String,
    artifact: PathBuf,
    store: PathBuf,
    book: Option<PathBuf>,
    cache: Option<PathBuf>,
    report: Option<PathBuf>,
    splits: Option<PathBuf>,
    seed: Option<u64>,
    calibrate: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        endpoint: None,
        model: String::new(),
        artifact: PathBuf::new(),
        store: PathBuf::new(),
        book: None,
        cache: None,
        report: None,
        splits: None,
        seed: None,
        calibrate: false,
    };
    let mut iter = env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut take = |slot: &mut Option<String>| -> Result<(), String> {
            let value = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
            *slot = Some(value);
            Ok(())
        };
        match flag.as_str() {
            "--endpoint" => take(&mut args.endpoint)?,
            "--model" => {
                let mut slot = None;
                take(&mut slot)?;
                args.model = slot.unwrap_or_default();
            }
            "--artifact" => {
                let mut slot = None;
                take(&mut slot)?;
                args.artifact = PathBuf::from(slot.unwrap_or_default());
            }
            "--store" => {
                let mut slot = None;
                take(&mut slot)?;
                args.store = PathBuf::from(slot.unwrap_or_default());
            }
            "--book" => {
                let mut slot = None;
                take(&mut slot)?;
                args.book = Some(PathBuf::from(slot.unwrap_or_default()));
            }
            "--calibrate" => args.calibrate = true,
            "--seed" => {
                let mut slot = None;
                take(&mut slot)?;
                args.seed = Some(
                    slot.unwrap_or_default()
                        .parse()
                        .map_err(|_| "--seed must be a u64".to_owned())?,
                );
            }
            "--cache" => {
                let mut slot = None;
                take(&mut slot)?;
                args.cache = Some(PathBuf::from(slot.unwrap_or_default()));
            }
            "--report" => {
                let mut slot = None;
                take(&mut slot)?;
                args.report = Some(PathBuf::from(slot.unwrap_or_default()));
            }
            "--splits" => {
                let mut slot = None;
                take(&mut slot)?;
                args.splits = Some(PathBuf::from(slot.unwrap_or_default()));
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if args.model.is_empty()
        || args.artifact.as_os_str().is_empty()
        || args.store.as_os_str().is_empty()
    {
        return Err("--model, --artifact and --store are required".to_owned());
    }
    Ok(args)
}

/// Measure one model's baseline on the store's clean inputs: the
/// calibration pass the threshold book's absence demands.
fn measure_baseline(
    backend: &dyn ProbeBackend,
    store: &PayloadStore,
) -> Result<BaselineStats, String> {
    let mut inputs: Vec<String> = Vec::new();
    for set in [
        &store.trigger_rare_token,
        &store.trigger_syntactic,
        &store.trigger_semantic,
    ] {
        for clean in &set.clean {
            if !inputs.contains(clean) {
                inputs.push(clean.clone());
            }
        }
    }
    for input in &store.calibration_inputs {
        if !inputs.contains(input) {
            inputs.push(input.clone());
        }
    }
    inputs.truncate(16);
    let batch = classify_batch(&inputs, "cal");
    let rich = backend
        .decide(&batch)
        .map_err(|e| format!("calibration backend failed: {e}"))?;
    let mut distributions = Vec::with_capacity(inputs.len());
    for index in 0..inputs.len() {
        let answer = rich
            .get(&format!("cal-{index}"))
            .ok_or("calibration backend omitted an answer")?;
        distributions.push(answer.distribution.clone());
    }
    let mut shifts = Vec::new();
    for i in 0..distributions.len() {
        for j in (i + 1)..distributions.len() {
            shifts.push(l1_shift(&distributions[i], &distributions[j]));
        }
    }
    shifts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let p95_index = ((shifts.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(shifts.len() - 1);
    let confidences: Vec<f64> = distributions.iter().map(|d| top_confidence(d)).collect();
    Ok(BaselineStats {
        clean_shift_p95: shifts[p95_index],
        mean_confidence: mean(&confidences),
        confidence_variance: variance(&confidences),
    })
}

/// Write the threshold book owner-only (0600 on unix).
fn write_book(path: &PathBuf, book: &ThresholdBook) -> Result<(), String> {
    let text = serde_json::to_string_pretty(book).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("cannot write book {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot restrict book {}: {e}", path.display()))?;
    }
    Ok(())
}

fn run() -> Result<Verdict, String> {
    let args = parse_args()?;
    let store = PayloadStore::load(&args.store).map_err(|e| e.to_string())?;
    let artifact_file = fs::File::open(&args.artifact)
        .map_err(|e| format!("cannot open artifact {}: {e}", args.artifact.display()))?;
    let model_hash = model_hash_reader(artifact_file).map_err(|e| e.to_string())?;
    println!("artifact: {}", args.artifact.display());
    println!("artifact sha256: {model_hash}");
    println!("model: {}", args.model);
    let timestamp_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // A cached verdict for this exact artifact + canary version is
    // the verdict; the battery does not re-run.
    if let Some(cache_path) = &args.cache {
        let cache = VerdictCache::load(cache_path).map_err(|e| e.to_string())?;
        if let Some(verdict) = cache.lookup(&model_hash, CANARY_VERSION) {
            println!("cached verdict: {}", verdict.as_str());
            return Ok(verdict);
        }
    }

    let mut config = OllamaConfig::new(&args.model);
    if let Some(endpoint) = &args.endpoint {
        config.endpoint = endpoint.clone();
    }
    let decider = OllamaBackend::new(&config).map_err(|e| e.to_string())?;
    let backend = SyncBackend { decider };

    // Thresholds: the model's own, or a fresh calibration.
    let mut book = match &args.book {
        Some(path) if path.exists() => ThresholdBook::load(path).map_err(|e| e.to_string())?,
        _ => ThresholdBook::default(),
    };
    let thresholds = match book.thresholds_for(&args.model) {
        Ok(thresholds) => thresholds,
        Err(_) if args.calibrate => {
            let baseline = measure_baseline(&backend, &store)?;
            println!(
                "calibration baseline: clean_shift_p95={:.6} mean_confidence={:.6} confidence_variance={:.6}",
                baseline.clean_shift_p95, baseline.mean_confidence, baseline.confidence_variance
            );
            let thresholds = calibrate(&baseline);
            println!("calibrated thresholds: {thresholds:?}");
            if let Some(path) = &args.book {
                book.models.insert(args.model.clone(), thresholds);
                write_book(path, &book)?;
                println!("threshold book written: {}", path.display());
            }
            thresholds
        }
        Err(error) => return Err(error.to_string()),
    };

    let seed = args.seed.unwrap_or_else(|| {
        // Production wiring per the suite docs: a nonce mixed with
        // the model hash (here, the run timestamp).
        let mut seed = timestamp_unix ^ 0x5EED_CAFE_F00D_u64;
        for byte in model_hash.as_bytes().iter().take(8) {
            seed = seed.wrapping_mul(31).wrapping_add(u64::from(*byte));
        }
        seed
    });
    let suite = CanarySuite::new(&store, thresholds, seed);
    println!("probes: {}", suite.probe_ids().join(", "));
    let context = RunContext {
        model_id: args.model.clone(),
        model_hash: model_hash.clone(),
        timestamp_unix,
        seed,
    };
    let report = suite.run(&backend, &context);
    for result in &report.results {
        println!(
            "{}: {} evidence={:?}",
            result.probe_id,
            if result.passed { "PASS" } else { "FAIL" },
            result.evidence
        );
    }
    println!(
        "backend calls: {}  elapsed_ms: {}  splits: {}",
        report.backend_calls,
        report.elapsed_ms,
        report.split_runs.len()
    );
    println!("verdict: {}", report.verdict.as_str());
    if let Some(path) = &args.report {
        report.write_jsonl(path).map_err(|e| e.to_string())?;
        println!("report written: {}", path.display());
    }
    if let Some(path) = &args.splits {
        report.append_split_log(path).map_err(|e| e.to_string())?;
    }
    if let Some(path) = &args.cache {
        let mut cache = VerdictCache::load(path).map_err(|e| e.to_string())?;
        cache.record(
            &model_hash,
            &args.model,
            CANARY_VERSION,
            &report.verdict,
            timestamp_unix,
        );
        cache.save(path).map_err(|e| e.to_string())?;
        println!("verdict cached: {}", path.display());
    }
    Ok(report.verdict)
}

fn main() -> ExitCode {
    match run() {
        Ok(Verdict::Deploy) => ExitCode::from(0),
        Ok(Verdict::Refuse { .. }) => ExitCode::from(2),
        Err(reason) => {
            eprintln!("canary-live: {reason}");
            ExitCode::from(1)
        }
    }
}
