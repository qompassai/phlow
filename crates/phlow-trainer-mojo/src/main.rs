//! CLI driver for the Mojo scoring + sampling backend.
//!
//! Modes:
//! - `version` — print the scoring ABI version reported by the library.
//! - `score --manifest <json> --blocks <bin>` — score every entry of a
//!   control-run manifest (parity batch or one export group) through the
//!   Rust→Mojo FFI and compare against the manifest's torch values.
//! - `advantages --groups <export.json>` — recompute RLOO advantages for
//!   every group of a trainlab groups export and compare against the
//!   export's f64 values.
//! - `bench --batch <bin> --repeats <n>` — time the logprob kernel on a
//!   seeded synthetic batch file (header + f32 logits + i32 targets).
//! - `sample --batch <bin> [--temperature t] [--top-k k] [--top-p p]
//!   [--seed s] [--draw d] [--dump-candidates <json>]` — sample one token
//!   per row of a batch file (header + f32 logits; trailing targets, if
//!   present, are ignored) through the backend dispatch.
//! - `sample-bench --batch <bin> --repeats <n> [sampling flags]` — time
//!   the sampling kernel on resident data.
//! - `backend-info` — report backend availability (GPU residency policy
//!   + kernel ABI) without allocating.
//! - `run --export <groups.json> --trainlab-receipt <receipt.json>
//!   --manifest <json> --blocks <bin> --group <n> --out <receipt.json>
//!   [--sample] [sampling flags] [--gpu-free-mib-min <n>] [--no-fallback]`
//!   — the production path: verify the export↔receipt tie, score one
//!   group through the backend dispatch, and write a scoring receipt
//!   (refusing to overwrite). `run`/`backend-info`/`sample` need the
//!   default `trainlab-contract` feature.
//!
//! Exit status is non-zero if any comparison exceeds its tolerance or
//! any kernel call fails; the tolerances match the Python check driver
//! (token/mean 5e-4, advantages 1e-5).

use std::env;
use std::fs;
use std::process::ExitCode;

#[cfg(feature = "trainlab-contract")]
use phlow_trainer_mojo::backend::{self, BackendConfig, LogitBlock, ManifestEntry};
use phlow_trainer_mojo::error::ScoringError;
use phlow_trainer_mojo::ffi;
use phlow_trainer_mojo::reference::SampleParams;

/// Input files larger than this are refused before reading (1 GiB).
const FILE_BYTES_MAX: u64 = 1 << 30;
/// Token/mean logprob agreement bound vs the torch control values.
const LOGPROB_DIFF_MAX: f64 = 5e-4;
/// Advantage agreement bound vs trainlab's f64 values.
const ADVANTAGE_DIFF_MAX: f64 = 1e-5;

/// One scored block read from a control `.bin` file.
struct Block {
    /// Row count (completion tokens scored).
    rows: usize,
    /// Logit width (vocabulary).
    width: usize,
    /// Row-major f32 logits, `rows * width` values.
    logits: Vec<f32>,
}

/// Read a whole input file, refusing anything above the size bound.
fn read_bounded(path: &str) -> Result<Vec<u8>, ScoringError> {
    let metadata = fs::metadata(path).map_err(|e| ScoringError::InputFile {
        source: path.to_string(),
        detail: e.to_string(),
    })?;
    if metadata.len() > FILE_BYTES_MAX {
        return Err(ScoringError::InputFile {
            source: path.to_string(),
            detail: format!(
                "{} bytes exceeds the {FILE_BYTES_MAX} bound",
                metadata.len()
            ),
        });
    }
    fs::read(path).map_err(|e| ScoringError::InputFile {
        source: path.to_string(),
        detail: e.to_string(),
    })
}

