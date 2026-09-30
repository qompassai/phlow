# phlow-release: cut a GitHub release end to end — but only when Matt
# says so, explicitly, with a version. There is no default release and
# no accidental tag: the app refuses to run without <version>.
#
#   nix run .#release -- 0.3.0 [--dry-run]
#   nix run .#release -- v0.3.0 --dry-run
#
# Steps:
#   1. Validate <version> (^v?\d+\.\d+\.\d+$); refuse if the tag exists
#      locally or as a GitHub release.
#   2. Refuse unless crates/phlow-cli/Cargo.toml is already at that
#      version (release the version you bumped; see version-bump.sh).
#   3. gh must be authenticated — ambient auth only; this app never
#      accepts a token via args or env.
#   4. Build the Linux tarball deterministically (package-linux.sh in
#      the isolated cargo env, pinned toolchain).
#   5. Release notes, self-populating, never hand-written:
#      CHANGELOG.md section for the version if present, else git-cliff
#      (conventional commits) between the previous tag and HEAD, else a
#      plain git log summary.
#   6. gh release create with the tarball + sha256 as assets
#      (skipped with --dry-run).
#   7. Report + cleanup (dist artifacts removed after upload).
#
# GitHub Releases only. crates.io publish (tbr-*), signing keys, and
# Play/F-Droid submissions are deliberately NOT part of this app.
#
# Report: reports/phlow-release-<UTC timestamp>.md (gitignored).

# --- argument parsing (before the common lib: we may exit early) ---
VERSION=""
DRY_RUN=0
for a in "$@"; do
  case "$a" in
    --dry-run) DRY_RUN=1 ;;
    -*) echo "unknown flag: $a" >&2; exit 2 ;;
    *)
      if [ -z "$VERSION" ]; then VERSION="$a";
      else echo "unexpected argument: $a" >&2; exit 2; fi ;;
  esac
done
if [ -z "$VERSION" ]; then
  echo "usage: nix run .#release -- <version> [--dry-run]  (e.g. 0.3.0)" >&2
  echo "No version, no release: refusing to guess." >&2
  exit 2
fi

need_bin cargo cargo
need_bin git git
need_bin gh gh
# git-cliff is intentionally NOT a hard dep: the notes chain is
# CHANGELOG.md -> git-cliff -> git log, and the last two legs must work
# with or without the binary present.
require_repo_root
isolated_cargo_env
report_begin

fail=0

# --- 1. version validation -------------------------------------------------
report_section "version validation"
VNUM="${VERSION#v}"
if ! printf '%s' "$VNUM" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "ABORT: version '$VERSION' is not MAJOR.MINOR.PATCH (optional v prefix)." >&2
  report_line "result: **FAIL** — version \`$VERSION\` rejected (want \`v?\d+.\d+.\d+\`)."
  exit 2
fi
TAG="v${VNUM}"
echo "version: $VNUM, tag: $TAG"
report_line "version: \`${VNUM}\`, tag: \`${TAG}\`, dry-run: ${DRY_RUN}"

if git rev-parse --verify --quiet "refs/tags/${TAG}" >/dev/null; then
  echo "ABORT: tag ${TAG} already exists locally." >&2
  report_line "result: **FAIL** — tag \`${TAG}\` already exists; refusing to rebuild a release."
  exit 2
fi
if gh release view "$TAG" >/dev/null 2>&1; then
  echo "ABORT: GitHub release ${TAG} already exists." >&2
  report_line "result: **FAIL** — GitHub release \`${TAG}\` already exists."
  exit 2
fi
report_line "tag \`${TAG}\` is new: **PASS**"

# --- 2. crate version must match -------------------------------------------
CRATE_VER="$(grep -m1 '^version' crates/phlow-cli/Cargo.toml | cut -d'"' -f2)"
if [ "$CRATE_VER" != "$VNUM" ]; then
  echo "ABORT: crates/phlow-cli is at ${CRATE_VER}, not ${VNUM}." >&2
  echo "Run scripts/publish/version-bump.sh ${VNUM} first, then re-run." >&2
  report_line "result: **FAIL** — crate version \`${CRATE_VER}\` != requested \`${VNUM}\`."
  exit 2
fi
report_line "crate version matches: **PASS**"

