// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Events in, state plus effects out.
//!
//! The adapted pattern (from Ghostex `packages/gx-core/src/core.rs`):
//! a pure transition function `reduce(state, event)` computes the next
//! state and *records* the side effects as data; a separate effect
//! interpreter executes them. The reducer never performs I/O — this
//! module imports zero I/O facilities (statically asserted by
//! task-166's `reducer_imports_no_io` case via source scan; there is
//! nothing to import here: the module body uses only the std prelude).
//!
//! Ghostex's domain model (tabs, sidebar, `FocusSession`) is NOT
//! lifted. The domain here is phlow-relevant: the agent-run lifecycle
//! a TUI / phlow-tuios host would track (queued → running →
//! blocked/done/failed), with effects for persistence and operator
//! notification.
//!
//! Two execution paths, with different contracts:
//! - [`MockInterpreter::interpret_batch`]: in-process handoff straight
//!   from the reducer. Atomic batch reject: every effect is validated
//!   as known BEFORE any is applied; one unknown effect rejects the
//!   whole batch with [`EffectError::Unhandled`] and zero side
//!   effects are recorded. (We chose atomic batch reject over
//!   apply-then-rollback because the reducer already applied the
//!   state half purely — there is nothing to roll back in the
//!   interpreter, only effects to withhold.)
//! - [`MockInterpreter::deliver`]: transport path (daemon → TUI) with
//!   sequence numbers and idempotency keys. Duplicate keyed delivery
//!   → [`EffectError::Duplicate`]; keyless retry →
//!   [`EffectError::RetryRefused`]; out-of-order → bounded hold
//!   buffer, [`EffectError::OutOfOrder`] past the bound.
//!
//! Timing: the reducer takes no clock. When a scenario needs time,
//! the caller passes it explicitly in the event (there is no
//! `ManualClock` import here by design).

/// Max task label bytes accepted at ingestion.
pub const MAX_LABEL_BYTES: usize = 256;
/// Max block/fail reason bytes accepted at ingestion.
pub const MAX_REASON_BYTES: usize = 1_024;
/// Max tasks the machine tracks; a Spawn past this is refused.
pub const MAX_TASKS: usize = 4_096;
/// Hard ceiling on effects one `reduce` call may emit (construction
/// sites emit at most 2; this is the belt-and-braces bound).
pub const MAX_EFFECTS_PER_EVENT: usize = 8;
/// Hold buffer for out-of-order deliveries on the transport path.
pub const HOLD_BUFFER_MAX: usize = 16;
/// Max applied deliveries the mock ledger retains.
pub const MAX_LEDGER_ENTRIES: usize = 65_536;
/// Allocation budget for one hostile-payload ingestion (task-168):
/// the payload is refused on length *before* any copy into state, so
/// a 10 MB hostile field must cost far less than this.
pub const INGEST_ALLOC_BUDGET_BYTES: usize = 64 * 1_024;

/// Lifecycle of one agent run, as the TUI would show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Blocked,
    Done,
    Failed,
}

/// One tracked agent run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: u64,
    pub state: TaskState,
    pub label: String,
    pub progress: u8,
}

/// The whole machine state. Deterministic: no timestamps, no hash
/// maps — `tasks` stays in insertion order and every byte of
/// [`MachineState::canonical_bytes`] derives from inputs alone.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MachineState {
    pub tasks: Vec<TaskRecord>,
    pub version: u64,
}

/// Inputs to the transition function. Every variant is shaped by
/// [`ingest`] before it may reach [`reduce`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Spawn { id: u64, label: String },
    Start { id: u64 },
    Progress { id: u64, percent: u8 },
    Block { id: u64, reason: String },
    Unblock { id: u64 },
    Complete { id: u64 },
    Fail { id: u64, reason: String },
}

/// A side effect the reducer *records*. The reducer never executes
/// these; the interpreter does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Persist one task's row (the "Write" of the [Write, Notify,
    /// Write] scenario).
    PersistTask { task_id: u64 },
    /// Raise a TUI attention item (the "Notify").
    Notify { task_id: u64, message: String },
    /// A reducer extension this interpreter build does not know.
    /// Must be refused with [`EffectError::Unhandled`] — never
    /// silently dropped, never executed inline.
    VendorExtension { name: String },
}

