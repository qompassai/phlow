# System 1 gauntlet integration: tasks 254–303

Pending registry integration by its owner; all test calls now use the observed public API. Do not count these files as compiled
until their modules are included in the library and the System 1 dependency is wired.
No production source, Cargo manifest, or live registry was modified by the test author.

## Module declarations for `crates/phlow-gauntlet/src/tasks/mod.rs`

```rust
mod task_254;
mod task_255;
mod task_256;
mod task_257;
mod task_258;
mod task_259;
mod task_260;
mod task_261;
mod task_262;
mod task_263;
mod task_264;
mod task_265;
mod task_266;
mod task_267;
mod task_268;
mod task_269;
mod task_270;
mod task_271;
mod task_272;
mod task_273;
mod task_274;
mod task_275;
mod task_276;
mod task_277;
mod task_278;
mod task_279;
mod task_280;
mod task_281;
mod task_282;
mod task_283;
mod task_284;
mod task_285;
mod task_286;
mod task_287;
mod task_288;
mod task_289;
mod task_290;
mod task_291;
mod task_292;
mod task_293;
mod task_294;
mod task_295;
mod task_296;
mod task_297;
mod task_298;
mod task_299;
mod task_300;
mod task_301;
mod task_302;
mod task_303;
```

## Entries to append after task 253 in `TASKS`

