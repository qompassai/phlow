//! Contract tests for the Kev-4B backend (llama.cpp `/v1/systemone`
//! dialect) and the D-1622 triage gate, against a scripted loopback
//! server speaking the exact response shapes recorded in the
//! D-1594 eval artefacts. Roughly half validation, half adversarial:
//! the adversarial half proves the fail-safe — HTTP errors, garbage
//! bodies, out-of-contract answers and a dead server all escalate
//! every question, so the caller's incumbent path runs unchanged.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use phlow_system1::{
    Answer, KevBackend, KevConfig, Question, QuestionBatch, System1Decider, System1Error,
    TriageEscalation, TriageScorer, TriageVerdict,
};
use serde_json::{Value, json};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
}

/// Serve one canned HTTP response on 127.0.0.1; the handle yields the
/// raw request bytes.
fn serve_once(status: &str, body: Vec<u8>) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        request
    });
    (endpoint, handle)
}

fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    while request.len() < 1 << 20 {
        let read = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&request).to_string();
        if let Some(split) = text.find("\r\n\r\n") {
            let length = text[..split]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= split + 4 + length || read == 0 {
                break;
            }
        }
        if read == 0 {
            break;
        }
    }
    request
}

fn backend_for(endpoint: &str) -> KevBackend {
    KevBackend::new(&KevConfig::new(endpoint)).unwrap()
}

fn route_batch() -> QuestionBatch {
    QuestionBatch {
        state: "Ticket T-1042 from a customer: the export button returns HTTP 500 \
                on large files."
            .to_owned(),
        questions: BTreeMap::from([(
            "route".to_owned(),
            Question::Choice {
                instructions: "Which handling does this ticket need?".to_owned(),
                options: vec![
                    "resolved".to_owned(),
                    "needs-engineer".to_owned(),
                    "needs-info".to_owned(),
                ],
                option_descriptions: Vec::new(),
            },
        )]),
    }
}

/// The eval's V1 response shape, verbatim field layout: the choice
/// answer's own `confidence` is the head's margin measure, and the
/// probability of the chosen option lives in `probabilities`.
fn v1_reply() -> Value {
    json!({
        "model": "kev-4b",
        "answers": {"route": {
            "type": "choice", "choice": "needs-engineer",
            "probabilities": {
                "resolved": 0.0515, "needs-engineer": 0.8589, "needs-info": 0.0896,
            },
            "confidence": 0.8118,
        }},
        "usage": {"input_tokens": 187, "output_tokens": 0},
    })
}

#[test]
fn kev_backend_decodes_the_llama_dialect() {
    let (endpoint, server) = serve_once("200 OK", v1_reply().to_string().into_bytes());
    let backend = backend_for(&endpoint);
    let answers = runtime().block_on(backend.decide(&route_batch())).unwrap();
    assert_eq!(
        answers.answers["route"],
        Answer::Choice {
            selected: 1,
            probability: 0.8589
        }
    );
    // The request that reached the server is the llama.cpp dialect:
    // lowercase type tag, criteria object keyed by option label, and
    // no model field.
    let request = String::from_utf8(server.join().unwrap()).unwrap();
    let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(request.starts_with("POST /v1/systemone "));
    assert_eq!(body["questions"]["route"]["type"], "choice");
    assert_eq!(
        body["questions"]["route"]["criteria"],
        json!({"resolved": null, "needs-engineer": null, "needs-info": null})
    );
    assert!(body.get("model").is_none());
}

#[test]
fn kev_backend_normalizes_score_values_end_to_end() {
    let batch = QuestionBatch {
        state: "Change request: rotate the production database credentials.".to_owned(),
        questions: BTreeMap::from([(
            "risk".to_owned(),
            Question::Score {
                instructions: "Rate the risk.".to_owned(),
                criteria: vec![
                    "low".to_owned(),
                    "medium".to_owned(),
                    "high".to_owned(),
                    "critical".to_owned(),
                ],
            },
        )]),
    };
    // Expected level index 2.0 of 0..=3 normalizes to 2/3.
    let reply = json!({
        "model": "kev-4b",
        "answers": {"risk": {
            "type": "score", "score": 2.0,
            "legend": {"0": "low", "1": "medium", "2": "high", "3": "critical"},
            "probabilities": {"0": 0.1, "1": 0.2, "2": 0.4, "3": 0.3},
            "confidence": 0.55,
        }},
        "usage": {"input_tokens": 96, "output_tokens": 0},
    });
    let (endpoint, _server) = serve_once("200 OK", reply.to_string().into_bytes());
    let backend = backend_for(&endpoint);
    let answers = runtime().block_on(backend.decide(&batch)).unwrap();
    assert_eq!(
        answers.answers["risk"],
        Answer::Score {
            value: 2.0 / 3.0,
            confidence: 0.55
        }
    );
}