/// Read an i32 from little-endian bytes at `pos`.
fn read_i32(bytes: &[u8], pos: usize) -> Result<i32, ScoringError> {
    let slice = bytes
        .get(pos..pos + 4)
        .ok_or_else(|| ScoringError::InputFile {
            source: "blocks".to_string(),
            detail: "truncated block header".to_string(),
        })?;
    Ok(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Parse the control block format: repeated (rows, width, f32 data).
fn read_blocks(path: &str) -> Result<Vec<Block>, ScoringError> {
    let bytes = read_bounded(path)?;
    let mut blocks = Vec::new();
    let mut pos = 0_usize;
    while pos < bytes.len() {
        let rows = read_i32(&bytes, pos)?;
        let width = read_i32(&bytes, pos + 4)?;
        pos += 8;
        if rows < 0 || width < 0 {
            return Err(ScoringError::InputFile {
                source: path.to_string(),
                detail: "negative block shape".to_string(),
            });
        }
        let count =
            (rows as usize)
                .checked_mul(width as usize)
                .ok_or_else(|| ScoringError::InputFile {
                    source: path.to_string(),
                    detail: "block shape overflows usize".to_string(),
                })?;
        let byte_len = count
            .checked_mul(4)
            .ok_or_else(|| ScoringError::InputFile {
                source: path.to_string(),
                detail: "block size overflows usize".to_string(),
            })?;
        let data = bytes
            .get(pos..pos + byte_len)
            .ok_or_else(|| ScoringError::InputFile {
                source: path.to_string(),
                detail: "truncated block data".to_string(),
            })?;
        let mut logits = Vec::with_capacity(count);
        for index in 0..count {
            let at = index * 4;
            logits.push(f32::from_le_bytes([
                data[at],
                data[at + 1],
                data[at + 2],
                data[at + 3],
            ]));
        }
        pos += byte_len;
        blocks.push(Block {
            rows: rows as usize,
            width: width as usize,
            logits,
        });
    }
    Ok(blocks)
}

/// Parse a JSON input file into a serde_json value.
fn read_json(path: &str) -> Result<serde_json::Value, ScoringError> {
    let bytes = read_bounded(path)?;
    serde_json::from_slice(&bytes).map_err(|e| ScoringError::InputFile {
        source: path.to_string(),
        detail: e.to_string(),
    })
}

/// Extract a JSON array of numbers as f64 values.
fn json_f64s(value: &serde_json::Value, field: &str) -> Result<Vec<f64>, ScoringError> {
    let items =
        value
            .get(field)
            .and_then(|v| v.as_array())
            .ok_or_else(|| ScoringError::InputFile {
                source: field.to_string(),
                detail: "missing or non-array field".to_string(),
            })?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(item.as_f64().ok_or_else(|| ScoringError::InputFile {
            source: field.to_string(),
            detail: "non-numeric array element".to_string(),
        })?);
    }
    Ok(out)
}

/// Score one manifest entry; returns the worst token diff observed.
fn score_entry(
    index: usize,
    entry: &serde_json::Value,
    block: &Block,
) -> Result<f64, ScoringError> {
    let targets_f = json_f64s(entry, "targets")?;
    let targets: Vec<i32> = targets_f.iter().map(|v| *v as i32).collect();
    let want = json_f64s(entry, "token_logps")?;
    let got = ffi::logprob_token_logps(&block.logits, block.rows, block.width, &targets)?;
    if got.len() != want.len() {
        return Err(ScoringError::InputFile {
            source: format!("entry {index}"),
            detail: "token count mismatch with manifest".to_string(),
        });
    }
    let mut worst = 0.0_f64;
    for (g, w) in got.iter().zip(want.iter()) {
        worst = worst.max((f64::from(*g) - w).abs());
    }
    let mean = f64::from(got.iter().sum::<f32>()) / got.len() as f64;
    let control_mean = entry
        .get("mean_logprob")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(f64::NAN);
    println!(
        "entry {index}: rows={} mojo mean {mean:.5} vs control {control_mean:.5} \
         (worst token |d| {worst:.2e})",
        block.rows
    );
    Ok(worst)
}

/// `score` mode: all manifest entries through the FFI.
fn mode_score(manifest_path: &str, blocks_path: &str) -> Result<bool, ScoringError> {
    let manifest = read_json(manifest_path)?;
    let entries = manifest
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScoringError::InputFile {
            source: manifest_path.to_string(),
            detail: "manifest has no entries array".to_string(),
        })?;
    let blocks = read_blocks(blocks_path)?;
    if blocks.len() != entries.len() {
        return Err(ScoringError::InputFile {
            source: blocks_path.to_string(),
            detail: format!(
                "{} blocks for {} manifest entries",
                blocks.len(),
                entries.len()
            ),
        });
    }
    let mut worst = 0.0_f64;
    for (index, (entry, block)) in entries.iter().zip(blocks.iter()).enumerate() {
        worst = worst.max(score_entry(index, entry, block)?);
    }
    println!("score: worst token |d| = {worst:.2e} (bound {LOGPROB_DIFF_MAX:.0e})");
    Ok(worst <= LOGPROB_DIFF_MAX)
}

