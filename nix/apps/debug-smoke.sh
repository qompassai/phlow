# phlow-debug-smoke: prove the debugger chain works end to end.
# Builds the phlow-json test binary, drives lldb-dap over the real DAP
# protocol (initialize -> launch stopped-at-entry -> setBreakpoints ->
# verified -> continue -> stopped at breakpoint), and confirms the
# breakpoint on phlow_json::parse_limited both VERIFIES and HITS.
#
# The DAP server comes from the ambient PATH on purpose: nixpkgs' lldb
# 21.1.8 ships a broken lldb-dap (every launch fails "invalid debugger",
# verified 2026-09-30 inside and outside the devShell), and the repo's
# devShell does not pin lldb either. `nix run` preserves the user's PATH,
# so this resolves to the environment's lldb-dap (LLVM 23.1.1 on primo).
# No DAP server on PATH -> honest FAIL (the chain is unverifiable).
#
#   nix run .#debug-smoke
#
# Report: reports/phlow-debug-smoke-<UTC timestamp>.md (gitignored).

need_bin cargo cargo
need_bin python3 python3
require_repo_root

if command -v lldb-dap >/dev/null 2>&1; then
  DAP_BIN="$(command -v lldb-dap)"
elif command -v lldb-vscode >/dev/null 2>&1; then
  DAP_BIN="$(command -v lldb-vscode)"
else
  DAP_BIN=""
fi

report_begin
report_section "debugger smoke"

if [ -z "$DAP_BIN" ]; then
  echo "FAIL debug-smoke: neither lldb-dap nor lldb-vscode on PATH" >&2
  report_line "result: **FAIL** — no DAP server on PATH; the debugger chain is unverifiable."
  report_section "summary"
  report_line "exit: 1"
  exit 1
fi

isolated_cargo_env
report_line "DAP server: \`${DAP_BIN}\`"
report_line "target: \`crates/phlow-json/src/lib.rs:121\` (phlow_json::parse_limited)"
report_line ""

report_section "build test binary"
if ! cargo test --locked -p phlow-json --no-run \
    >"${PHLOW_SCRATCH}/build.log" 2>&1; then
  echo "FAIL debug-smoke: could not build phlow-json tests"
  report_line "result: **FAIL** — test binary build failed."
  verify_cleanup >/dev/null 2>&1 || true
  exit 1
fi
TESTBIN="$(find "${CARGO_TARGET_DIR}/debug" -name 'phlow_json-*' ! -name '*.*' \
  -type f -executable 2>/dev/null | head -1 || true)"
if [ -z "$TESTBIN" ]; then
  echo "FAIL debug-smoke: no phlow_json test binary found"
  report_line "result: **FAIL** — no phlow_json test binary under ${CARGO_TARGET_DIR}/debug."
  verify_cleanup >/dev/null 2>&1 || true
  exit 1
fi
echo "test binary: $TESTBIN"
report_line "test binary built: \`$(basename "$TESTBIN")\`"
report_line ""

SRC="$PWD/crates/phlow-json/src/lib.rs"
LINE=121

report_section "DAP session"
echo "== DAP: launch stopped at entry, set breakpoint ${SRC}:${LINE}, continue, await stop =="
if python3 - "$DAP_BIN" "$TESTBIN" "$SRC" "$LINE" \
    >"${PHLOW_SCRATCH}/dap.log" 2>&1 <<'PYEOF'
import json, os, select, subprocess, sys, time

dap_bin, program, source, line = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])

proc = subprocess.Popen(
    [dap_bin], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL)

def send(obj):
    body = json.dumps(obj).encode()
    proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    proc.stdin.flush()

seq = 0
def req(command, arguments=None):
    global seq
    seq += 1
    send({"seq": seq, "type": "request", "command": command,
          "arguments": arguments or {}})