```rust
    TaskEntry {
        id: task_254::ID,
        name: task_254::NAME,
        kind: task_254::KIND,
        run: task_254::run,
    },
    TaskEntry {
        id: task_255::ID,
        name: task_255::NAME,
        kind: task_255::KIND,
        run: task_255::run,
    },
    TaskEntry {
        id: task_256::ID,
        name: task_256::NAME,
        kind: task_256::KIND,
        run: task_256::run,
    },
    TaskEntry {
        id: task_257::ID,
        name: task_257::NAME,
        kind: task_257::KIND,
        run: task_257::run,
    },
    TaskEntry {
        id: task_258::ID,
        name: task_258::NAME,
        kind: task_258::KIND,
        run: task_258::run,
    },
    TaskEntry {
        id: task_259::ID,
        name: task_259::NAME,
        kind: task_259::KIND,
        run: task_259::run,
    },
    TaskEntry {
        id: task_260::ID,
        name: task_260::NAME,
        kind: task_260::KIND,
        run: task_260::run,
    },
    TaskEntry {
        id: task_261::ID,
        name: task_261::NAME,
        kind: task_261::KIND,
        run: task_261::run,
    },
    TaskEntry {
        id: task_262::ID,
        name: task_262::NAME,
        kind: task_262::KIND,
        run: task_262::run,
    },
    TaskEntry {
        id: task_263::ID,
        name: task_263::NAME,
        kind: task_263::KIND,
        run: task_263::run,
    },
    TaskEntry {
        id: task_264::ID,
        name: task_264::NAME,
        kind: task_264::KIND,
        run: task_264::run,
    },
    TaskEntry {
        id: task_265::ID,
        name: task_265::NAME,
        kind: task_265::KIND,
        run: task_265::run,
    },
    TaskEntry {
        id: task_266::ID,
        name: task_266::NAME,
        kind: task_266::KIND,
        run: task_266::run,
    },
    TaskEntry {
        id: task_267::ID,
        name: task_267::NAME,
        kind: task_267::KIND,
        run: task_267::run,
    },
    TaskEntry {
        id: task_268::ID,
        name: task_268::NAME,
        kind: task_268::KIND,
        run: task_268::run,
    },
    TaskEntry {
        id: task_269::ID,
        name: task_269::NAME,
        kind: task_269::KIND,
        run: task_269::run,
    },
    TaskEntry {
        id: task_270::ID,
        name: task_270::NAME,
        kind: task_270::KIND,
        run: task_270::run,
    },
    TaskEntry {
        id: task_271::ID,
        name: task_271::NAME,
        kind: task_271::KIND,
        run: task_271::run,
    },
    TaskEntry {
        id: task_272::ID,
        name: task_272::NAME,
        kind: task_272::KIND,
        run: task_272::run,
    },
    TaskEntry {
        id: task_273::ID,
        name: task_273::NAME,
        kind: task_273::KIND,
        run: task_273::run,
    },
    TaskEntry {
        id: task_274::ID,
        name: task_274::NAME,
        kind: task_274::KIND,
        run: task_274::run,
    },
    TaskEntry {
        id: task_275::ID,
        name: task_275::NAME,
        kind: task_275::KIND,
        run: task_275::run,
    },
    TaskEntry {
        id: task_276::ID,
        name: task_276::NAME,
        kind: task_276::KIND,
        run: task_276::run,
    },
    TaskEntry {
        id: task_277::ID,
        name: task_277::NAME,
        kind: task_277::KIND,
        run: task_277::run,
    },
    TaskEntry {
        id: task_278::ID,
        name: task_278::NAME,
        kind: task_278::KIND,
        run: task_278::run,
    },
    TaskEntry {
        id: task_279::ID,
        name: task_279::NAME,
        kind: task_279::KIND,
        run: task_279::run,
    },
    TaskEntry {
        id: task_280::ID,
        name: task_280::NAME,
        kind: task_280::KIND,
        run: task_280::run,
    },
    TaskEntry {
        id: task_281::ID,
        name: task_281::NAME,
        kind: task_281::KIND,
        run: task_281::run,
    },
    TaskEntry {
        id: task_282::ID,
        name: task_282::NAME,
        kind: task_282::KIND,
        run: task_282::run,
    },
    TaskEntry {
        id: task_283::ID,
        name: task_283::NAME,
        kind: task_283::KIND,
        run: task_283::run,
    },
    TaskEntry {
        id: task_284::ID,
        name: task_284::NAME,
        kind: task_284::KIND,
        run: task_284::run,
    },
    TaskEntry {
        id: task_285::ID,
        name: task_285::NAME,
        kind: task_285::KIND,
        run: task_285::run,
    },
    TaskEntry {
        id: task_286::ID,
        name: task_286::NAME,
        kind: task_286::KIND,
        run: task_286::run,
    },
    TaskEntry {
        id: task_287::ID,
        name: task_287::NAME,
        kind: task_287::KIND,
        run: task_287::run,
    },
    TaskEntry {
        id: task_288::ID,
        name: task_288::NAME,
        kind: task_288::KIND,
        run: task_288::run,
    },
    TaskEntry {
        id: task_289::ID,
        name: task_289::NAME,
        kind: task_289::KIND,
        run: task_289::run,
    },
    TaskEntry {
        id: task_290::ID,
        name: task_290::NAME,
        kind: task_290::KIND,
        run: task_290::run,
    },
    TaskEntry {
        id: task_291::ID,
        name: task_291::NAME,
        kind: task_291::KIND,
        run: task_291::run,
    },
    TaskEntry {
        id: task_292::ID,
        name: task_292::NAME,
        kind: task_292::KIND,
        run: task_292::run,
    },
    TaskEntry {
        id: task_293::ID,
        name: task_293::NAME,
        kind: task_293::KIND,
        run: task_293::run,
    },
    TaskEntry {
        id: task_294::ID,
        name: task_294::NAME,
        kind: task_294::KIND,
        run: task_294::run,
    },
    TaskEntry {
        id: task_295::ID,
        name: task_295::NAME,
        kind: task_295::KIND,
        run: task_295::run,
    },
    TaskEntry {
        id: task_296::ID,
        name: task_296::NAME,
        kind: task_296::KIND,
        run: task_296::run,
    },
    TaskEntry {
        id: task_297::ID,
        name: task_297::NAME,
        kind: task_297::KIND,
        run: task_297::run,
    },
    TaskEntry {
        id: task_298::ID,
        name: task_298::NAME,
        kind: task_298::KIND,
        run: task_298::run,
    },
    TaskEntry {
        id: task_299::ID,
        name: task_299::NAME,
        kind: task_299::KIND,
        run: task_299::run,
    },
    TaskEntry {
        id: task_300::ID,
        name: task_300::NAME,
        kind: task_300::KIND,
        run: task_300::run,
    },
    TaskEntry {
        id: task_301::ID,
        name: task_301::NAME,
        kind: task_301::KIND,
        run: task_301::run,
    },
    TaskEntry {
        id: task_302::ID,
        name: task_302::NAME,
        kind: task_302::KIND,
        run: task_302::run,
    },
    TaskEntry {
        id: task_303::ID,
        name: task_303::NAME,
        kind: task_303::KIND,
        run: task_303::run,
    },
```