/// `advantages` mode: every group of a trainlab groups export.
fn mode_advantages(groups_path: &str) -> Result<bool, ScoringError> {
    let doc = read_json(groups_path)?;
    let groups = doc
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScoringError::InputFile {
            source: groups_path.to_string(),
            detail: "export has no groups array".to_string(),
        })?;
    let mut rewards: Vec<f32> = Vec::new();
    let mut want: Vec<f64> = Vec::new();
    let mut offsets: Vec<i32> = vec![0];
    for group in groups {
        for value in json_f64s(group, "rewards")? {
            rewards.push(value as f32);
        }
        want.extend(json_f64s(group, "advantages")?);
        offsets.push(
            i32::try_from(rewards.len()).map_err(|_| ScoringError::InputFile {
                source: groups_path.to_string(),
                detail: "completion count exceeds i32".to_string(),
            })?,
        );
    }
    let got = ffi::rloo_advantages(&rewards, &offsets)?;
    let mut worst = 0.0_f64;
    for (g, w) in got.iter().zip(want.iter()) {
        worst = worst.max((f64::from(*g) - w).abs());
    }
    println!(
        "advantages: {} groups, {} completions, worst |d| = {worst:.2e} \
         (bound {ADVANTAGE_DIFF_MAX:.0e})",
        groups.len(),
        rewards.len()
    );
    Ok(worst <= ADVANTAGE_DIFF_MAX)
}

/// `bench` mode: one bench-batch file, kernel launch timing.
///
/// The bench file is a single block (header + f32 logits) with the i32
/// targets appended directly after the logits — it is NOT a blocks
/// file, so it gets its own reader rather than abusing read_blocks.
fn mode_bench(batch_path: &str, repeats: usize) -> Result<bool, ScoringError> {
    let bytes = read_bounded(batch_path)?;
    if bytes.len() < 8 {
        return Err(ScoringError::InputFile {
            source: batch_path.to_string(),
            detail: "bench batch shorter than its header".to_string(),
        });
    }
    let rows = read_i32(&bytes, 0)?;
    let width = read_i32(&bytes, 4)?;
    if rows < 0 || width < 0 {
        return Err(ScoringError::InputFile {
            source: batch_path.to_string(),
            detail: "negative bench batch shape".to_string(),
        });
    }
    let rows = rows as usize;
    let width = width as usize;
    let count = rows
        .checked_mul(width)
        .ok_or_else(|| ScoringError::InputFile {
            source: batch_path.to_string(),
            detail: "bench batch shape overflows usize".to_string(),
        })?;
    let logits_end = 8 + count * 4;
    let data = bytes
        .get(8..logits_end)
        .ok_or_else(|| ScoringError::InputFile {
            source: batch_path.to_string(),
            detail: "truncated bench logits".to_string(),
        })?;
    let mut logits = Vec::with_capacity(count);
    for index in 0..count {
        let at = index * 4;
        logits.push(f32::from_le_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]));
    }
    let block = Block {
        rows,
        width,
        logits,
    };
    let mut targets = Vec::with_capacity(block.rows);
    for row in 0..block.rows {
        targets.push(read_i32(&bytes, logits_end + row * 4)?);
    }
    let total_ns =
        ffi::logprob_bench_ns(&block.logits, block.rows, block.width, &targets, repeats)?;
    let per_launch_us = total_ns as f64 / repeats as f64 / 1_000.0;
    println!(
        "bench: {repeats} launches of {}x{} f32 = {per_launch_us:.1} us/launch",
        block.rows, block.width
    );
    Ok(true)
}