# --- 3. gh auth (ambient only) ----------------------------------------------
if ! gh auth status >"${PHLOW_SCRATCH}/gh-auth.log" 2>&1; then
  echo "ABORT: gh is not authenticated. Authenticate gh yourself; this app" >&2
  echo "never accepts a token via args or env." >&2
  report_line "result: **FAIL** — \`gh auth status\` failed; refusing to proceed without ambient auth."
  exit 2
fi
report_line "gh authenticated (ambient): **PASS**"

# --- 4. build the Linux tarball ---------------------------------------------
report_section "build release artifacts"
echo "== scripts/publish/package-linux.sh =="
# package-linux.sh hardcodes ./target/release/phlow, but this app builds
# with an isolated CARGO_TARGET_DIR. Bridge it with a symlink for the
# duration of the script; target/ is gitignored so the tree check is
# unaffected. Refuse to clobber a real ./target the user already has.
if [ -e ./target ] && [ ! -L ./target ]; then
  echo "ABORT: ./target exists and is not a symlink; the release app needs" >&2
  echo "to link it to its isolated build dir. Move it aside and re-run." >&2
  report_line "result: **FAIL** — ./target exists (not a symlink); refusing to clobber it."
  exit 2
fi
LOCK_HASH_BEFORE="$(sha256sum Cargo.lock | cut -d' ' -f1)"
ln -sfn "${CARGO_TARGET_DIR}" ./target
if ! ./scripts/publish/package-linux.sh \
    >"${PHLOW_SCRATCH}/package.log" 2>&1; then
  rm -f ./target
  echo "FAIL package-linux.sh"
  report_line "result: **FAIL** — package-linux.sh failed."
  report_line '```'
  # The log is dominated by giant rustc command lines; show the actual
  # error lines instead of the raw tail.
  grep -aE "^error|FAILED|cannot stat|No such file|will not overwrite|cp:" \
    "${PHLOW_SCRATCH}/package.log" | head -20 >>"$REPORT_FILE" || true
  report_line '```'
  verify_cleanup >/dev/null 2>&1 || true
  exit 1
fi
rm -f ./target
# package-linux.sh does not pass --locked; prove it didn't move the lockfile.
LOCK_HASH_AFTER="$(sha256sum Cargo.lock | cut -d' ' -f1)"
if [ "$LOCK_HASH_BEFORE" != "$LOCK_HASH_AFTER" ]; then
  echo "FAIL: package-linux.sh modified Cargo.lock (non-deterministic build)" >&2
  report_line "result: **FAIL** — Cargo.lock changed during packaging; build not pinned."
  git checkout -- Cargo.lock
  exit 1
fi
report_line "Cargo.lock unchanged by packaging: **PASS**"
TARBALL="dist/phlow-${VNUM}-x86_64-unknown-linux-gnu.tar.gz"
if [ ! -f "$TARBALL" ] || [ ! -f "${TARBALL}.sha256" ]; then
  echo "FAIL: expected artifacts missing after package-linux.sh" >&2
  report_line "result: **FAIL** — tarball or sha256 missing."
  verify_cleanup >/dev/null 2>&1 || true
  exit 1
fi
echo "PASS artifacts: $TARBALL"
report_line "result: **PASS**"
report_line "- \`$(basename "$TARBALL")\` ($(du -h "$TARBALL" | cut -f1))"
report_line "- sha256: \`$(cut -d' ' -f1 "${TARBALL}.sha256")\`"

# --- 5. release notes ---------------------------------------------------------
report_section "release notes"
NOTES="${PHLOW_SCRATCH}/notes.md"
NOTES_SRC=""
PREV_TAG="$(git describe --tags --abbrev=0 2>/dev/null || echo "")"
if [ -n "$PREV_TAG" ]; then RANGE="${PREV_TAG}..HEAD"; else RANGE="HEAD"; fi

if [ -f CHANGELOG.md ] && grep -qE "^## \[?${VNUM}\]?" CHANGELOG.md; then
  awk -v v="$VNUM" '/^## /{if(found) exit; found=($0 ~ v)} found' \
    CHANGELOG.md > "$NOTES"
  NOTES_SRC="CHANGELOG.md section for ${VNUM}"