The integration owner must also update `TASK_COUNT_MAX` to 303 and add the
`phlow-system1` dependency. These files are outside this task's ownership.

## Per-task classification

| Task | Class | Concern | Result |
| --- | --- | --- | --- |
| 254 | validation | simple negation in state | PASS |
| 255 | validation | negation in instructions | PASS |
| 256 | validation | double negation | PASS |
| 257 | validation | negation scoped to one action | PASS |
| 258 | validation | negated option labels | PASS |
| 259 | validation | noul negated phrasing | PASS |
| 260 | validation | score negated criterion | PASS |
| 261 | adversarial | negation blind high confidence escalates | PASS |
| 262 | validation | approved label is data | PASS |
| 263 | validation | instructions override option labels | PASS |
| 264 | validation | system prefix remains state data | PASS |
| 265 | adversarial | approved label echo escalates | FAIL (behavior) |
| 266 | adversarial | option label echo escalates | FAIL (behavior) |
| 267 | adversarial | system prefix override escalates | FAIL (behavior) |
| 268 | validation | non latin state flagged | FAIL (behavior) |
| 269 | adversarial | emoji only state flagged | FAIL (behavior) |
| 270 | adversarial | empty state rejected | FAIL (behavior) |
| 271 | adversarial | oversized state fails before backend | PASS |
| 272 | validation | mixed language state flagged | FAIL (behavior) |
| 273 | adversarial | ood non latin wrong answer surfaces | FAIL (behavior) |
| 274 | adversarial | ood emoji wrong answer surfaces | FAIL (behavior) |
| 275 | adversarial | ood mixed wrong answer surfaces | FAIL (behavior) |
| 276 | validation | two hundred options round trip | FAIL (behavior) |
| 277 | adversarial | out of range index is protocol error | PASS |
| 278 | validation | duplicate option strings preserve indices | PASS |
| 279 | adversarial | max index rejected without panic | PASS |
| 280 | adversarial | invalid choice poison rejects batch | PASS |
| 281 | validation | ordinal scores preserve monotonicity | PASS |
| 282 | validation | identical state score stability | PASS |
| 283 | validation | score threshold tie escalates | PASS |
| 284 | adversarial | inverted score cannot approve high risk | PASS |
| 285 | adversarial | low score cannot override irreversibility | PASS |
| 286 | validation | non english requires multilingual path | FAIL (behavior) |
| 287 | validation | language mismatch is detected | FAIL (behavior) |
| 288 | adversarial | english profile non english escalates | FAIL (behavior) |
| 289 | adversarial | english profile mixed script escalates | FAIL (behavior) |
| 290 | validation | risk boundary is strict | PASS |
| 291 | validation | confidence boundary is strict | FAIL (behavior) |
| 292 | validation | threshold four quadrants | PASS |
| 293 | adversarial | joint ties and nonfinite scores fail closed | PASS |
| 294 | validation | thresholds require workload validation | PASS |
| 295 | adversarial | unreachable endpoint escalates | PASS |
| 296 | adversarial | malformed json has no partial application | PASS |
| 297 | adversarial | timeout discards entire batch | PASS |
| 298 | adversarial | http 500 cause is surfaced | PASS |
| 299 | adversarial | alternating transport does not flap | PASS |
| 300 | validation | ten question batch preserves ids | PASS |
| 301 | adversarial | malformed answer isolated in valid envelope | FAIL (behavior) |
| 302 | adversarial | empty batch rejected before transport | PASS |
| 303 | validation | duplicate ids have deterministic map semantics | PASS |

## Observed API and divergence from the sketch

The crate was absent at the initial read but landed during authoring. All provisional
calls were replaced before the reported test run. Inspected production sources:

- `crates/phlow-system1/src/{lib,backend,protocol,risk,error,config}.rs`
- `crates/phlow-system1/Cargo.toml`
- `crates/phlow-approval/src/{lib,scope,policy,queue}.rs`
- `crates/phlow-gauntlet/{Cargo.toml,src/lib.rs,src/tasks/mod.rs}`
- Existing drivers `task_209.rs`, `task_233.rs`, `task_246.rs`, `task_250.rs`.