/// Read a batch file's logits: `(rows, vocab, logits)`. The bench
/// batch format appends i32 targets after the logits; sampling
/// ignores any trailing bytes.
fn read_batch_logits(path: &str) -> Result<(usize, usize, Vec<f32>), ScoringError> {
    let bytes = read_bounded(path)?;
    if bytes.len() < 8 {
        return Err(ScoringError::InputFile {
            source: path.to_string(),
            detail: "batch shorter than its header".to_string(),
        });
    }
    let rows = read_i32(&bytes, 0)?;
    let width = read_i32(&bytes, 4)?;
    if rows < 0 || width < 0 {
        return Err(ScoringError::InputFile {
            source: path.to_string(),
            detail: "negative batch shape".to_string(),
        });
    }
    let rows = rows as usize;
    let width = width as usize;
    let count = rows
        .checked_mul(width)
        .ok_or_else(|| ScoringError::InputFile {
            source: path.to_string(),
            detail: "batch shape overflows usize".to_string(),
        })?;
    let data = bytes
        .get(8..8 + count * 4)
        .ok_or_else(|| ScoringError::InputFile {
            source: path.to_string(),
            detail: "truncated batch logits".to_string(),
        })?;
    let mut logits = Vec::with_capacity(count);
    for index in 0..count {
        let at = index * 4;
        logits.push(f32::from_le_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]));
    }
    Ok((rows, width, logits))
}

/// Parse a `--flag value` as `T`, falling back to `default`.
fn flag_parse<T: std::str::FromStr>(args: &[String], flag: &str, default: T) -> T {
    flag_value(args, flag)
        .ok()
        .and_then(|value| value.parse::<T>().ok())
        .unwrap_or(default)
}

/// Sampling parameters from CLI flags (defaults: T=1, no top-k,
/// no nucleus limit, seed 0, draw 0).
fn sample_params_from(args: &[String]) -> SampleParams {
    SampleParams {
        temperature: flag_parse(args, "--temperature", 1.0_f32),
        top_k: flag_parse(args, "--top-k", 0_u32),
        top_p: flag_parse(args, "--top-p", 1.0_f32),
        seed: flag_parse(args, "--seed", 0_u64),
        draw_index: flag_parse(args, "--draw", 0_u32),
    }
}

/// `sample` mode: one draw per batch row through the backend
/// dispatch; prints the tokens/counts as one JSON object.
#[cfg(feature = "trainlab-contract")]
fn mode_sample(args: &[String]) -> Result<bool, ScoringError> {
    let (rows, width, logits) = read_batch_logits(&flag_value(args, "--batch")?)?;
    let params = sample_params_from(args);
    let config = BackendConfig::default();
    let dispatch = backend::sample(&logits, rows, width, &params, &config)?;
    let batch = &dispatch.value;
    println!(
        "{}",
        serde_json::json!({
            "path": dispatch.path.as_str(),
            "fallback_reason": dispatch.fallback_reason,
            "tokens": batch.tokens,
            "candidate_counts": batch.candidate_counts,
        })
    );
    if let Ok(path) = flag_value(args, "--dump-candidates") {
        let max = phlow_trainer_mojo::SAMPLE_CANDIDATES_MAX as usize;
        let rows_json: Vec<serde_json::Value> = (0..rows)
            .map(|row| {
                let count = batch.candidate_counts[row].max(0) as usize;
                let shown = count.min(max);
                serde_json::json!({
                    "token": batch.tokens[row],
                    "candidate_count": count,
                    "candidates": batch.cand_idx[row * max..row * max + shown],
                    "candidate_logits": batch.cand_val[row * max..row * max + shown],
                })
            })
            .collect();
        fs::write(
            &path,
            serde_json::to_string_pretty(&rows_json).unwrap_or_default(),
        )
        .map_err(|error| ScoringError::InputFile {
            source: path.clone(),
            detail: error.to_string(),
        })?;
        println!("candidates written to {path}");
    }
    Ok(true)
}