/// What the wire hands the TUI: untrusted, unshaped. [`ingest`]
/// validates it into an [`Event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: String,
    pub task_id: u64,
    pub text: String,
    pub percent: u64,
}

/// The reducer refused a transition: a driver bug, not hostile input
/// (hostile input is stopped earlier, at [`ingest`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReduceError {
    UnknownTask {
        id: u64,
    },
    DuplicateTask {
        id: u64,
    },
    IllegalTransition {
        id: u64,
        from: TaskState,
        event: &'static str,
    },
    TooManyTasks,
    BadProgress {
        percent: u8,
    },
}

/// The ingestion layer refused a raw event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventError {
    /// The event kind is not one the reducer speaks.
    Unknown { kind: String },
    /// A text payload exceeds its named bound. `bytes` is the
    /// refused length — the payload is never copied.
    OversizedPayload {
        field: &'static str,
        bytes: usize,
        max: usize,
    },
    /// The id is a reserved sentinel (0 or `u64::MAX`) and can never
    /// name a real task.
    OutOfRangeIndex { id: u64 },
    /// Percent field outside 0..=100.
    BadPercent { percent: u64 },
}

/// The interpreter refused an effect or delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectError {
    /// No interpreter clause for this effect kind. Loud, never a
    /// silent drop.
    Unhandled { name: String },
    /// A keyed effect delivered twice: typed no-op, zero
    /// double-applied side effects.
    Duplicate { key: String },
    /// A keyless effect presented as a retry. Keyless effects are
    /// declared non-idempotent: the interpreter cannot dedupe what
    /// it cannot identify, so it refuses instead of double-applying.
    RetryRefused { seq: u64 },
    /// A delivery outside the expected sequence: stale (below
    /// `expected`) or past the hold buffer's bound (above it).
    OutOfOrder { seq: u64, expected: u64 },
    /// The mock ledger is full; refusing beats silent eviction.
    LedgerFull,
}

fn find_task(tasks: &[TaskRecord], id: u64) -> Option<usize> {
    tasks.iter().position(|t| t.id == id)
}

fn state_tag(state: TaskState) -> &'static str {
    match state {
        TaskState::Queued => "queued",
        TaskState::Running => "running",
        TaskState::Blocked => "blocked",
        TaskState::Done => "done",
        TaskState::Failed => "failed",
    }
}

fn effect_tag(effect: &Effect) -> &'static str {
    match effect {
        Effect::PersistTask { .. } => "persist",
        Effect::Notify { .. } => "notify",
        Effect::VendorExtension { .. } => "vendor",
    }
}

/// Escape the two structural bytes out of a label so
/// [`MachineState::canonical_bytes`] stays unambiguous.
fn escape_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for c in label.chars() {
        if c == ':' || c == ';' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl MachineState {
    /// Deterministic byte encoding of the state: field order fixed,
    /// tasks in insertion order. Two replays are "byte-identical"
    /// iff these bytes are.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(format!("v{};", self.version).as_bytes());
        for t in &self.tasks {
            out.extend_from_slice(
                format!(
                    "{}:{}:{}:{};",
                    t.id,
                    state_tag(t.state),
                    t.progress,
                    escape_label(&t.label)
                )
                .as_bytes(),
            );
        }
        out
    }
}

/// Deterministic byte encoding of an effect list, in order.
pub fn canonical_effect_bytes(effects: &[Effect]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in effects {
        match e {
            Effect::PersistTask { task_id } => {
                out.extend_from_slice(format!("persist:{task_id};").as_bytes());
            }
            Effect::Notify { task_id, message } => {
                out.extend_from_slice(
                    format!("notify:{task_id}:{};", escape_label(message)).as_bytes(),
                );
            }
            Effect::VendorExtension { name } => {
                out.extend_from_slice(format!("vendor:{};", escape_label(name)).as_bytes());
            }
        }
    }
    out
}