#[test]
fn triage_accepts_a_confident_verdict_from_the_live_dialect() {
    let (endpoint, _server) = serve_once("200 OK", v1_reply().to_string().into_bytes());
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(
        outcome.verdict("route"),
        Some(&TriageVerdict::Accepted(Answer::Choice {
            selected: 1,
            probability: 0.8589
        }))
    );
}

// ---- adversarial: every failure shape escalates everything ----

#[test]
fn http_error_status_escalates_everything() {
    let (endpoint, _server) = serve_once("500 Internal Server Error", b"boom".to_vec());
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(outcome.accepted(), 0);
    assert!(matches!(
        outcome.verdict("route"),
        Some(TriageVerdict::Escalated(TriageEscalation::Backend(
            System1Error::Transport { .. }
        )))
    ));
}

#[test]
fn garbage_body_escalates_everything() {
    let (endpoint, _server) = serve_once("200 OK", b"this is not json".to_vec());
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(outcome.accepted(), 0);
    assert!(matches!(
        outcome.verdict("route"),
        Some(TriageVerdict::Escalated(TriageEscalation::Backend(
            System1Error::Protocol { .. }
        )))
    ));
}

#[test]
fn unknown_option_label_escalates_everything() {
    let reply = json!({
        "model": "kev-4b",
        "answers": {"route": {
            "type": "choice", "choice": "auto-close",
            "probabilities": {"auto-close": 0.99},
            "confidence": 0.99,
        }},
        "usage": {"input_tokens": 187, "output_tokens": 0},
    });
    let (endpoint, _server) = serve_once("200 OK", reply.to_string().into_bytes());
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(outcome.accepted(), 0);
}

#[test]
fn missing_answer_escalates_everything() {
    let reply = json!({
        "model": "kev-4b",
        "answers": {},
        "usage": {"input_tokens": 187, "output_tokens": 0},
    });
    let (endpoint, _server) = serve_once("200 OK", reply.to_string().into_bytes());
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(outcome.accepted(), 0);
    assert_eq!(outcome.escalated(), 1);
}

#[test]
fn service_down_escalates_everything() {
    // Bind then drop a listener so the port refuses connections:
    // the Kev service being down must look exactly like no System 1.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let scorer = TriageScorer::new(backend_for(&endpoint));
    let outcome = runtime().block_on(scorer.route(&route_batch()));
    assert_eq!(outcome.accepted(), 0);
    assert!(matches!(
        outcome.verdict("route"),
        Some(TriageVerdict::Escalated(TriageEscalation::Backend(_)))
    ));
}

#[test]
fn kev_backend_sends_descriptions_as_criteria_text() {
    // D-1641: descriptions the question carries reach the server as
    // criteria text; an undescribed option stays null.
    let batch = QuestionBatch {
        state: "Ticket T-1042 from a customer: the export button returns HTTP 500 \
                on large files."
            .to_owned(),
        questions: BTreeMap::from([(
            "route".to_owned(),
            Question::Choice {
                instructions: "Which handling does this ticket need?".to_owned(),
                options: vec![
                    "resolved".to_owned(),
                    "needs-engineer".to_owned(),
                    "needs-info".to_owned(),
                ],
                option_descriptions: vec![
                    Some("The reported problem is already fixed".to_owned()),
                    Some("A defect an engineer must fix".to_owned()),
                    None,
                ],
            },
        )]),
    };
    let (endpoint, server) = serve_once("200 OK", v1_reply().to_string().into_bytes());
    let backend = backend_for(&endpoint);
    let answers = runtime().block_on(backend.decide(&batch)).unwrap();
    assert_eq!(
        answers.answers["route"],
        Answer::Choice {
            selected: 1,
            probability: 0.8589
        }
    );
    let request = String::from_utf8(server.join().unwrap()).unwrap();
    let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body["questions"]["route"]["criteria"],
        json!({
            "resolved": "The reported problem is already fixed",
            "needs-engineer": "A defect an engineer must fix",
            "needs-info": null,
        })
    );
}
