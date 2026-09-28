//! The agent inbox, adapted from tuios's `internal/session/verb_mailbox.go`.
//!
//! A bounded, in-memory, per-session ring of messages agents leave for each
//! other, plus the loop guards that keep two agents holding each other's
//! address from running away. Store-and-forward: the ring keeps backfill for
//! agents that drive the daemon through one-shot calls and are never
//! subscribed at publish time.
//!
//! Deliberately not durable: a message dies with the daemon. A restored
//! session brings back panes whose shells are new, so a queued instruction
//! surviving into one would be addressed to an agent that no longer holds the
//! context the instruction assumed.
//!
//! Addressing rules, all enforced here:
//!
//! - [`HUMAN`] is the reserved inbox of the person at the attached client.
//!   Only a connection the daemon knows is human may send *from* it; human
//!   questions are fenced as data and only a human answers as human.
//! - No self-address: a pane messaging itself is refused with `loop_refused`.
//! - Ask edges form a graph; an ask that would close a cycle is refused with
//!   `loop_refused` before the edge is added, not after the loop has run.
//! - Each sender is rate-limited: a burst of [`SEND_BURST`] sends, refilling at
//!   [`SENDS_PER_MINUTE`] per minute; over the cap is `rate_limited`.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::error::{ErrorCode, TuiosError};

/// Reserved inbox id of the person at the attached client. It is the one
/// address that is not a window: a message to it is read from the client's
/// mail view. It cannot be asked, because there is no keyboard behind it.
pub const HUMAN: &str = "human";

/// Bound: one message body, in bytes. Bigger than a paragraph, smaller than
/// a file; a message that wants to carry a file attaches a path instead.
pub const MESSAGE_TEXT_BYTES_MAX: usize = 8 * 1024;
/// Bound: the one-line summary a reader scans, in characters.
pub const MESSAGE_SUBJECT_CHARS_MAX: usize = 120;
/// Bound: attachment path references carried by one message.
pub const MESSAGE_ATTACHMENTS_MAX: usize = 8;
/// Bound: one attachment path reference, in bytes.
pub const ATTACHMENT_PATH_BYTES_MAX: usize = 4 * 1024;
/// Bound: a session's ring, by message count.
pub const RING_MESSAGES_MAX: usize = 256;
/// Bound: a session's ring, by the text it holds. A full ring of
/// maximum-sized messages still cannot cost more than this.
pub const RING_TEXT_BYTES_MAX: usize = 512 * 1024;
/// How many messages a read answers with when the caller names no limit.
pub const READ_DEFAULT_LIMIT: usize = 20;
/// How many sends one sender may make back to back.
pub const SEND_BURST: u32 = 10;
/// The sustained rate one sender's bucket refills at, per minute.
pub const SENDS_PER_MINUTE: f64 = 30.0;
/// Defensive cap on the ask-graph reachability walk, in steps.
pub const ASK_REACH_STEPS_MAX: usize = 1024;

/// What a message is addressed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Addressed to one window's inbox. Has a recipient and a read state.
    Direct,
    /// Addressed to the session rather than to anyone. Everyone can read it,
    /// nobody owns it, and it is never unread for a particular reader.
    Notice,
    /// The record of one ask-agent call: the question in the subject, what
    /// the pane printed in answer in the text, and `settled_by` saying which
    /// signal ended the wait. Written after the ask finishes, so the person
    /// can see exchanges that happened between two agents' keyboards.
    Ask,
}

/// One message in a session's ring.
#[derive(Debug, Clone)]
pub struct Message {
    id: u64,
    kind: MessageKind,
    from: String,
    to: Option<String>,
    subject: String,
    text: String,
    attachments: Vec<String>,
    thread_id: u64,
    settled_by: Option<String>,
    read: bool,
}

impl Message {
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
    #[must_use]
    pub fn kind(&self) -> MessageKind {
        self.kind
    }
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }
    #[must_use]
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
    #[must_use]
    pub fn thread_id(&self) -> u64 {
        self.thread_id
    }
    #[must_use]
    pub fn settled_by(&self) -> Option<&str> {
        self.settled_by.as_deref()
    }
    #[must_use]
    pub fn attachments(&self) -> &[String] {
        &self.attachments
    }
}

