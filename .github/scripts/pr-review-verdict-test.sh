#!/usr/bin/env bash
#
# Unit tests for the ENS-569 / ENS-854 reviewer verdict logic. Sources
# pr-review.sh in SELFTEST mode so only the pure helpers
# (count_blocking_issues, effective_decision) are defined — no gh/network
# calls run. Asserts the mapping from (model verdict, blocking issues,
# truncation, confidence-gate result) to the verdict the pipeline acts on.
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

# pass_conf <confidence> <threshold>  -- mirrors the script's own awk gate,
# so fixture-derived tests exercise the real comparison, not a hardcoded 0/1.
pass_conf() {
  awk -v c="$1" -v t="$2" 'BEGIN{print (c+0>=t+0)?"1":"0"}'
}
THRESHOLD_DEFAULT=0.80

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

# --- effective_decision: synthetic cases ------------------------------------
# (a) request_changes with ZERO blockers, high confidence, full diff -> coerced
check "(a) rc + 0 blk + pass_conf=1 + full   -> approve:coerced" \
  approve:coerced "$(effective_decision request_changes 0 0 1)"
# (a2) ENS-569's original case didn't carry a passing confidence either -- same result
check "(a2) rc + 0 blk + pass_conf=0 + full  -> approve:coerced" \
  approve:coerced "$(effective_decision request_changes 0 0 0)"
# (b) request_changes WITH blockers still blocks, regardless of confidence
check "(b) rc + 1 blk + full                 -> request_changes:real" \
  request_changes:real "$(effective_decision request_changes 1 0 1)"
check "(b) rc + 3 blk + full                 -> request_changes:real" \
  request_changes:real "$(effective_decision request_changes 3 0 0)"
# (c) genuine high-confidence approve, 0 blockers -> passes straight through
check "(c) approve + 0 blk + pass_conf=1     -> approve:model" \
  approve:model "$(effective_decision approve 0 0 1)"
# (c2) ENS-854: approve + 0 blockers BELOW threshold -- the actual root cause of
# today's 6 flaps. Confidence alone must never turn "no named defect" into a hold.
check "(c2) approve + 0 blk + pass_conf=0    -> approve:coerced [ENS-854]" \
  approve:coerced "$(effective_decision approve 0 0 0)"
# (c3) FIX (behavior change from the old test suite): a self-contradictory
# "approve" that DOES name blocking issues is now a real hold, not a silent
# pass-through -- routing is driven by blocker count, never by the label.
check "(c3) approve + blockers                -> request_changes:real [was a bug: silently approved]" \
  request_changes:real "$(effective_decision approve 2 0 1)"
# (d) truncated diff always blocks, whatever the label/confidence/blockers
check "(d) rc + 0 blk + trunc + pass_conf=1  -> request_changes:truncated" \
  request_changes:truncated "$(effective_decision request_changes 0 1 1)"
check "(d2) approve + 0 blk + trunc + pass_conf=1 -> request_changes:truncated" \
  request_changes:truncated "$(effective_decision approve 0 1 1)"
check "(d3) approve + 1 blk + trunc          -> request_changes:truncated" \
  request_changes:truncated "$(effective_decision approve 1 1 1)"

# --- effective_decision: real captured flap bodies (2026-07-08) ------------
# ENS-854 evidence: 6 of 8 PRs reviewed today got an empty-blocker
# CHANGES_REQUESTED with a praising body. Every one, verified from the actual
# GitHub Actions run logs (`Parsed decision: ...`), carried decision:"approve",
# blocking_issues:[], and a confidence between 0.72 and 0.78 -- all below the
# 0.80 default APPROVE_THRESHOLD. These fixtures are the real JSON bodies the
# model returned; the guard must coerce every one of them to approve.
F_DEV191=$(fixture dev_pr191 '{"decision":"approve","confidence":0.78,"summary":"The PR cleanly threads gRPC ResourceExhausted through SearchError/QueryEmbeddingsError into a new ApiError::TooManyRequests (429 with Retry-After and a shared machine-readable code), with corresponding call-site cleanup and matching unit/integration tests; the mapping logic (retry-after fallback, status matches, IntoResponse) is internally consistent and matches the stated intent.","blocking_issues":[],"high_risk_notes":""}')
F_DEV190=$(fixture dev_pr190 '{"decision":"approve","confidence":0.72,"summary":"The diff correctly extends the ENS-802 fail-closed gate (used already in /v1/search) to the portal search and query-embeddings handlers, replacing warn-and-serve-free behavior with 400 MODEL_UNPRICED / 503 model_resolution_unavailable, matching the PR stated scope and adding a corresponding test assertion block.","blocking_issues":[],"high_risk_notes":""}')
F_DEV192=$(fixture dev_pr192 '{"decision":"approve","confidence":0.72,"summary":"The vector-passthrough gate is implemented fail-closed (flag AND admin capability AND required model, checked in that order) and all call sites are updated consistently with additive, wire-safe proto/type changes; tests cover the new logic. No security or correctness defect stands out as blocking.","blocking_issues":[],"high_risk_notes":""}')
F_EMBED55=$(fixture embed_pr55 '{"decision":"approve","confidence":0.78,"summary":"The PR correctly moves expensive ESM/substrate/provider health probes to a background TTL-based refresher, with per-provider timeouts and a stale-cache fail-loud fallback, matching its stated intent and adding relevant tests; no security issues or scope creep were found in the diff.","blocking_issues":[],"high_risk_notes":""}')
F_DEPLOY_FLAP1=$(fixture deploy_flap1 '{"decision":"approve","confidence":0.72,"summary":"This is a self-contained benchmarks/ tooling addition (query-vector cache seeding + replay-mode load driver + local mock server + tests) with no changes to production/application code, and it includes fail-closed guardrails (mix check, model/dimension verification, preflight capability probe) consistent with its stated goals.","blocking_issues":[],"high_risk_notes":""}')
F_DEPLOY_FLAP2=$(fixture deploy_flap2 '{"decision":"approve","confidence":0.72,"summary":"This is a benchmarks-only addition (new vector-cache/replay driver code, mock server, and tests) that clearly documents its speculative-contract assumptions and preserves all existing non-replay code paths unchanged; the implementation is fail-closed as claimed and matches the PR description without touching production/workflow files.","blocking_issues":[],"high_risk_notes":""}')

