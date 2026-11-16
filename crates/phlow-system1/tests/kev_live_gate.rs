//! Live gate for the D-1622 integration (ignored by default): re-runs
//! the D-1594 eval's 28 items through the INTEGRATED path —
//! [`KevBackend`] + [`TriageScorer`] at the default cutoff — against
//! the running kev-serve unit, twice per item (determinism), and
//! prints the gate numbers: agreement with the bare-model eval,
//! escalation rate, accepted vs escalated accuracy, and end-to-end
//! latency. The escalated subset's "incumbent" column replays the
//! eval's recorded incumbent answers, standing in for the caller's
//! existing path exactly as the D-1622 contract defines escalation.
//!
//! Run with:
//! `cargo test -p phlow-system1 --test kev_live_gate -- --ignored --nocapture`
//!
//! Environment: `KEV_GATE_ENDPOINT` (default [`DEFAULT_KEV_ENDPOINT`]),
//! `KEV_GATE_QUESTIONS` (default the primo eval workdir's
//! `questions.json`), `KEV_GATE_SUMMARY` (default
//! `<questions dir>/artefacts/probe1-summary.json`).

use std::collections::BTreeMap;
use std::time::Instant;

use phlow_system1::{
    Answer, DEFAULT_KEV_ENDPOINT, KevBackend, KevConfig, Question, QuestionBatch, TriageEscalation,
    TriageScorer, TriageVerdict,
};
use serde_json::Value;

const DEFAULT_QUESTIONS: &str =
    "/home/phaedrus/.local/share/pax/model-evals-20261116/questions.json";

struct Item {
    id: String,
    batch: QuestionBatch,
    options: Vec<String>,
    gold_label: Option<String>,
    gold_yes: Option<bool>,
}

fn load_items(path: &str) -> Vec<Item> {
    let text = std::fs::read_to_string(path).expect("questions file readable");
    let items: Vec<Value> = serde_json::from_str(&text).expect("questions file is a JSON array");
    items
        .iter()
        .map(|item| {
            let state = item["state"].as_str().expect("state").to_owned();
            let instructions = item["question"].as_str().expect("question").to_owned();
            let (question, options, gold_label, gold_yes) = match item["type"].as_str() {
                Some("choice") => {
                    let options: Vec<String> = item["options"]
                        .as_object()
                        .expect("choice options")
                        .keys()
                        .cloned()
                        .collect();
                    (
                        Question::Choice {
                            instructions,
                            options: options.clone(),
                        },
                        options,
                        Some(item["gold"].as_str().expect("gold label").to_owned()),
                        None,
                    )
                }
                Some("noul") => (
                    Question::Noul { instructions },
                    Vec::new(),
                    None,
                    Some(item["gold"].as_bool().expect("gold bool")),
                ),
                other => panic!("unknown item type {other:?}"),
            };
            Item {
                id: item["id"].as_str().expect("id").to_owned(),
                batch: QuestionBatch {
                    state,
                    questions: BTreeMap::from([("q".to_owned(), question)]),
                },
                options,
                gold_label,
                gold_yes,
            }
        })
        .collect()
}

/// (pred, correct) per (item id, model) from the eval summary,
/// whose shape is `{"kev": {"model": "kev", "rows": [...]},
/// "incumbent": {"model": "incumbent", "rows": [...]}}`.
fn load_eval(path: &str) -> BTreeMap<(String, String), (String, bool)> {
    let text = std::fs::read_to_string(path).expect("summary file readable");
    let root: Value = serde_json::from_str(&text).expect("summary is a JSON object");
    let mut out = BTreeMap::new();
    for section in ["kev", "incumbent"] {
        let model = root[section]["model"].as_str().expect("section model");
        for row in root[section]["rows"].as_array().expect("section rows") {
            // Choice rows carry a string label; noul rows a bool,
            // normalized here to the "yes"/"no" labels `predicted` uses.
            let pred = match &row["pred"] {
                Value::String(label) => label.clone(),
                Value::Bool(true) => "yes".to_owned(),
                Value::Bool(false) => "no".to_owned(),
                other => panic!("unexpected pred shape {other}"),
            };
            out.insert(
                (
                    row["id"].as_str().expect("row id").to_owned(),
                    model.to_owned(),
                ),
                (pred, row["correct"].as_bool().expect("correct")),
            );
        }
    }
    out
}