/// Filter for [`Mailbox::read`].
#[derive(Debug, Clone, Default)]
pub struct ReadFilter<'a> {
    /// Read this inbox. `None` reads everything and marks nothing read.
    pub inbox: Option<&'a str>,
    /// Return only directed messages nobody has read yet.
    pub unread: bool,
    /// Include session-wide notices in an inbox read.
    pub notices: bool,
    /// Return only the messages in this thread.
    pub thread: Option<u64>,
    /// Maximum messages to return; defaults to [`READ_DEFAULT_LIMIT`].
    pub limit: Option<usize>,
    /// Read without marking anything read.
    pub peek: bool,
}

/// One session's bounded message ring.
#[derive(Debug)]
pub struct Mailbox {
    messages: VecDeque<Message>,
    /// Sum of `text.len()` over the ring, for the byte bound.
    text_bytes: usize,
    next_id: u64,
    senders: HashMap<String, RateLimiter>,
}

impl Mailbox {
    #[must_use]
    pub fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            text_bytes: 0,
            next_id: 1,
            senders: HashMap::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Leave a message in the ring. `to = None` posts a session notice.
    /// `is_human_origin` must be true exactly when the sending connection is
    /// the person; a pane claiming `from = "human"` is refused.
    ///
    /// # Errors
    ///
    /// `invalid_params` on over-bound text/subject/attachments,
    /// `loop_refused` on self-address, `forbidden` on a human spoof,
    /// `rate_limited` over the sender's cap.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &mut self,
        kind: MessageKind,
        from: &str,
        to: Option<&str>,
        subject: &str,
        text: &str,
        attachments: &[String],
        is_human_origin: bool,
        now: Instant,
    ) -> Result<u64, TuiosError> {
        validate_send(from, to, subject, text, attachments)?;
        if from == HUMAN && !is_human_origin {
            return Err(TuiosError::protocol(
                ErrorCode::Forbidden,
                "only the person may send as human",
            ));
        }
        if let Some(recipient) = to
            && recipient == from
        {
            return Err(TuiosError::protocol(
                ErrorCode::LoopRefused,
                "a pane cannot address itself",
            ));
        }
        let limiter = self
            .senders
            .entry(from.to_owned())
            .or_insert_with(|| RateLimiter::new(now));
        if !limiter.allow(now) {
            return Err(TuiosError::protocol(
                ErrorCode::RateLimited,
                "sender is over the message rate cap; wait and retry",
            ));
        }
        Ok(self.push(kind, from, to, subject, text, attachments))
    }

    /// Record a daemon-generated message (an ask record) without consulting
    /// the sender's rate limiter. Bounds and addressing rules still apply:
    /// history the daemon writes must not be a back door around them.
    pub(crate) fn record(
        &mut self,
        kind: MessageKind,
        from: &str,
        to: Option<&str>,
        subject: &str,
        text: &str,
        settled_by: Option<&str>,
    ) -> Result<u64, TuiosError> {
        validate_send(from, to, subject, text, &[])?;
        if let Some(recipient) = to
            && recipient == from
        {
            return Err(TuiosError::protocol(
                ErrorCode::LoopRefused,
                "a pane cannot address itself",
            ));
        }
        let id = self.push(kind, from, to, subject, text, &[]);
        if let Some(message) = self.messages.back_mut() {
            debug_assert!(message.id == id);
            message.settled_by = settled_by.map(str::to_owned);
        }
        Ok(id)
    }

    /// Push a validated message, evicting the oldest first. Returns the id.
    fn push(
        &mut self,
        kind: MessageKind,
        from: &str,
        to: Option<&str>,
        subject: &str,
        text: &str,
        attachments: &[String],
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.evict_until_fits(text.len());
        let thread_id = id;
        self.messages.push_back(Message {
            id,
            kind,
            from: from.to_owned(),
            to: to.map(str::to_owned),
            subject: subject.to_owned(),
            text: text.to_owned(),
            attachments: attachments.to_vec(),
            thread_id,
            settled_by: Option::None,
            read: false,
        });
        self.text_bytes += text.len();
        id
    }

    /// Read the ring under `filter`. Naming an inbox marks the directed
    /// messages returned as read, unless `peek`.
    pub fn read(&mut self, filter: &ReadFilter<'_>) -> Vec<Message> {
        let limit = filter
            .limit
            .unwrap_or(READ_DEFAULT_LIMIT)
            .min(RING_MESSAGES_MAX);
        let mut out = Vec::new();
        for message in self.messages.iter_mut() {
            if out.len() >= limit {
                break;
            }
            if !message_matches(message, filter) {
                continue;
            }
            if !filter.peek && filter.inbox.is_some() && message.kind == MessageKind::Direct {
                message.read = true;
            }
            out.push(message.clone());
        }
        out
    }

    /// Make room for `incoming_bytes` of new text, evicting the oldest
    /// messages first. The ring bound is by count and by bytes together.
    fn evict_until_fits(&mut self, incoming_bytes: usize) {
        while !self.messages.is_empty()
            && (self.messages.len() >= RING_MESSAGES_MAX
                || self.text_bytes + incoming_bytes > RING_TEXT_BYTES_MAX)
        {
            if let Some(old) = self.messages.pop_front() {
                self.text_bytes -= old.text.len();
            }
        }
        debug_assert!(self.text_bytes <= RING_TEXT_BYTES_MAX || self.messages.is_empty());
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_send(
    from: &str,
    _to: Option<&str>,
    subject: &str,
    text: &str,
    attachments: &[String],
) -> Result<(), TuiosError> {
    if from.is_empty() {
        return Err(TuiosError::protocol(
            ErrorCode::InvalidParams,
            "message needs a sender",
        ));
    }
    if text.len() > MESSAGE_TEXT_BYTES_MAX {
        return Err(TuiosError::protocol(
            ErrorCode::InvalidParams,
            "text is longer than the message cap",
        ));
    }
    if subject.chars().count() > MESSAGE_SUBJECT_CHARS_MAX {
        return Err(TuiosError::protocol(
            ErrorCode::InvalidParams,
            "subject is longer than 120 characters",
        ));
    }
    if attachments.len() > MESSAGE_ATTACHMENTS_MAX {
        return Err(TuiosError::protocol(
            ErrorCode::InvalidParams,
            "message carries more than 8 attachments",
        ));
    }
    for path in attachments {
        if path.len() > ATTACHMENT_PATH_BYTES_MAX {
            return Err(TuiosError::protocol(
                ErrorCode::InvalidParams,
                "attachment path is longer than the cap",
            ));
        }
    }
    Ok(())
}

fn message_matches(message: &Message, filter: &ReadFilter<'_>) -> bool {
    if let Some(thread) = filter.thread
        && message.thread_id != thread
    {
        return false;
    }
    match filter.inbox {
        Option::None => true,
        Some(inbox) => match message.kind {
            MessageKind::Direct => {
                if message.to.as_deref() != Some(inbox) {
                    return false;
                }
                if filter.unread && message.read {
                    return false;
                }
                true
            }
            MessageKind::Notice => filter.notices,
            // Ask records are session-visible history, not inbox mail.
            MessageKind::Ask => false,
        },
    }
}

/// Token-bucket rate limiter for one sender: a burst of [`SEND_BURST`]
/// sends, refilling at [`SENDS_PER_MINUTE`] per minute. Time is a parameter
/// so tests control the clock; production passes `Instant::now()`.
#[derive(Debug)]
pub struct RateLimiter {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: f64::from(SEND_BURST),
            last: now,
        }
    }

    /// Take one token if any is available.
    pub fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last);
        self.tokens = (self.tokens + elapsed.as_secs_f64() * (SENDS_PER_MINUTE / 60.0))
            .min(f64::from(SEND_BURST));
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The open ask edges between agents. An ask that would close a cycle is
/// refused before the edge is added: two agents holding each other's address
/// is the failure mode this exists to stop. Keyed on claimed window ids, so
/// it is exactly as trustworthy as the claim — enough to stop an
/// orchestrator wiring A to B to A by mistake.
#[derive(Debug, Default)]
pub struct AskGraph {
    edges: HashMap<String, HashMap<String, u32>>,
}