def read_messages(timeout):
    """Collect complete DAP messages available within timeout seconds."""
    msgs, buf = [], b""
    end = time.time() + timeout
    while time.time() < end:
        r, _, _ = select.select([proc.stdout], [], [],
                               max(0.0, end - time.time()))
        if not r:
            break
        chunk = os.read(proc.stdout.fileno(), 65536)
        if not chunk:
            break
        buf += chunk
        while True:
            head, sep, rest = buf.partition(b"\r\n\r\n")
            if not sep:
                break
            try:
                clen = int([l for l in head.split(b"\r\n")
                            if l.lower().startswith(b"content-length")
                            ][0].split(b":", 1)[1].strip())
            except (IndexError, ValueError):
                break
            if len(rest) < clen:
                break
            msgs.append(json.loads(rest[:clen].decode()))
            buf = rest[clen:]
    return msgs

def note_verified(bp):
    """Record verification from a breakpoint object; return its location."""
    if bp.get("verified"):
        # The response does not always echo the source path; fall back to
        # the path we requested the breakpoint on.
        path = bp.get("source", {}).get("path") or source
        return "%s:%s" % (path, bp.get("line", "?"))
    return None

req("initialize", {"adapterID": "phlow-debug-smoke"})
# Stop at entry: this lldb-dap only resolves source breakpoints once the
# target exists, so we set them while stopped at the entry point.
req("launch", {"program": program, "stopOnEntry": True,
               "args": ["tests::parses_all_scalar_shapes", "--exact",
                        "--nocapture", "--test-threads=1"]})
req("configurationDone")

thread_id, verified_at = None, None
hit, bp_answered = False, False
deadline = time.time() + 90
stage = "entry"  # entry -> bp -> run
while time.time() < deadline and not hit:
    for m in read_messages(2.0):
        mtype = m.get("type")
        body = m.get("body") or {}
        if mtype == "event" and m.get("event") == "stopped":
            reason = body.get("reason")
            if stage == "entry" and reason == "entry":
                thread_id = body.get("threadId", 1)
                req("setBreakpoints",
                    {"source": {"path": source},
                     "breakpoints": [{"line": line}]})
                stage = "bp"
            elif stage == "run" and reason == "breakpoint":
                hit = True
        elif mtype == "response" and m.get("command") == "setBreakpoints":
            bp_answered = True
            for b in body.get("breakpoints", []):
                loc = note_verified(b)
                if loc:
                    verified_at = loc
            if verified_at and stage == "bp":
                req("continue", {"threadId": thread_id or 1})
                stage = "run"
        elif mtype == "event" and m.get("event") == "breakpoint":
            loc = note_verified(body.get("breakpoint", {}))
            if loc:
                verified_at = loc
            if verified_at and stage == "bp":
                req("continue", {"threadId": thread_id or 1})
                stage = "run"
        elif mtype == "event" and m.get("event") in ("exited", "terminated"):
            deadline = 0.0  # process gone; stop waiting
            break
    if stage == "bp" and bp_answered and not verified_at:
        break  # breakpoint answered but never verified: honest FAIL

try:
    req("disconnect", {"terminateDebuggee": True})
except (BrokenPipeError, ValueError):
    pass
try:
    proc.wait(timeout=10)
except subprocess.TimeoutExpired:
    proc.kill()

print("BREAKPOINT_VERIFIED_AT=%s" % (verified_at or "never"))
print("BREAKPOINT_HIT=%s" % hit)
sys.exit(0 if (verified_at and hit) else 1)
PYEOF
then
  echo "PASS debug-smoke: breakpoint verified and hit"
  report_line "result: **PASS** — breakpoint on phlow_json::parse_limited verified AND hit under lldb-dap."
  report_line "The breakpoint is set while stopped at the program entry point"
  report_line "(this lldb-dap only resolves source breakpoints once the target"
  report_line "exists); the session then continues and must observe a"
  report_line "\`stopped\` event with reason \`breakpoint\`."
  rc=0
else
  echo "FAIL debug-smoke: breakpoint did not verify or was not hit"
  report_line "result: **FAIL** — breakpoint did not verify, or the stop event never arrived."
  rc=1
fi
report_line ""
report_line "driver output:"
report_line '```'
cat "${PHLOW_SCRATCH}/dap.log" >>"$REPORT_FILE"
report_line '```'

verify_cleanup || rc=1
verify_tree_clean || rc=1

report_section "summary"
report_line "exit: ${rc}"
exit "$rc"
