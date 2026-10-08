//! CLI driver for the Mojo kernel track.
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
//!
//! Exit status is non-zero if any comparison exceeds its tolerance or
//! any kernel call fails; the tolerances match the Python check driver
//! (token/mean 5e-4, advantages 1e-5).

use std::env;
use std::fs;
use std::process::ExitCode;

use phlow_trainer_mojo::error::ScoringError;
use phlow_trainer_mojo::ffi;

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
        for chunk in data.chunks_exact(4) {
            logits.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
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
    for chunk in data.chunks_exact(4) {
        logits.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
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
        other => Err(ScoringError::InputFile {
            source: other.to_string(),
            detail: "unknown mode (version|score|advantages|bench)".to_string(),
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