/// `sample-bench` mode: kernel launch timing on resident data.
fn mode_sample_bench(args: &[String]) -> Result<bool, ScoringError> {
    let (rows, width, logits) = read_batch_logits(&flag_value(args, "--batch")?)?;
    let repeats = flag_parse(args, "--repeats", 50_usize);
    let params = sample_params_from(args);
    if let Ok(snapshot) = phlow_trainer_mojo::gpu_policy::query_gpu() {
        println!(
            "gpu: {} MiB free / {} MiB total at bench start",
            snapshot.free_mib, snapshot.total_mib
        );
    }
    let total_ns = ffi::sample_bench_ns(&logits, rows, width, &params, repeats)?;
    let per_launch_us = total_ns as f64 / repeats as f64 / 1_000.0;
    println!(
        "sample-bench: {repeats} launches of {rows}x{width} f32 \
         (T={} top_k={} top_p={}) = {per_launch_us:.1} us/launch",
        params.temperature, params.top_k, params.top_p
    );
    Ok(true)
}

/// `backend-info` mode: availability probe (no allocation).
#[cfg(feature = "trainlab-contract")]
fn mode_backend_info(args: &[String]) -> Result<bool, ScoringError> {
    let mut config = BackendConfig::default();
    if let Ok(value) = flag_value(args, "--gpu-free-mib-min") {
        config.gpu_free_mib_min = value.parse().unwrap_or(config.gpu_free_mib_min);
    }
    if args.iter().any(|arg| arg == "--no-fallback") {
        config.allow_reference_fallback = false;
    }
    let info = backend::availability(&config);
    println!(
        "{}",
        serde_json::json!({
            "backend": backend::BACKEND_ID,
            "kernel_version": backend::KERNEL_VERSION_LABEL,
            "abi_version": info.abi_version,
            "available": info.available,
            "gpu_free_mib": info.gpu.map(|gpu| gpu.free_mib),
            "gpu_total_mib": info.gpu.map(|gpu| gpu.total_mib),
            "gpu_free_mib_min": config.gpu_free_mib_min,
            "fallback_allowed": config.allow_reference_fallback,
            "reason": info.reason,
        })
    );
    Ok(true)
}

