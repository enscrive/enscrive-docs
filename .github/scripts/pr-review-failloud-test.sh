#!/usr/bin/env bash
#
# Driver-level forced-failure test for the fail-loud reviewer contract
# (Rebalance ADR D4,
# enscrive-governance/plans/PR-GOVERNANCE-REBALANCE-2026-08-19/ADR.md).
#
# Runs the REAL pr-review.sh end to end with PATH-shimmed stub `claude` and
# `gh` executables, and asserts the two D4 terminal behaviors the pure-helper
# unit tests cannot reach:
#
#   forced failure — every model call returns the captured tool-call garbage
#     (enscrive-code#71 run 31899300151, 2026-08-19, verbatim): the driver
#     must exhaust exactly MAX_ATTEMPTS claude calls, post NO review, post a
#     PR comment naming the engine failure, add the reviewer-retried label,
#     and exit 1 (RED check).
#
#   retry recovery — garbage on attempt 1, a valid decision on attempt 2: the
#     driver must exit 0 and post exactly one approval review.
#
# Per A-036 a fail-loud path ships only after being OBSERVED failing; this
# test is that observation, and the pr-review.yml self-test step re-observes
# it on every PR.
#
# Run: bash .github/scripts/pr-review-failloud-test.sh
# Exit 0 = all pass; non-zero = one or more failures.

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

FAILS=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

check() { # desc expected actual
  if [ "$2" = "$3" ]; then
    printf 'ok   - %s\n' "$1"
  else
    printf 'FAIL - %s: expected [%s] got [%s]\n' "$1" "$2" "$3"
    FAILS=$((FAILS + 1))
  fi
}

check_ge() { # desc min actual
  if [ "$3" -ge "$2" ] 2>/dev/null; then
    printf 'ok   - %s (got %s)\n' "$1" "$3"
  else
    printf 'FAIL - %s: expected >= %s got [%s]\n' "$1" "$2" "$3"
    FAILS=$((FAILS + 1))
  fi
}

# The real captured engine-failure output (narration + mangled tool-call
# markup), verbatim.
cat > "$TMP/garbage.txt" <<'GARBAGE_EOF'
`Let me verify the change compiles conceptually — checking for other `Summary` literal constructions and the FLEET contents.`

I'll inspect the repo.

<invoke_name>Bash</invoke_name>
<parameter name="command">ls; grep -rn "item_type" --include=*.sql . | head -50</parameter>
</invoke>
GARBAGE_EOF

printf '%s' '{"decision":"approve","confidence":0.95,"summary":"Stub decision: change matches its stated intent.","blocking_issues":[],"high_risk_notes":""}' \
  > "$TMP/valid.txt"

# --- stubs -------------------------------------------------------------------
# Both stubs append every invocation (argv) to $STUB_CALL_LOG. `claude` reads
# and discards stdin like the real CLI, then emits a VALID CLI envelope whose
# result text is controlled per-invocation by $STUB_STATE/result.N (N = the
# invocation count), falling back to result.last. `gh` answers the driver's
# read paths with minimal valid data and silently records every write path
# (review/comment/edit/label/merge).
mkdir -p "$TMP/bin"

cat > "$TMP/bin/claude" <<'STUB_EOF'
#!/usr/bin/env bash
cat > /dev/null
printf 'claude %s\n' "$*" >> "$STUB_CALL_LOG"
n=$(cat "$STUB_STATE/claude_calls" 2>/dev/null || echo 0)
n=$((n + 1))
printf '%s' "$n" > "$STUB_STATE/claude_calls"
src="$STUB_STATE/result.$n"
[ -f "$src" ] || src="$STUB_STATE/result.last"
jq -n --rawfile r "$src" '{"type":"result","subtype":"success","is_error":false,"result":$r}'
STUB_EOF