/// The pure transition function: events in, state plus effects out.
///
/// Same (state, event) always yields the same (state, effects).
/// Effects are recorded, never executed. No clock, no I/O, no
/// allocation beyond the returned state and effect list.
pub fn reduce(
    state: &MachineState,
    event: &Event,
) -> Result<(MachineState, Vec<Effect>), ReduceError> {
    let mut next = state.clone();
    let mut effects: Vec<Effect> = Vec::new();
    match event {
        Event::Spawn { id, label } => reduce_spawn(&mut next, *id, label, &mut effects)?,
        Event::Start { id } => reduce_start(&mut next, *id, &mut effects)?,
        Event::Progress { id, percent } => reduce_progress(&mut next, *id, *percent, &mut effects)?,
        Event::Block { id, reason } => reduce_block(&mut next, *id, reason, &mut effects)?,
        Event::Unblock { id } => reduce_unblock(&mut next, *id, &mut effects)?,
        Event::Complete { id } => reduce_complete(&mut next, *id, &mut effects)?,
        Event::Fail { id, reason } => reduce_fail(&mut next, *id, reason, &mut effects)?,
    }
    debug_assert!(
        effects.len() <= MAX_EFFECTS_PER_EVENT,
        "reduce emitted more than MAX_EFFECTS_PER_EVENT effects"
    );
    next.version = next.version.saturating_add(1);
    Ok((next, effects))
}

/// Find task `id` and require it to be in `from`; move it to `to`.
/// Returns the task's index. The shared guard behind every
/// state-changing event.
fn transition(
    next: &mut MachineState,
    id: u64,
    from: TaskState,
    to: TaskState,
    event: &'static str,
) -> Result<usize, ReduceError> {
    let at = find_task(&next.tasks, id).ok_or(ReduceError::UnknownTask { id })?;
    if next.tasks[at].state != from {
        return Err(ReduceError::IllegalTransition {
            id,
            from: next.tasks[at].state,
            event,
        });
    }
    next.tasks[at].state = to;
    Ok(at)
}

/// Record the persistence effect every transition emits.
fn persist(next_id: u64, effects: &mut Vec<Effect>) {
    effects.push(Effect::PersistTask { task_id: next_id });
}

fn reduce_spawn(
    next: &mut MachineState,
    id: u64,
    label: &str,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    if next.tasks.len() >= MAX_TASKS {
        return Err(ReduceError::TooManyTasks);
    }
    if find_task(&next.tasks, id).is_some() {
        return Err(ReduceError::DuplicateTask { id });
    }
    next.tasks.push(TaskRecord {
        id,
        state: TaskState::Queued,
        label: label.to_string(),
        progress: 0,
    });
    persist(id, effects);
    Ok(())
}

fn reduce_start(
    next: &mut MachineState,
    id: u64,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    transition(next, id, TaskState::Queued, TaskState::Running, "start")?;
    persist(id, effects);
    Ok(())
}

fn reduce_progress(
    next: &mut MachineState,
    id: u64,
    percent: u8,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    if percent > 100 {
        return Err(ReduceError::BadProgress { percent });
    }
    let at = transition(next, id, TaskState::Running, TaskState::Running, "progress")?;
    next.tasks[at].progress = percent;
    persist(id, effects);
    Ok(())
}

fn reduce_block(
    next: &mut MachineState,
    id: u64,
    reason: &str,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    transition(next, id, TaskState::Running, TaskState::Blocked, "block")?;
    persist(id, effects);
    effects.push(Effect::Notify {
        task_id: id,
        message: format!("task {id} blocked: {reason}"),
    });
    Ok(())
}

fn reduce_unblock(
    next: &mut MachineState,
    id: u64,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    transition(next, id, TaskState::Blocked, TaskState::Running, "unblock")?;
    persist(id, effects);
    Ok(())
}

fn reduce_complete(
    next: &mut MachineState,
    id: u64,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    let at = transition(next, id, TaskState::Running, TaskState::Done, "complete")?;
    next.tasks[at].progress = 100;
    persist(id, effects);
    effects.push(Effect::Notify {
        task_id: id,
        message: format!("task {id} done"),
    });
    Ok(())
}

fn reduce_fail(
    next: &mut MachineState,
    id: u64,
    reason: &str,
    effects: &mut Vec<Effect>,
) -> Result<(), ReduceError> {
    transition(next, id, TaskState::Running, TaskState::Failed, "fail")?;
    persist(id, effects);
    effects.push(Effect::Notify {
        task_id: id,
        message: format!("task {id} failed: {reason}"),
    });
    Ok(())
}