Actual contract used:

- `System1Decider::decide(&QuestionBatch)` returns a Send future. All question
  and answer variants match the sketch. `AnswerBatch.answers` is a BTreeMap.
- `MockBackend::new().with_answer(id, answer)` repeats its fixed answer map;
  `failing(System1Error)` repeats a typed failure. `calls()` and `last_batch()`
  expose observations. It deliberately returns unvalidated answers and counts
  even rejected calls. Consumers must call `AnswerBatch::validate` themselves.
- `AnswerBatch::{to_wire,from_wire,validate}` are public. Response JSON is keyed
  directly by question ID, without an `answers` envelope or answer type tags.
- `RiskScorer::new(decider).route(&phlow_approval::Decision, context)` returns
  `Route::{AutoApprove(Assessment),Escalate(Escalation)}`. It takes ownership of
  its decider and exposes it via `decider()`. The shared test fixture only forwards
  a borrowed MockBackend; it implements no decision or protocol policy.
- RiskScorer asks **five** questions in one batch: `risk`, `reversible`,
  `irreversible`, `consistent`, `forbidden`. Negation-blind contradictions are
  rejected; consistently wrong answers can still auto-approve eligible scopes.
- Actual independent classes are `phlow_approval::Risk::{Observe,
  LocalReversible,Process,Network,Irreversible}`. Only the first two are eligible.
  Test decisions use an explicit Approval verdict and a real parsed Scope.
- `RISK_MAX = 0.2` is exclusive; `CONFIDENCE_MIN = 0.9` is currently inclusive.
  Test 291 deliberately requires strict confidence, as requested, and fails.
  Constants are documented as UNVALIDATED PLACEHOLDERs; no held-out calibration
  is claimed by these tests or by the test author.
- Limits: state 16 KiB, instructions 1024 bytes, 16 questions, **16 options**,
  option/criterion 256 bytes, response 64 KiB, risk context 8 KiB. Test 276 keeps
  the requested 200-option acceptance assertion and fails, rather than silently
  shrinking it. Test 279 uses 16 valid options so its usize::MAX answer actually
  reaches the answer-validation boundary.
- No language-routing metadata or language-mismatch error exists. The mocked
  confidently wrong multilingual profiles are fixtures, not real checkpoints.
- The public parser rejects the whole response on any malformed answer. There
  is no partial-result/diagnostic map; test 301 therefore fails its assertion that
  a valid answer remains available for inspection.

## Coverage boundaries and test-only fixtures

Exactly 50 `#[test]` scenarios: **25 validation / 25 adversarial**. Tasks
254–260, 262–264, 276, 278, 281–282, 300 and 303 characterize deterministic
protocol behavior. They do not prove a model understands natural language,
learned ordinal monotonicity, or calibration. No model or network endpoint ran.

The shared test adapter calls the real public answer validator after the
untrusted mock. It contains no custom response validation or approval logic.
Tasks 271/302 call `QuestionBatch::validate` as the public preflight boundary
and verify no mock script was consumed; they do not claim an HTTP observation.
Task 294 additionally checks the production threshold documentation in `risk.rs`.
Task 295 passes a transport escalation into the real ApprovalQueue and verifies
Pending state, unchanged scope, and no recorded approver.

MockBackend has no raw-response, streaming, or sequential-script API. Task 296
calls the real wire parser and feeds its exact typed error through MockBackend
into RiskScorer. Task 297 injects Timeout into a two-question batch; it tests
whole-batch failure, not an actual network timeout after a partial body. Task 299
uses a three-call dispatcher over two real MockBackends (success/failure/success),
rejects script exhaustion, and verifies exactly one backend call per decision.
It does not reimplement decision logic or introduce a retry.

Whole-response syntax errors/timeouts reject the batch (296–297). Invalid Choice
indices also reject the batch (277/279/280). Task 301 requests partial inspection
of a syntactically valid response with a wrong-kind answer; it never authorizes
from partial data. No per-answer diagnostics API exists yet; the implementing
worker must add an observable diagnostic seam to fully complete that concern.
A BTreeMap cannot represent duplicate IDs: task 303 documents last insertion
wins and verifies the final question reaches the backend with one answer.