cat > "$TMP/bin/gh" <<'STUB_EOF'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$STUB_CALL_LOG"
case "${1:-} ${2:-}" in
  "pr view")
    json=""
    prev=""
    for a in "$@"; do
      [ "$prev" = "--json" ] && json="$a"
      prev="$a"
    done
    case "$json" in
      author)  echo "stub-author" ;;
      title)   echo "Stub PR title" ;;
      body)    echo "Stub PR body" ;;
      files)   echo '["src/lib.rs","README.md"]' ;;
      reviews) echo "0" ;;
    esac
    ;;
  "pr diff")
    printf 'diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n+pub fn stub() {}\n'
    ;;
  *) : ;;
esac
exit 0
STUB_EOF

chmod +x "$TMP/bin/claude" "$TMP/bin/gh"

run_driver() { # scenario_dir; sets DRIVER_EXIT
  export STUB_CALL_LOG="$1/calls.log"
  export STUB_STATE="$1"
  : > "$STUB_CALL_LOG"
  printf '%s' 0 > "$STUB_STATE/claude_calls"
  (
    export PATH="$TMP/bin:$PATH"
    export PR=123 GITHUB_REPOSITORY=stub-org/stub-repo
    bash "$HERE/pr-review.sh"
  ) > "$1/driver.log" 2>&1
  DRIVER_EXIT=$?
}

count_calls() { # scenario_dir prefix
  grep -c "^$2" "$1/calls.log" || true
}

# --- scenario 1: forced failure (every attempt returns the garbage) ----------
S1="$TMP/forced-failure"
mkdir -p "$S1"
cp "$TMP/garbage.txt" "$S1/result.last"
run_driver "$S1"

echo "--- forced-failure: driver log ---"
sed -e 's/^/    /' "$S1/driver.log"
echo "--- forced-failure: recorded gh/claude calls (one line per call, truncated) ---"
grep -E '^(gh|claude) ' "$S1/calls.log" | cut -c1-160 | sed -e 's/^/    /'
echo "---"

check "forced failure: driver exit 1 (RED check)" 1 "$DRIVER_EXIT"
check "forced failure: exactly 3 claude invocations" 3 "$(count_calls "$S1" 'claude ')"
check "forced failure: zero 'gh pr review' calls (no review posted, none fabricated)" \
  0 "$(count_calls "$S1" 'gh pr review')"
check_ge "forced failure: at least one 'gh pr comment' naming the engine failure" \
  1 "$(count_calls "$S1" 'gh pr comment')"
check "forced failure: reviewer-retried label added" 1 \
  "$(grep -q -- '--add-label reviewer-retried' "$S1/calls.log" && echo 1 || echo 0)"
check "forced failure: retry attempt 1 logged with reason" 1 \
  "$(grep -q '^reviewer-retry attempt=1 reason=unparseable$' "$S1/driver.log" && echo 1 || echo 0)"
check "forced failure: retry attempt 2 logged with reason" 1 \
  "$(grep -q '^reviewer-retry attempt=2 reason=unparseable$' "$S1/driver.log" && echo 1 || echo 0)"

# --- scenario 2: retry recovery (garbage, then a valid decision) -------------
S2="$TMP/recovery"
mkdir -p "$S2"
cp "$TMP/garbage.txt" "$S2/result.1"
cp "$TMP/valid.txt" "$S2/result.last"
run_driver "$S2"

echo "--- recovery: driver log (tail) ---"
tail -n 8 "$S2/driver.log" | sed -e 's/^/    /'
echo "---"

check "recovery: driver exit 0" 0 "$DRIVER_EXIT"
check "recovery: exactly 2 claude invocations" 2 "$(count_calls "$S2" 'claude ')"
check "recovery: exactly one approval review posted" 1 \
  "$(count_calls "$S2" 'gh pr review 123 --approve')"
check "recovery: no changes-requested review posted" 0 \
  "$(grep -c -- '--request-changes' "$S2/calls.log" || true)"
check "recovery: reviewer-retried label added for the retry" 1 \
  "$(grep -q -- '--add-label reviewer-retried' "$S2/calls.log" && echo 1 || echo 0)"

echo
if [ "$FAILS" -eq 0 ]; then
  echo "ALL PASS"
  exit 0
else
  echo "$FAILS FAILURE(S)"
  exit 1
fi
