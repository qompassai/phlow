//! Per-token parity harness: Mojo logprob kernel vs the crate's f64
//! reference (and, optionally, the torch f32 sidecar outputs) on one
//! bench batch.
//!
//! Usage:
//!   cargo run --release --example f64_parity -- \
//!     --batch tools/out/bench_logits.bin [--torch tools/out/bench_torch_out.npy]
//!
//! The batch is the bench format written by `tools/check_via_ctypes.py`
//! (i32 rows, i32 vocab, f32 logits, i32 targets). The torch file is
//! the 1-D `<f4` .npy that `tools/bench_torch.py` saves for that batch.
//! The kernel path is `ffi::logprob_token_logps`; the f64 reference is
//! `reference::logprob_token_logps` (f64 log-sum-exp in index order,
//! rounded once to f32). Prints one line per token, then the summary.
//! Measurement tooling only: it exits non-zero on I/O or kernel
//! failure, never on the size of a diff.

use std::fs;
use std::process::ExitCode;

use randori::{ffi, reference};

/// Input files larger than this are refused before reading (1 GiB).
const FILE_BYTES_MAX: u64 = 1 << 30;

/// Read a whole input file, refusing anything above the size bound.
fn read_bounded(path: &str) -> Result<Vec<u8>, String> {
    let len = fs::metadata(path)
        .map_err(|e| format!("{path}: {e}"))?
        .len();
    if len > FILE_BYTES_MAX {
        return Err(format!("{path}: {len} bytes exceeds {FILE_BYTES_MAX}"));
    }
    fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// Little-endian 4-byte word at `pos`, or a truncation error.
fn word(bytes: &[u8], pos: usize) -> Result<[u8; 4], String> {
    let slice = bytes.get(pos..pos + 4).ok_or("truncated input")?;
    Ok([slice[0], slice[1], slice[2], slice[3]])
}

/// Parse a bench batch: `(rows, vocab, logits, targets)`.
fn read_batch(path: &str) -> Result<(usize, usize, Vec<f32>, Vec<i32>), String> {
    let bytes = read_bounded(path)?;
    let rows = usize::try_from(i32::from_le_bytes(word(&bytes, 0)?)).map_err(|e| e.to_string())?;
    let vocab = usize::try_from(i32::from_le_bytes(word(&bytes, 4)?)).map_err(|e| e.to_string())?;
    let count = rows.checked_mul(vocab).ok_or("rows * vocab overflows")?;
    let targets_at = 8 + count * 4;
    if bytes.len() != targets_at + rows * 4 {
        return Err(format!(
            "{path}: {} bytes, want {}",
            bytes.len(),
            targets_at + rows * 4
        ));
    }
    let mut logits = Vec::with_capacity(count);
    for index in 0..count {
        logits.push(f32::from_le_bytes(word(&bytes, 8 + index * 4)?));
    }
    let mut targets = Vec::with_capacity(rows);
    for row in 0..rows {
        targets.push(i32::from_le_bytes(word(&bytes, targets_at + row * 4)?));
    }
    Ok((rows, vocab, logits, targets))
}

/// Parse a 1-D little-endian f32 .npy (format 1.0) of exactly `rows`.
fn read_npy_f32(path: &str, rows: usize) -> Result<Vec<f32>, String> {
    let bytes = read_bounded(path)?;
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" || bytes[6] != 1 {
        return Err(format!("{path}: not a format-1.0 .npy file"));
    }
    let header_len = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let header = std::str::from_utf8(bytes.get(10..10 + header_len).ok_or("truncated header")?)
        .map_err(|e| e.to_string())?;
    let shape = format!("'shape': ({rows},)");
    if !header.contains("'descr': '<f4'")
        || !header.contains("'fortran_order': False")
        || !header.contains(&shape)
    {
        return Err(format!("{path}: header {header:?} is not <f4 {shape}"));
    }
    let data_at = 10 + header_len;
    if bytes.len() != data_at + rows * 4 {
        return Err(format!("{path}: data length mismatch"));
    }
    (0..rows)
        .map(|row| word(&bytes, data_at + row * 4).map(f32::from_le_bytes))
        .collect()
}

/// `(max |a - b|, row of the max, mean |a - b|, exact-match count)`.
fn diff_stats(a: &[f32], b: &[f32]) -> (f64, usize, f64, usize) {
    assert_eq!(a.len(), b.len(), "diff_stats: length mismatch");
    let (mut worst, mut worst_row, mut total, mut exact) = (0.0_f64, 0_usize, 0.0_f64, 0_usize);
    for (row, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        let d = (f64::from(*x) - f64::from(*y)).abs();
        if d > worst {
            (worst, worst_row) = (d, row);
        }
        total += d;
        exact += usize::from(x == y);
    }
    (worst, worst_row, total / a.len() as f64, exact)
}

fn print_stats(label: &str, a: &[f32], b: &[f32]) {
    let (worst, row, mean, exact) = diff_stats(a, b);
    println!(
        "{label}: max |d| = {worst:.3e} (row {row}), mean |d| = {mean:.3e}, \
         exact {exact}/{}",
        a.len()
    );
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let batch = flag("--batch").ok_or("missing --batch <bin>")?;
    let (rows, vocab, logits, targets) = read_batch(&batch)?;
    let kernel = ffi::logprob_token_logps(&logits, rows, vocab, &targets)
        .map_err(|e| format!("kernel: {e}"))?;
    let f64_ref = reference::logprob_token_logps(&logits, rows, vocab, &targets);
    let torch = match flag("--torch") {
        Some(path) => Some(read_npy_f32(&path, rows)?),
        None => None,
    };
    println!("row target kernel f64_ref |kernel-f64_ref| torch |kernel-torch|");
    for row in 0..rows {
        let (k, r) = (kernel[row], f64_ref[row]);
        let d_ref = (f64::from(k) - f64::from(r)).abs();
        match &torch {
            Some(t) => {
                let d_t = (f64::from(k) - f64::from(t[row])).abs();
                println!(
                    "{row} {} {k:.9e} {r:.9e} {d_ref:.3e} {:.9e} {d_t:.3e}",
                    targets[row], t[row]
                );
            }
            None => println!("{row} {} {k:.9e} {r:.9e} {d_ref:.3e} - -", targets[row]),
        }
    }
    println!("batch: {batch} ({rows}x{vocab})");
    print_stats("kernel vs f64 reference", &kernel, &f64_ref);
    match &torch {
        Some(t) => {
            print_stats("kernel vs torch f32", &kernel, t);
            print_stats("torch f32 vs f64 reference", t, &f64_ref);
        }
        None => println!("kernel vs torch f32: not measured (no --torch file)"),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("f64_parity: {error}");
            ExitCode::FAILURE
        }
    }
}