/// Ingestion: validate a raw wire event into a reducer [`Event`].
///
/// Bounds are enforced BEFORE any copy into state: an oversized text
/// field is refused on length alone, so a hostile 10 MB string costs
/// no allocation spike. Reserved ids (0, `u64::MAX`) are refused as
/// [`EventError::OutOfRangeIndex`]; unknown kinds as
/// [`EventError::Unknown`]. Zero panics: no indexing by untrusted
/// values anywhere on this path.
pub fn ingest(raw: &RawEvent) -> Result<Event, EventError> {
    check_task_id(raw.task_id)?;
    match raw.kind.as_str() {
        "spawn" => ingest_spawn(raw),
        "start" => Ok(Event::Start { id: raw.task_id }),
        "progress" => ingest_progress(raw),
        "block" => ingest_block(raw),
        "unblock" => Ok(Event::Unblock { id: raw.task_id }),
        "complete" => Ok(Event::Complete { id: raw.task_id }),
        "fail" => ingest_fail(raw),
        _ => Err(EventError::Unknown {
            kind: raw.kind.clone(),
        }),
    }
}

/// Reserved ids (0, `u64::MAX`) are never real tasks.
fn check_task_id(task_id: u64) -> Result<u64, EventError> {
    if task_id == 0 || task_id == u64::MAX {
        return Err(EventError::OutOfRangeIndex { id: task_id });
    }
    Ok(task_id)
}

/// Refuse an oversized text field on length alone — before any copy.
fn check_text_len(text: &str, field: &'static str, max: usize) -> Result<(), EventError> {
    if text.len() > max {
        return Err(EventError::OversizedPayload {
            field,
            bytes: text.len(),
            max,
        });
    }
    Ok(())
}

fn ingest_spawn(raw: &RawEvent) -> Result<Event, EventError> {
    check_text_len(&raw.text, "label", MAX_LABEL_BYTES)?;
    Ok(Event::Spawn {
        id: raw.task_id,
        label: raw.text.clone(),
    })
}

fn ingest_progress(raw: &RawEvent) -> Result<Event, EventError> {
    if raw.percent > 100 {
        return Err(EventError::BadPercent {
            percent: raw.percent,
        });
    }
    Ok(Event::Progress {
        id: raw.task_id,
        percent: raw.percent as u8,
    })
}

fn ingest_block(raw: &RawEvent) -> Result<Event, EventError> {
    check_text_len(&raw.text, "reason", MAX_REASON_BYTES)?;
    Ok(Event::Block {
        id: raw.task_id,
        reason: raw.text.clone(),
    })
}

fn ingest_fail(raw: &RawEvent) -> Result<Event, EventError> {
    check_text_len(&raw.text, "reason", MAX_REASON_BYTES)?;
    Ok(Event::Fail {
        id: raw.task_id,
        reason: raw.text.clone(),
    })
}

/// One effect in flight on the transport path (daemon → TUI): the
/// effect itself plus the delivery metadata the idempotency and
/// ordering contracts run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// Sender-assigned sequence number; the interpreter expects them
    /// gapless from 0.
    pub seq: u64,
    /// Idempotency key. `Some` → the effect is idempotent and the
    /// second delivery of the key is a typed no-op. `None` → the
    /// effect is declared non-idempotent; the interpreter applies it
    /// once and refuses any retry loud ([`EffectError::RetryRefused`])
    /// rather than double-applying.
    pub key: Option<String>,
    pub effect: Effect,
}

/// One applied delivery, for the mock side-effect ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerEntry {
    pub seq: u64,
    pub key: Option<String>,
    pub kind: &'static str,
}

/// The effect interpreter (MOCK): stands in for the TUI host's effect
/// runner. `fs_log` / `notify_log` are the in-memory doubles for the
/// filesystem and the notification sink; `sequence` records the exact
/// cross-sink application order; `ledger` is the side-effect ledger
/// the exactly-once contract is audited against.
pub struct MockInterpreter {
    pub fs_log: Vec<String>,
    pub notify_log: Vec<String>,
    pub sequence: Vec<String>,
    pub ledger: Vec<LedgerEntry>,
    next_seq: u64,
    hold: Vec<Delivery>,
}

impl MockInterpreter {
    pub fn new() -> Self {
        MockInterpreter {
            fs_log: Vec::new(),
            notify_log: Vec::new(),
            sequence: Vec::new(),
            ledger: Vec::new(),
            next_seq: 0,
            hold: Vec::new(),
        }
    }
}