fn predicted(item: &Item, answer: &Answer) -> (String, bool) {
    match answer {
        Answer::Choice { selected, .. } => {
            let label = item.options[*selected].clone();
            (
                label.clone(),
                item.gold_label.as_deref() == Some(label.as_str()),
            )
        }
        Answer::Noul { yes, .. } => (
            if *yes {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
            item.gold_yes == Some(*yes),
        ),
        Answer::Score { .. } => panic!("eval items are choice/noul only"),
    }
}

#[test]
#[ignore = "live gate: needs the kev-serve unit running"]
fn kev_live_gate() {
    let endpoint =
        std::env::var("KEV_GATE_ENDPOINT").unwrap_or_else(|_| DEFAULT_KEV_ENDPOINT.to_owned());
    let questions_path =
        std::env::var("KEV_GATE_QUESTIONS").unwrap_or_else(|_| DEFAULT_QUESTIONS.to_owned());
    let summary_path = std::env::var("KEV_GATE_SUMMARY").unwrap_or_else(|_| {
        let dir = questions_path
            .rsplit_once('/')
            .map_or(".", |(dir, _)| dir)
            .to_owned();
        format!("{dir}/artefacts/probe1-summary.json")
    });
    let items = load_items(&questions_path);
    let eval = load_eval(&summary_path);
    let scorer = TriageScorer::new(KevBackend::new(&KevConfig::new(&endpoint)).unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    let (mut accepted, mut accepted_correct, mut escalated) = (0_usize, 0_usize, 0_usize);
    let (mut esc_bare_correct, mut esc_inc_correct) = (0_usize, 0_usize);
    let (mut backend_errors, mut agreements) = (0_usize, 0_usize);
    let mut max_delta = 0.0_f64;
    let mut total_latency = 0.0_f64;

    for item in &items {
        let (first, second, latency) = runtime.block_on(async {
            let start = Instant::now();
            let first = scorer.route(&item.batch).await;
            let latency = start.elapsed().as_secs_f64();
            let second = scorer.route(&item.batch).await;
            (first, second, latency)
        });
        total_latency += latency;
        let verdict = first.verdict("q").expect("verdict for q");
        let second_verdict = second.verdict("q").expect("verdict for q");
        // Determinism: same verdict kind, same answer, same confidence.
        match (verdict, second_verdict) {
            (TriageVerdict::Accepted(a), TriageVerdict::Accepted(b)) => {
                assert_eq!(a, b, "{}: repeated answers differ", item.id);
            }
            (
                TriageVerdict::Escalated(TriageEscalation::LowConfidence { confidence: c1 }),
                TriageVerdict::Escalated(TriageEscalation::LowConfidence { confidence: c2 }),
            ) => {
                max_delta = max_delta.max((c1 - c2).abs());
            }
            _ => panic!(
                "{}: repeated verdicts differ: {verdict:?} vs {second_verdict:?}",
                item.id
            ),
        }
        let (bare_pred, bare_correct) = &eval[&(item.id.clone(), "kev".to_owned())];
        let (_, inc_correct) = &eval[&(item.id.clone(), "incumbent".to_owned())];
        match verdict {
            TriageVerdict::Accepted(answer) => {
                accepted += 1;
                let (pred, correct) = predicted(item, answer);
                if correct {
                    accepted_correct += 1;
                }
                if pred == *bare_pred {
                    agreements += 1;
                }
                println!(
                    "{} ACCEPT pred={} conf={:.4} correct={} (bare {} correct={})",
                    item.id,
                    pred,
                    answer.confidence(),
                    correct,
                    bare_pred,
                    bare_correct
                );
            }
            TriageVerdict::Escalated(reason) => {
                escalated += 1;
                if matches!(reason, TriageEscalation::Backend(_)) {
                    backend_errors += 1;
                }
                if *bare_correct {
                    esc_bare_correct += 1;
                }
                if *inc_correct {
                    esc_inc_correct += 1;
                }
                println!(
                    "{} ESCALATE {} (bare pred={} correct={}; incumbent correct={})",
                    item.id, reason, bare_pred, bare_correct, inc_correct
                );
            }
        }
    }

    let n = items.len();
    let end_to_end = accepted_correct + esc_inc_correct;
    println!("== kev live gate: {n} items via {endpoint} ==");
    println!(
        "accepted {accepted}/{n}, escalated {escalated}/{n} (rate {:.3})",
        escalated as f64 / n as f64
    );
    println!("accepted accuracy {accepted_correct}/{accepted}");
    println!(
        "escalated subset: bare-model accuracy {esc_bare_correct}/{escalated}, incumbent accuracy {esc_inc_correct}/{escalated}"
    );
    println!("end-to-end (accepted + incumbent on escalated): {end_to_end}/{n}");
    println!("agreement accepted-vs-bare predictions: {agreements}/{accepted}");
    println!("backend-error escalations: {backend_errors}");
    println!(
        "mean route latency: {:.1} ms",
        total_latency * 1000.0 / n as f64
    );
    println!("max determinism delta (escalated confidences): {max_delta:.2e}");

    assert_eq!(backend_errors, 0, "the live unit must answer every item");
    assert!(accepted > 0, "the gate must accept something");
    assert!(
        max_delta < 1e-6,
        "repeated confidences must be deterministic"
    );
}
