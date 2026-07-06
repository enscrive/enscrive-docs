#!/usr/bin/env bash
#
# Unit tests for the ENS-569 reviewer verdict logic (empty-block false-positive
# guard). Sources pr-review.sh in SELFTEST mode so only the pure helpers
# (count_blocking_issues, effective_decision) are defined — no gh/network calls
# run. Asserts the mapping from (model verdict, blocking issues, truncation) to
# the verdict the pipeline acts on.
#
# Run: bash .github/scripts/pr-review-verdict-test.sh
# Exit 0 = all pass; non-zero = one or more failures.

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

# Load helpers only (guard returns before any PR/gh work).
PR_REVIEW_SELFTEST=1 source "$HERE/pr-review.sh"
set +e  # sourced script enabled -e; disable so failed asserts don't abort early

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

fixture() { # name json  -> echoes path
  local p="$TMP/$1.json"
  printf '%s' "$2" > "$p"
  echo "$p"
}

# --- count_blocking_issues --------------------------------------------------
F_EMPTY=$(fixture empty '{"decision":"request_changes","blocking_issues":[]}')
F_WS=$(fixture ws '{"decision":"request_changes","blocking_issues":["   ","\t"]}')
F_ONE=$(fixture one '{"decision":"request_changes","blocking_issues":["SQL injection in query builder"]}')
F_TWO=$(fixture two '{"decision":"request_changes","blocking_issues":["a","b"]}')
F_MIXED=$(fixture mixed '{"decision":"request_changes","blocking_issues":["","real one"]}')
F_NONSTR=$(fixture nonstr '{"decision":"request_changes","blocking_issues":[{"note":"x"}]}')
F_MISSING=$(fixture missing '{"decision":"approve"}')

check "count: []                 -> 0" 0 "$(count_blocking_issues "$F_EMPTY")"
check "count: whitespace-only    -> 0" 0 "$(count_blocking_issues "$F_WS")"
check "count: one real           -> 1" 1 "$(count_blocking_issues "$F_ONE")"
check "count: two real           -> 2" 2 "$(count_blocking_issues "$F_TWO")"
check "count: empty+real         -> 1" 1 "$(count_blocking_issues "$F_MIXED")"
check "count: non-string entry   -> 1" 1 "$(count_blocking_issues "$F_NONSTR")"
check "count: missing key        -> 0" 0 "$(count_blocking_issues "$F_MISSING")"

# --- effective_decision -----------------------------------------------------
# (a) request_changes with ZERO blockers on a full diff -> coerced approve
check "(a) rc + 0 blk + full  -> approve:coerced" approve:coerced "$(effective_decision request_changes 0 0)"
# (b) request_changes WITH blockers still blocks (real reviews unaffected)
check "(b) rc + 1 blk + full  -> request_changes"  request_changes:model "$(effective_decision request_changes 1 0)"
check "(b) rc + 3 blk + full  -> request_changes"  request_changes:model "$(effective_decision request_changes 3 0)"
# (c) genuine approve is passed through untouched
check "(c) approve            -> approve:model"    approve:model         "$(effective_decision approve 0 0)"
check "(c) approve + blockers -> approve:model"    approve:model         "$(effective_decision approve 2 0)"
# (d) truncated diff still blocks even with 0 blockers (we did not see it all)
check "(d) rc + 0 blk + trunc -> request_changes"  request_changes:model "$(effective_decision request_changes 0 1)"

# --- end-to-end: guard over count, mirroring the script's call site ---------
# Empty-block request_changes on a full diff coerces to approve.
BC=$(count_blocking_issues "$F_EMPTY")
check "e2e: empty-block rc, full diff -> approve:coerced" approve:coerced "$(effective_decision request_changes "$BC" 0)"
# Whitespace-only blockers are treated as empty -> coerced approve.
BC=$(count_blocking_issues "$F_WS")
check "e2e: whitespace blockers      -> approve:coerced" approve:coerced "$(effective_decision request_changes "$BC" 0)"
# The unparseable fail-safe fixture carries a real blocker -> still blocks.
F_FAILSAFE=$(fixture failsafe '{"decision":"request_changes","confidence":0,"blocking_issues":["Unparseable reviewer output"]}')
BC=$(count_blocking_issues "$F_FAILSAFE")
check "e2e: fail-safe unparseable     -> request_changes" request_changes:model "$(effective_decision request_changes "$BC" 0)"

# --- regression: enscrive-metering-sentinel PR #24 incident (2026-07-02) -----
# The reviewer posted CHANGES_REQUESTED with body ending "Blocking issues:" and
# nothing after it (confidence 0.78, CI green, positive prose) — the parsed
# decision below is the incident's actual shape. It must coerce to approve.
F_PR24=$(fixture pr24 '{"decision":"request_changes","confidence":0.78,"summary":"The change replaces the shared status.json.tmp temp file name with a per-write unique temp path (pid+atomic seq), correctly eliminating the described concurrent-writer rename race, and adds cleanup on error paths plus a matching unit test; the diff is small, scoped to src/status.rs, and matches its stated intent.","blocking_issues":[],"high_risk_notes":""}')
BC=$(count_blocking_issues "$F_PR24")
check "regression: PR #24 incident    -> approve:coerced" approve:coerced "$(effective_decision request_changes "$BC" 0)"

echo
if [ "$FAILS" -eq 0 ]; then
  echo "ALL PASS"
  exit 0
else
  echo "$FAILS FAILURE(S)"
  exit 1
fi