impl AskGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an in-flight ask from `from` to `to`.
    ///
    /// # Errors
    ///
    /// `loop_refused` when `from == to` or when the edge would close a cycle
    /// with an ask already in flight.
    pub fn open_ask(&mut self, from: &str, to: &str) -> Result<(), TuiosError> {
        if from.is_empty() || to.is_empty() {
            return Err(TuiosError::protocol(
                ErrorCode::InvalidParams,
                "ask needs both ends",
            ));
        }
        if from == to || self.reaches(to, from) {
            return Err(TuiosError::protocol(
                ErrorCode::LoopRefused,
                "ask would close a cycle with one already in flight",
            ));
        }
        *self
            .edges
            .entry(from.to_owned())
            .or_default()
            .entry(to.to_owned())
            .or_insert(0) += 1;
        Ok(())
    }

    /// Release an edge [`AskGraph::open_ask`] took. Closing what was never
    /// opened is a no-op, not an error.
    pub fn close_ask(&mut self, from: &str, to: &str) {
        let Some(targets) = self.edges.get_mut(from) else {
            return;
        };
        let remove_source = match targets.get_mut(to) {
            Option::None => false,
            Some(count) => {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    targets.remove(to);
                }
                targets.is_empty()
            }
        };
        if remove_source {
            self.edges.remove(from);
        }
    }

    /// Whether `src` can reach `dst` by following open ask edges. The graph
    /// holds one node per agent currently blocked in an ask, so it is tiny;
    /// the walk is still capped at [`ASK_REACH_STEPS_MAX`] steps.
    fn reaches(&self, src: &str, dst: &str) -> bool {
        if src == dst {
            return true;
        }
        let mut seen = HashMap::from([(src.to_owned(), ())]);
        let mut queue = VecDeque::from([src.to_owned()]);
        let mut steps = 0;
        while let Some(node) = queue.pop_front() {
            steps += 1;
            if steps > ASK_REACH_STEPS_MAX {
                return false;
            }
            if let Some(targets) = self.edges.get(&node) {
                for next in targets.keys() {
                    if next == dst {
                        return true;
                    }
                    if !seen.contains_key(next) {
                        seen.insert(next.clone(), ());
                        queue.push_back(next.clone());
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn now() -> Instant {
        Instant::now()
    }

    // --- validation ---

    #[test]
    fn send_and_read_round_trip() {
        let mut box_ = Mailbox::new();
        let id = box_
            .send(
                MessageKind::Direct,
                "agent-a",
                Some("agent-b"),
                "hello",
                "task context",
                &[],
                false,
                now(),
            )
            .expect("send must succeed");
        let got = box_.read(&ReadFilter {
            inbox: Some("agent-b"),
            ..Default::default()
        });
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id(), id);
        assert_eq!(got[0].text(), "task context");
    }

    #[test]
    fn notice_is_read_without_an_inbox_and_marks_nothing() {
        let mut box_ = Mailbox::new();
        box_.send(
            MessageKind::Notice,
            "agent-a",
            Option::None,
            "",
            "all hands",
            &[],
            false,
            now(),
        )
        .expect("notice must succeed");
        let all = box_.read(&ReadFilter::default());
        assert_eq!(all.len(), 1);
        let inbox = box_.read(&ReadFilter {
            inbox: Some("agent-b"),
            ..Default::default()
        });
        assert!(
            inbox.is_empty(),
            "notices need the notices flag in an inbox read"
        );
        let with = box_.read(&ReadFilter {
            inbox: Some("agent-b"),
            notices: true,
            ..Default::default()
        });
        assert_eq!(with.len(), 1);
    }

    #[test]
    fn unread_filter_and_peek() {
        let mut box_ = Mailbox::new();
        for i in 0..3 {
            box_.send(
                MessageKind::Direct,
                "a",
                Some("b"),
                "",
                &format!("m{i}"),
                &[],
                false,
                now(),
            )
            .expect("send");
        }
        let peeked = box_.read(&ReadFilter {
            inbox: Some("b"),
            peek: true,
            ..Default::default()
        });
        assert_eq!(peeked.len(), 3);
        let unread = box_.read(&ReadFilter {
            inbox: Some("b"),
            unread: true,
            ..Default::default()
        });
        assert_eq!(unread.len(), 3, "peek must not mark read");
        let _ = box_.read(&ReadFilter {
            inbox: Some("b"),
            ..Default::default()
        });
        let unread = box_.read(&ReadFilter {
            inbox: Some("b"),
            unread: true,
            ..Default::default()
        });
        assert!(unread.is_empty(), "a real read marks directed mail read");
    }

    #[test]
    fn ring_evicts_oldest_under_count_bound() {
        let mut box_ = Mailbox::new();
        for i in 0..=RING_MESSAGES_MAX {
            let from = format!("sender-{i}");
            box_.send(
                MessageKind::Direct,
                &from,
                Some("b"),
                "",
                &format!("msg-{i:04}"),
                &[],
                false,
                now(),
            )
            .expect("send");
        }
        assert_eq!(box_.len(), RING_MESSAGES_MAX);
        let all = box_.read(&ReadFilter {
            limit: Some(RING_MESSAGES_MAX),
            ..Default::default()
        });
        assert_eq!(all[0].text(), "msg-0001", "oldest must be evicted first");
        assert_eq!(
            all[all.len() - 1].text(),
            format!("msg-{RING_MESSAGES_MAX:04}")
        );
    }

    #[test]
    fn ring_evicts_oldest_under_byte_bound() {
        let mut box_ = Mailbox::new();
        let big = "x".repeat(MESSAGE_TEXT_BYTES_MAX);
        // 65 max-size bodies exceed the 512 KiB text bound.
        for i in 0..65 {
            let from = format!("bulk-{i}");
            box_.send(
                MessageKind::Direct,
                &from,
                Some("b"),
                "",
                &big,
                &[],
                false,
                now(),
            )
            .expect("send");
        }
        let total: usize = box_
            .read(&ReadFilter::default())
            .iter()
            .map(|m| m.text().len())
            .sum();
        assert!(total <= RING_TEXT_BYTES_MAX, "byte bound holds: {total}");
        assert!(box_.len() < 65, "oldest messages were evicted");
    }

    #[test]
    fn rate_limiter_refills_over_time() {
        let start = now();
        let mut limiter = RateLimiter::new(start);
        for _ in 0..SEND_BURST {
            assert!(limiter.allow(start));
        }
        assert!(!limiter.allow(start), "burst exhausted");
        let later = start + Duration::from_secs(60);
        assert!(limiter.allow(later), "a minute refills the bucket");
    }

    #[test]
    fn ask_open_close_cycle() {
        let mut graph = AskGraph::new();
        graph.open_ask("a", "b").expect("open");
        graph.open_ask("a", "b").expect("parallel edge");
        graph.close_ask("a", "b");
        // One edge still open: b -> a would now close a cycle.
        assert!(graph.open_ask("b", "a").is_err());
        graph.close_ask("a", "b");
        graph.open_ask("b", "a").expect("closed edges release");
    }

    #[test]
    fn read_limit_defaults_and_caps() {
        let mut box_ = Mailbox::new();
        for i in 0..30 {
            // Distinct senders: this test is about read limits, and one
            // sender's burst cap would rate-limit the fixture.
            let from = format!("reader-{i}");
            box_.send(
                MessageKind::Direct,
                &from,
                Some("b"),
                "",
                &format!("{i}"),
                &[],
                false,
                now(),
            )
            .expect("send");
        }
        let got = box_.read(&ReadFilter {
            inbox: Some("b"),
            ..Default::default()
        });
        assert_eq!(got.len(), READ_DEFAULT_LIMIT);
    }

    // --- adversarial ---

    #[test]
    fn oversize_text_is_refused() {
        let mut box_ = Mailbox::new();
        let err = box_
            .send(
                MessageKind::Direct,
                "a",
                Some("b"),
                "",
                &"x".repeat(MESSAGE_TEXT_BYTES_MAX + 1),
                &[],
                false,
                now(),
            )
            .expect_err("oversize text must fail");
        assert_eq!(err.code(), ErrorCode::InvalidParams);
    }

    #[test]
    fn oversize_subject_is_refused() {
        let mut box_ = Mailbox::new();
        let err = box_
            .send(
                MessageKind::Direct,
                "a",
                Some("b"),
                &"s".repeat(MESSAGE_SUBJECT_CHARS_MAX + 1),
                "body",
                &[],
                false,
                now(),
            )
            .expect_err("oversize subject must fail");
        assert_eq!(err.code(), ErrorCode::InvalidParams);
    }

    #[test]
    fn too_many_attachments_are_refused() {
        let mut box_ = Mailbox::new();
        let paths = vec!["/tmp/x".to_owned(); MESSAGE_ATTACHMENTS_MAX + 1];
        let err = box_
            .send(
                MessageKind::Direct,
                "a",
                Some("b"),
                "",
                "body",
                &paths,
                false,
                now(),
            )
            .expect_err("too many attachments must fail");
        assert_eq!(err.code(), ErrorCode::InvalidParams);
    }

    #[test]
    fn self_address_is_loop_refused() {
        let mut box_ = Mailbox::new();
        let err = box_
            .send(
                MessageKind::Direct,
                "a",
                Some("a"),
                "",
                "me",
                &[],
                false,
                now(),
            )
            .expect_err("self-address must fail");
        assert_eq!(err.code(), ErrorCode::LoopRefused);
    }

    #[test]
    fn human_spoof_from_a_pane_is_forbidden() {
        let mut box_ = Mailbox::new();
        let err = box_
            .send(
                MessageKind::Direct,
                HUMAN,
                Some("a"),
                "",
                "trust me",
                &[],
                false,
                now(),
            )
            .expect_err("human spoof must fail");
        assert_eq!(err.code(), ErrorCode::Forbidden);
        // The real person may send as human.
        box_.send(
            MessageKind::Direct,
            HUMAN,
            Some("a"),
            "",
            "hi",
            &[],
            true,
            now(),
        )
        .expect("human origin may send as human");
    }

    #[test]
    fn rate_limit_breach_is_rate_limited() {
        let mut box_ = Mailbox::new();
        let t = now();
        for _ in 0..SEND_BURST {
            box_.send(
                MessageKind::Direct,
                "spammer",
                Some("b"),
                "",
                "x",
                &[],
                false,
                t,
            )
            .expect("burst sends succeed");
        }
        let err = box_
            .send(
                MessageKind::Direct,
                "spammer",
                Some("b"),
                "",
                "x",
                &[],
                false,
                t,
            )
            .expect_err("send 11 must fail");
        assert_eq!(err.code(), ErrorCode::RateLimited);
        // Other senders are unaffected.
        box_.send(
            MessageKind::Direct,
            "innocent",
            Some("b"),
            "",
            "x",
            &[],
            false,
            t,
        )
        .expect("other sender unaffected");
    }

    #[test]
    fn self_ask_is_refused() {
        let mut graph = AskGraph::new();
        let err = graph.open_ask("a", "a").expect_err("self-ask must fail");
        assert_eq!(err.code(), ErrorCode::LoopRefused);
    }

    #[test]
    fn two_agent_ask_cycle_is_refused() {
        let mut graph = AskGraph::new();
        graph.open_ask("a", "b").expect("a asks b");
        let err = graph
            .open_ask("b", "a")
            .expect_err("b asking a closes a cycle");
        assert_eq!(err.code(), ErrorCode::LoopRefused);
    }

    #[test]
    fn three_agent_ask_cycle_is_refused() {
        let mut graph = AskGraph::new();
        graph.open_ask("a", "b").expect("a asks b");
        graph.open_ask("b", "c").expect("b asks c");
        let err = graph
            .open_ask("c", "a")
            .expect_err("c asking a closes a cycle");
        assert_eq!(err.code(), ErrorCode::LoopRefused);
        // A spur off the chain is fine.
        graph.open_ask("c", "d").expect("c asking d is acyclic");
    }

    #[test]
    fn close_without_open_is_a_no_op() {
        let mut graph = AskGraph::new();
        graph.close_ask("ghost", "nobody");
        graph.open_ask("a", "b").expect("still works after");
    }
}