impl Default for MockInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl MockInterpreter {
    /// Execute one effect against the mocks. Unknown kinds are a
    /// typed error — never a silent drop, never inline execution.
    fn apply_one(&mut self, effect: &Effect) -> Result<(), EffectError> {
        match effect {
            Effect::PersistTask { task_id } => {
                let line = format!("persist task {task_id}");
                self.fs_log.push(line.clone());
                self.sequence.push(format!("fs:{line}"));
                Ok(())
            }
            Effect::Notify { task_id, message } => {
                let line = format!("notify task {task_id}: {message}");
                self.notify_log.push(line.clone());
                self.sequence.push(format!("notify:{line}"));
                Ok(())
            }
            Effect::VendorExtension { name } => Err(EffectError::Unhandled { name: name.clone() }),
        }
    }

    /// In-process handoff: run a reducer-produced effect list in
    /// recorded order.
    ///
    /// Atomicity contract (declared): the whole list is validated as
    /// known BEFORE any effect is applied. One unknown effect rejects
    /// the batch with [`EffectError::Unhandled`] and zero side effects
    /// are recorded — the batch is rejected atomically, not rolled
    /// back (the reducer's state half is already pure and separate, so
    /// there is nothing to roll back here).
    pub fn interpret_batch(&mut self, effects: &[Effect]) -> Result<(), EffectError> {
        for effect in effects {
            if let Effect::VendorExtension { name } = effect {
                return Err(EffectError::Unhandled { name: name.clone() });
            }
        }
        for effect in effects {
            self.apply_one(effect)?;
        }
        Ok(())
    }

    /// Transport delivery with ordering + idempotency.
    ///
    /// `attempt` is the sender's retry counter (0 = first delivery).
    /// Checks run in this order: (1) keyed duplicate → `Duplicate`;
    /// (2) keyless retry → `RetryRefused`; (3) sequence: the expected
    /// seq applies immediately, a future seq parks in the bounded
    /// hold buffer (`OutOfOrder` past the bound), a stale seq is
    /// `OutOfOrder`. Applying advances `next_seq` and drains whatever
    /// the hold buffer can now fill, in order.
    pub fn deliver(&mut self, delivery: Delivery, attempt: u32) -> Result<(), EffectError> {
        if let Some(key) = &delivery.key {
            if self
                .ledger
                .iter()
                .any(|e| e.key.as_deref() == Some(key.as_str()))
            {
                return Err(EffectError::Duplicate { key: key.clone() });
            }
        } else if attempt > 0 {
            return Err(EffectError::RetryRefused { seq: delivery.seq });
        }
        if delivery.seq == self.next_seq {
            self.apply_delivery(&delivery)?;
            self.drain_hold()?;
        } else if delivery.seq > self.next_seq {
            if self.hold.len() >= HOLD_BUFFER_MAX {
                return Err(EffectError::OutOfOrder {
                    seq: delivery.seq,
                    expected: self.next_seq,
                });
            }
            self.hold.push(delivery);
        } else {
            return Err(EffectError::OutOfOrder {
                seq: delivery.seq,
                expected: self.next_seq,
            });
        }
        Ok(())
    }

    /// Apply one in-sequence delivery: effect, ledger entry, advance.
    fn apply_delivery(&mut self, delivery: &Delivery) -> Result<(), EffectError> {
        if self.ledger.len() >= MAX_LEDGER_ENTRIES {
            return Err(EffectError::LedgerFull);
        }
        self.apply_one(&delivery.effect)?;
        self.ledger.push(LedgerEntry {
            seq: delivery.seq,
            key: delivery.key.clone(),
            kind: effect_tag(&delivery.effect),
        });
        self.next_seq = self.next_seq.saturating_add(1);
        Ok(())
    }

    /// Apply held deliveries that now fill the gap, in seq order.
    /// Bounded: at most HOLD_BUFFER_MAX iterations, no recursion.
    fn drain_hold(&mut self) -> Result<(), EffectError> {
        for _ in 0..HOLD_BUFFER_MAX {
            let at = self.hold.iter().position(|d| d.seq == self.next_seq);
            match at {
                Some(i) => {
                    let delivery = self.hold.remove(i);
                    self.apply_delivery(&delivery)?;
                }
                None => break,
            }
        }
        Ok(())
    }
}