The only async executor is a bounded single poll of in-memory mock futures.
Pending is an explicit driver error, never a fabricated successful result.

## Gate evidence and reproducible commands

The live registry/manifest are owned by another worker. To compile these exact
50 source files without editing them, validation used a temporary copy of the
real gauntlet crate, with path dependencies pointing to the real production
crates. Only the temporary manifest, registry and task count were integrated.
The temporary crate is:

`/tmp/phlow-system1-gauntlet/integration/crates/phlow-gauntlet`

The 50 copied files were checked byte-for-byte against the repository files.
No surrogate policy or protocol implementation was built; the only trait wrappers
forward to the production MockBackend. The source files remain unregistered in the live tree until the
registry owner applies the snippet.

Commands run from the repository root:

```sh
cargo test -p phlow-gauntlet --lib
cargo fmt --all -- --check
cargo clippy -p phlow-gauntlet --all-targets
rustfmt --edition 2024 --check crates/phlow-gauntlet/src/tasks/task_{254..303}.rs
cargo test --manifest-path /tmp/phlow-system1-gauntlet/integration/crates/phlow-gauntlet/Cargo.toml -p phlow-gauntlet --lib
cargo clippy --manifest-path /tmp/phlow-system1-gauntlet/integration/crates/phlow-gauntlet/Cargo.toml -p phlow-gauntlet --all-targets
git diff --check
```

| Gate | Observed result |
| --- | --- |
| Live baseline library | Exit 101: 245 passed / 24 failed / 0 ignored; none of the new tasks included |
| Temporary integrated library, final | Exit 101: 275 passed / 44 failed / 0 ignored; all 50 new tests compiled |
| New tasks extracted from that run | **33 passed / 17 behavioral failures / 0 compile errors**; individual results above |
| Focused run with all 50 `tasks::task_N::` filters | Exit 101: 33 passed / 17 failed / 269 filtered out |
| `cargo fmt --all -- --check` | Exit 1: formatting differences in sibling-owned System 1 files; not modified |
| Direct rustfmt check of all 50 new files | Exit 0: all 50 clean |
| Live baseline Clippy | Exit 0; does not include the new unregistered modules |
| Temporary integrated Clippy, all targets | Exit 0: **zero warnings from tasks 254–303**; one existing task_233 items-after-test-module warning |
| `git diff --check` | Exit 0; direct rustfmt separately covers untracked new Rust files |

Temporary full-library failures comprise 17 new behavioral failures, the 24
baseline failures, two task_238 PATH-child fixture failures (the temporary copy
has no sibling built gauntlet binary), and one registry continuity failure
because tasks 251–253 had not yet been integrated in that snapshot. Those last
three are temporary-copy integration limitations, not System 1 test failures.

Final logs: `/tmp/phlow-system1-gauntlet/final-integrated-test.log`,
`final-integrated-clippy.log`, `final-fmt.log`; baseline test log:
`/tmp/phlow-system1-baseline-test.log`. Cargo also reported that its global
last-use database was read-only; compilation and Clippy still completed.
Toolchain: `nightly-2026-09-25-x86_64-unknown-linux-gnu` from the repository pin.

Earlier temporary-copy wiring errors and two authoring warnings were fixed
before these final runs; no production diagnostics or tests were suppressed.

## Resume contract

1. Registry owner applies the module/entry snippet after tasks 251–253, adds
   `phlow-system1` to gauntlet dependencies, and sets TASK_COUNT_MAX to 303.
2. Run the three requested gates on the live tree and verify all 50 new names
   appear; a successful run excluding these modules is not verification.
3. Preserve the 17 failing behavioral assertions while implementation proceeds.
   Do not make the test fixtures perform language routing or safety decisions.
4. Resolve the partial-inspection API for 301 and explicit language-profile
   routing for 286–289 in production; then strengthen typed diagnostic assertions.
5. Stop for stale APIs, ownership conflicts, uncompiled tests, live-model/network
   requirements, or any need to alter files outside the assigned ownership.

Only task_254.rs through task_303.rs and this snippet were added by this author.
No commit, push, production implementation, or live-registry edit was made.