/// `run` mode: the production scoring path + scoring receipt.
#[cfg(feature = "trainlab-contract")]
fn mode_run(args: &[String]) -> Result<bool, ScoringError> {
    let manifest_path = flag_value(args, "--manifest")?;
    let manifest = read_json(&manifest_path)?;
    let entries_json = manifest
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScoringError::InputFile {
            source: manifest_path.clone(),
            detail: "manifest has no entries array".to_string(),
        })?;
    let blocks_raw = read_blocks(&flag_value(args, "--blocks")?)?;
    if blocks_raw.len() != entries_json.len() {
        return Err(ScoringError::InputFile {
            source: manifest_path.clone(),
            detail: format!(
                "{} blocks for {} manifest entries",
                blocks_raw.len(),
                entries_json.len()
            ),
        });
    }
    let mut entries: Vec<ManifestEntry> = Vec::with_capacity(entries_json.len());
    let mut blocks: Vec<LogitBlock> = Vec::with_capacity(blocks_raw.len());
    for (entry, block) in entries_json.iter().zip(blocks_raw.iter()) {
        let targets: Vec<i32> = json_f64s(entry, "targets")?
            .iter()
            .map(|value| *value as i32)
            .collect();
        entries.push(ManifestEntry {
            targets: targets.clone(),
            token_logps: json_f64s(entry, "token_logps")?,
            mean_logprob: entry
                .get("mean_logprob")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(f64::NAN),
        });
        blocks.push(LogitBlock {
            logits: block.logits.clone(),
            rows: block.rows,
            vocab: block.width,
            targets,
        });
    }
    let group: usize = flag_parse(args, "--group", 0_usize);
    if group == 0 {
        return Err(ScoringError::InputFile {
            source: "--group".to_string(),
            detail: "required 1-based group number missing".to_string(),
        });
    }
    let mut config = BackendConfig::default();
    if let Ok(value) = flag_value(args, "--gpu-free-mib-min") {
        config.gpu_free_mib_min = value.parse().unwrap_or(config.gpu_free_mib_min);
    }
    if args.iter().any(|arg| arg == "--no-fallback") {
        config.allow_reference_fallback = false;
    }
    let sampler = if args.iter().any(|arg| arg == "--sample") {
        Some(sample_params_from(args))
    } else {
        None
    };
    let run = backend::run_scoring(
        std::path::Path::new(&flag_value(args, "--export")?),
        std::path::Path::new(&flag_value(args, "--trainlab-receipt")?),
        group,
        &entries,
        &blocks,
        sampler,
        &config,
    )?;
    let out_path = flag_value(args, "--out")?;
    run.receipt.write_new(std::path::Path::new(&out_path))?;
    println!(
        "run: path={} worst token |d| = {:.2e} (bound {LOGPROB_DIFF_MAX:.0e}), \
         worst mean |d| = {:.2e}, worst advantage |d| = {:.2e} (bound \
         {ADVANTAGE_DIFF_MAX:.0e})",
        run.receipt.scoring_path,
        run.worst_token_diff,
        run.worst_mean_diff,
        run.worst_advantage_diff
    );
    if let Some(tokens) = &run.sampled_tokens {
        println!("run: sampled tokens (first block rows): {tokens:?}");
    }
    println!("run: scoring receipt written to {out_path}");
    Ok(run.worst_mean_diff <= LOGPROB_DIFF_MAX
        && run.worst_token_diff <= LOGPROB_DIFF_MAX
        && run.worst_advantage_diff <= ADVANTAGE_DIFF_MAX)
}

/// Fetch a `--flag value` pair from argv.
fn flag_value(args: &[String], flag: &str) -> Result<String, ScoringError> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned().ok_or_else(|| ScoringError::InputFile {
                source: flag.to_string(),
                detail: "flag needs a value".to_string(),
            });
        }
    }
    Err(ScoringError::InputFile {
        source: flag.to_string(),
        detail: "required flag missing".to_string(),
    })
}

fn run() -> Result<bool, ScoringError> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = args.first().map_or("version", String::as_str);
    match mode {
        "version" => {
            println!("scoring ABI version: {}", ffi::scoring_version());
            Ok(true)
        }
        "score" => mode_score(
            &flag_value(&args, "--manifest")?,
            &flag_value(&args, "--blocks")?,
        ),
        "advantages" => mode_advantages(&flag_value(&args, "--groups")?),
        "bench" => {
            let repeats = flag_value(&args, "--repeats")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            mode_bench(&flag_value(&args, "--batch")?, repeats)
        }
        "sample-bench" => mode_sample_bench(&args),
        #[cfg(feature = "trainlab-contract")]
        "sample" => mode_sample(&args),
        #[cfg(feature = "trainlab-contract")]
        "backend-info" => mode_backend_info(&args),
        #[cfg(feature = "trainlab-contract")]
        "run" => mode_run(&args),
        other => Err(ScoringError::InputFile {
            source: other.to_string(),
            detail: "unknown mode (version|score|advantages|bench|sample|sample-bench|\
                     backend-info|run)"
                .to_string(),
        }),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("comparison FAILED");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