elif command -v git-cliff >/dev/null 2>&1; then
  # Conventional-commits notes between the previous tag and HEAD.
  # If git-cliff can't parse the history (nonzero exit or empty output)
  # we fall through to git log.
  cat > "${PHLOW_SCRATCH}/cliff.toml" <<'CLIFF_EOF'
[changelog]
header = ""
body = """
{% for group, commits in commits | group_by(attribute="group") -%}
### {{ group }}
{% for commit in commits -%}
- {{ commit.message | split(pat="\n") | first }} ({{ commit.id | truncate(length=7) }})
{% endfor -%}
{% endfor -%}
"""
trim = true
[git]
conventional_commits = true
filter_unconventional = true
split_commits = false
commit_parsers = [
  { message = "^feat", group = "Features" },
  { message = "^fix", group = "Bug Fixes" },
  { message = "^docs", group = "Documentation" },
  { message = "^perf", group = "Performance" },
  { message = "^refactor", group = "Refactoring" },
  { message = "^test", group = "Testing" },
  { message = "^chore", group = "Chores" },
]
CLIFF_EOF
  if git cliff --config "${PHLOW_SCRATCH}/cliff.toml" "$RANGE" \
      > "$NOTES" 2>"${PHLOW_SCRATCH}/cliff.log" && [ -s "$NOTES" ]; then
    NOTES_SRC="git-cliff (${RANGE})"
  fi
fi

if [ -z "$NOTES_SRC" ]; then
  # Last resort: plain git log summary. Never hand-written, never empty.
  {
    echo "Changes in ${RANGE}:"
    echo
    git log "$RANGE" --pretty=format:'- %s (%h, %an)'
  } > "$NOTES"
  NOTES_SRC="git log fallback (${RANGE})"
fi
{
  echo "# phlow ${VNUM}"
  echo
  echo "_Release notes generated $(date -u +%Y-%m-%dT%H:%M:%SZ) from ${NOTES_SRC}._"
  echo
  cat "$NOTES"
} > "${NOTES}.tmp" && mv "${NOTES}.tmp" "$NOTES"
echo "notes from: $NOTES_SRC"
report_line "source: ${NOTES_SRC}"
report_line '```'
head -20 "$NOTES" >>"$REPORT_FILE"
report_line '```'

# --- 6. create the release ------------------------------------------------------
report_section "gh release create"
if [ "$DRY_RUN" -eq 1 ]; then
  echo "DRY RUN — would execute:"
  # shellcheck disable=SC2086
  echo "  gh release create ${TAG} --title \"phlow ${VNUM}\" --notes-file ${NOTES} ${TARBALL} ${TARBALL}.sha256"
  report_line "result: **DRY-RUN** — no release created. Command that would run:"
  report_line "\`gh release create ${TAG} --title \"phlow ${VNUM}\" --notes-file <notes> <tarball> <tarball>.sha256\`"
else
  echo "== gh release create ${TAG} =="
  if gh release create "$TAG" --title "phlow ${VNUM}" \
      --notes-file "$NOTES" "$TARBALL" "${TARBALL}.sha256" \
      >"${PHLOW_SCRATCH}/gh-release.log" 2>&1; then
    echo "PASS gh release create ${TAG}"
    report_line "result: **PASS** — release \`${TAG}\` created with tarball assets."
  else
    echo "FAIL gh release create ${TAG}"
    report_line "result: **FAIL** — gh release create failed."
    report_line '```'
    tail -20 "${PHLOW_SCRATCH}/gh-release.log" >>"$REPORT_FILE"
    report_line '```'
    fail=1
  fi
fi

# --- 7. cleanup: remove the artifacts we built ----------------------------------
rm -f "$TARBALL" "${TARBALL}.sha256"
# Remove the staging dir too (rmdir would fail: it still holds the binary
# and packaging files). Only the versioned dir this run created.
rm -rf "dist/phlow-${VNUM}-x86_64-unknown-linux-gnu"
# Remove dist/ itself if we left it empty.
rmdir dist 2>/dev/null || true
verify_cleanup || fail=1
verify_tree_clean || fail=1

report_section "summary"
report_line "dry-run: ${DRY_RUN}, failures: ${fail}"
echo "== release ${TAG}: dry-run=${DRY_RUN}, failures=${fail} =="
[ "$fail" -eq 0 ]