for f in "dev_pr191:$F_DEV191:0.78" "dev_pr190:$F_DEV190:0.72" "dev_pr192:$F_DEV192:0.72" \
         "embed_pr55:$F_EMBED55:0.78" "deploy_flap1:$F_DEPLOY_FLAP1:0.72" "deploy_flap2:$F_DEPLOY_FLAP2:0.72"; do
  name="${f%%:*}"; rest="${f#*:}"; path="${rest%%:*}"; conf="${rest#*:}"
  bc=$(count_blocking_issues "$path")
  pc=$(pass_conf "$conf" "$THRESHOLD_DEFAULT")
  check "flap[$name]: real body -> approve:coerced" approve:coerced \
    "$(effective_decision approve "$bc" 0 "$pc")"
done

# The ONE genuine finding from today (deploy's ens-808-ld0-replay re-review,
# 2026-07-08T16:42Z): a real, concrete, well-articulated blocker
# (harvest-cost misreporting). The guard must NEVER suppress this -- it is
# exactly the case the fix must not collaterally break.
F_DEPLOY_REAL=$(fixture deploy_real '{"decision":"request_changes","confidence":0.6,"summary":"The LD-0 replay driver, cache and tests are well structured and fail-closed as advertised, but the new harvest-documents CLI command performs a live, billed query embedding yet unconditionally prints Marginal embedding cost: $0.000000, contradicting the PRs own core cost-accounting promise.","blocking_issues":["query_vector_cache.py _cmd_harvest_documents prints '\''Marginal embedding cost: $0.000000'\'' after calling harvest_documents(), but harvest_documents() drives /v1/search with real query text, which re-embeds the query on the live stack and is billed; this misleading zero-cost claim is not tracked into seed_cost and could cause real unexpected billing surprise."],"high_risk_notes":""}')
BC_REAL=$(count_blocking_issues "$F_DEPLOY_REAL")
PC_REAL=$(pass_conf 0.6 "$THRESHOLD_DEFAULT")
check "real[deploy_ld0]: genuine blocker -> request_changes:real (MUST NOT suppress)" \
  request_changes:real "$(effective_decision request_changes "$BC_REAL" 0 "$PC_REAL")"

# --- end-to-end: guard over count, mirroring the script's call site ---------
# Empty-block request_changes on a full diff coerces to approve.
BC=$(count_blocking_issues "$F_EMPTY")
check "e2e: empty-block rc, full diff -> approve:coerced" approve:coerced "$(effective_decision request_changes "$BC" 0 1)"
# Whitespace-only blockers are treated as empty -> coerced approve.
BC=$(count_blocking_issues "$F_WS")
check "e2e: whitespace blockers      -> approve:coerced" approve:coerced "$(effective_decision request_changes "$BC" 0 1)"
# The unparseable fail-safe fixture carries a real blocker -> still blocks.
F_FAILSAFE=$(fixture failsafe '{"decision":"request_changes","confidence":0,"blocking_issues":["Unparseable reviewer output"]}')
BC=$(count_blocking_issues "$F_FAILSAFE")
check "e2e: fail-safe unparseable     -> request_changes:real" request_changes:real "$(effective_decision request_changes "$BC" 0 0)"

echo
if [ "$FAILS" -eq 0 ]; then
  echo "ALL PASS"
  exit 0
else
  echo "$FAILS FAILURE(S)"
  exit 1
fi
