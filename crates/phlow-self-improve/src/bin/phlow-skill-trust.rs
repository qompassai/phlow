//! `phlow-skill-trust` — PH-8's operator surface: recompute
//! per-skill trust from a feedback store's run rows and print the
//! report (writing it only when `--out` names a file).
//!
//! Usage:
//!     phlow-skill-trust --db PATH [--out PATH]
//!
//! The estate's skill-trust CLI (hatch-side) ranks skills from
//! recorded outcomes; this is phlow's side of that contract —
//! evidence produced *from run rows*, on this machine, with no
//! network path between the two. The recompute itself lives in
//! `phlow_self_improve::skill_trust`; see its docs for the method.

use std::path::PathBuf;
use std::process::ExitCode;

use phlow_self_improve::skill_trust::{load_skill_rows, render_trust_report, skill_evidence};

/// Parsed command line.
struct Args {
    db: PathBuf,
    out: Option<PathBuf>,
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut db: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut index = 0;
    while index < argv.len() {
        let flag = argv[index].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            argv.get(*index)
                .cloned()
                .ok_or_else(|| format!("flag {flag} needs a value"))
        };
        match flag {
            "--db" => db = Some(PathBuf::from(value(&mut index)?)),
            "--out" => out = Some(PathBuf::from(value(&mut index)?)),
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(Args {
        db: db.ok_or_else(|| "--db is required".to_owned())?,
        out,
    })
}

fn run(args: &Args) -> Result<String, String> {
    let rows = load_skill_rows(&args.db).map_err(|e| e.to_string())?;
    let evidence = skill_evidence(&rows);
    let report = render_trust_report(&rows, &evidence);
    if let Some(path) = &args.out {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(path, &report)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(report)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&argv).and_then(|args| run(&args)) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("phlow-skill-trust: {error}");
            ExitCode::FAILURE
        }
    }
}
