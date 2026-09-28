//! The fake platform: submission intake, scripted 429s, and scripted
//! triage events. Models platform *mechanics* (states, rate limits,
//! events), not any real platform's API.

use std::collections::VecDeque;

/// Triage verdicts the platform can emit. `Unknown` carries the raw
/// string: the tracker records it without mapping it (task 147).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriageKind {
    NeedsMoreInfo,
    Accepted,
    DuplicateOf(String),
    Closed,
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct TriageEvent {
    pub finding_id: String,
    pub kind: TriageKind,
    pub at: u64,
}

/// Scripted platform. `submit` records payloads; `poll_events` drains
/// the scripted triage queue; `next_submit_429` makes the next submit
/// call answer 429 (for the scheduler's backoff arm).
pub struct FakePlatform {
    submissions: Vec<Vec<u8>>,
    events: VecDeque<TriageEvent>,
    fail_next_submit: bool,
    submit_calls: u64,
}

impl FakePlatform {
    pub fn new() -> Self {
        FakePlatform {
            submissions: Vec::new(),
            events: VecDeque::new(),
            fail_next_submit: false,
            submit_calls: 0,
        }
    }

    pub fn queue_event(&mut self, e: TriageEvent) {
        self.events.push_back(e);
    }

    pub fn fail_next_submit(&mut self) {
        self.fail_next_submit = true;
    }

    /// Returns Ok(submission id) or Err(429).
    pub fn submit(&mut self, payload: &[u8]) -> Result<String, u16> {
        self.submit_calls += 1;
        if self.fail_next_submit {
            self.fail_next_submit = false;
            return Err(429);
        }
        self.submissions.push(payload.to_vec());
        Ok(format!("sub{:06}", self.submissions.len()))
    }

    pub fn poll_events(&mut self) -> Vec<TriageEvent> {
        self.events.drain(..).collect()
    }

    pub fn submission_count(&self) -> usize {
        self.submissions.len()
    }

    pub fn submit_calls(&self) -> u64 {
        self.submit_calls
    }

    pub fn last_payload(&self) -> Option<&[u8]> {
        self.submissions.last().map(|v| v.as_slice())
    }
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}
