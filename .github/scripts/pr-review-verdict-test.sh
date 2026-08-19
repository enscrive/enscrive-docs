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
# HISTORICAL: the retired pre-D4 fail-safe fabricated this exact body for
# unparseable output and posted it as a real review. The Rebalance ADR (D4)
# removed the fabrication — engine failures now retry and then fail RED, and
# the driver's flap ceiling excludes reviews with this body from its count —
# but reviews of this shape still exist on old PRs, and the pure helpers must
# keep treating a named blocker as a real hold.
F_FAILSAFE=$(fixture failsafe '{"decision":"request_changes","confidence":0,"blocking_issues":["Unparseable reviewer output"]}')
BC=$(count_blocking_issues "$F_FAILSAFE")
check "e2e: historical fabricated body -> request_changes:real" request_changes:real "$(effective_decision request_changes "$BC" 0 0)"

# --- parse_model_text: D4 fail-loud parsing + shape validation ---------------
#
# The reviewer sometimes emits narration + mangled tool-call markup instead of
# the required single JSON object. The fixture below is REAL captured output
# (enscrive-code#71 run 31899300151, 2026-08-19), verbatim. parse_model_text
# must refuse it (rc 1) so the driver retries and, failing that, goes RED —
# it must never be massaged into a fabricated verdict. Rebalance ADR D4:
# enscrive-governance/plans/PR-GOVERNANCE-REBALANCE-2026-08-19/ADR.md.
GARBAGE_FIXTURE="$TMP/garbage.txt"
cat > "$GARBAGE_FIXTURE" <<'GARBAGE_EOF'
`Let me verify the change compiles conceptually — checking for other `Summary` literal constructions and the FLEET contents.`

I'll inspect the repo.

<invoke_name>Bash</invoke_name>
<parameter name="command">ls; grep -rn "item_type" --include=*.sql . | head -50</parameter>
</invoke>
GARBAGE_EOF

POUT="$TMP/parsed.json"
VALID_MIN='{"decision":"approve","confidence":0.91,"summary":"Change matches intent.","blocking_issues":[],"high_risk_notes":""}'

parse_model_text "$POUT" "$(cat "$GARBAGE_FIXTURE")"; rc=$?
check "parse: captured tool-call garbage (arg)    -> rc 1" 1 "$rc"
parse_model_text "$POUT" < "$GARBAGE_FIXTURE"; rc=$?
check "parse: captured tool-call garbage (stdin)  -> rc 1" 1 "$rc"

rm -f "$POUT"
parse_model_text "$POUT" "$VALID_MIN"; rc=$?
check "parse: minified decision JSON              -> rc 0" 0 "$rc"
check "parse: decision field preserved" approve "$(jq -r '.decision' "$POUT" 2>/dev/null)"
check "parse: confidence field preserved" 0.91 "$(jq -r '.confidence' "$POUT" 2>/dev/null)"

rm -f "$POUT"
FENCED=$(printf '```json\n%s\n```' "$VALID_MIN")
parse_model_text "$POUT" "$FENCED"; rc=$?
check "parse: same JSON in code fences            -> rc 0" 0 "$rc"
check "parse: fenced decision field" approve "$(jq -r '.decision' "$POUT" 2>/dev/null)"

rm -f "$POUT"
PROSE="Here is my assessment of the change. $VALID_MIN Let me know if anything is unclear."
parse_model_text "$POUT" "$PROSE"; rc=$?
check "parse: JSON embedded mid-prose             -> rc 0" 0 "$rc"
check "parse: mid-prose decision field" approve "$(jq -r '.decision' "$POUT" 2>/dev/null)"

parse_model_text "$POUT" '{"decision":"maybe","confidence":0.9,"summary":"","blocking_issues":[]}'; rc=$?
check "parse: decision \"maybe\" fails shape check  -> rc 1" 1 "$rc"
parse_model_text "$POUT" '{"confidence":0.9,"summary":"no decision field"}'; rc=$?
check "parse: missing decision field              -> rc 1" 1 "$rc"
parse_model_text "$POUT" '{"decision":"request_changes","confidence":0.4,"summary":"x","blocking_issues":["a real defect"]}'; rc=$?
check "parse: request_changes decision            -> rc 0" 0 "$rc"

# --- ENS-4350: root-level HIGH_RISK escalation (negative control) ------------
#
# WHY THIS EXISTS. The HIGH_RISK regex silently failed to escalate
# REPOSITORY-ROOT paths for its entire life. Alternatives written with a
# leading slash — /proto/, /migrations/, /audit, /auth — can never match a root
# path, because there is no preceding separator. Nested paths matched via the
# slash, so the defect left no visible trace: the gate reported protection it
# did not provide, and nothing failed.
#
# The first fix, `(^|/)`, was ALSO wrong, and the way it was wrong is the point.
# It assumed $FILES was newline-delimited. It is not:
#
#   $ gh pr view N --json files -q '[.files[].path]'
#   [".github/CODEOWNERS",".github/scripts/pr-review.sh"]
#
# One compact JSON array on ONE line. `^` therefore matches only before the
# opening bracket, and a root path appears as "proto/embed.proto" — preceded by
# a QUOTE. The verification that "confirmed" the fix used bare path strings
# rather than that JSON, so it reproduced the assumption instead of testing it.
# Correct anchor: (^|["/]).
#
# These assertions run the LIVE regex, extracted from pr-review.sh, against the
# EXACT shape the script greps. Every fixture below is SYNTHESIZED from the
# extracted regex itself — nothing here names a repo-specific path, so this
# file stays byte-identical fleet-wide while adapting to each repo's sentinel
# regex line, per Rebalance ADR D5
# (enscrive-governance/plans/PR-GOVERNANCE-REBALANCE-2026-08-19/ADR.md).
# Alternatives the synthesizer cannot confidently realize print as
# `skip - ...` so coverage gaps stay visible, never silent. The benign
# negatives are not decoration: without them a regex that matched everything
# would pass, which is how a gate starts reporting protection it does not
# provide.
# Extract the HIGH_RISK pattern by SENTINEL, not by "last grep -qiE in the
# file" — that earlier form bound to whatever grep happened to come last, so
# any unrelated grep added later would silently repoint every assertion below
# at the wrong pattern while still reporting PASS. A guard that can quietly
# test the wrong thing is the defect class this file exists to catch.
#
# Each failure mode below reports with a message naming which control failed
# and for which alternative. An earlier revision incremented FAILS in two
# branches for a single missing sentinel, which inflated the count and
# obscured the cause; never double-report one cause.
HR_RE=""
HR_ERR=""
if grep -q 'ENS-4350-REGEX-SENTINEL' "$HERE/pr-review.sh"; then
  HR_RE=$(grep -A6 'ENS-4350-REGEX-SENTINEL' "$HERE/pr-review.sh" \
          | grep -oE "grep -qiE '[^']*'" | head -1 | sed -E "s/grep -qiE '//; s/'$//")
  if [ -z "$HR_RE" ]; then
    # Sentinel present but no single-line `grep -qiE '...'` followed it. Most
    # likely the regex was reflowed across lines, or now contains a single
    # quote that breaks this extraction. Say which, rather than proceeding
    # with an empty pattern — that would make every assertion below fail
    # vacuously and read like a regex regression instead of a harness one.
    HR_ERR="ENS-4350 sentinel found, but no single-line grep -qiE within 6 lines of it (regex reflowed onto two lines, or it now contains a single quote?)"
  fi
elif grep -q '^HIGH_RISK=1$' "$HERE/pr-review.sh"; then
  HR_ERR="unconditional"
else
  HR_ERR="ENS-4350 sentinel missing from pr-review.sh; the guard cannot bind to the HIGH_RISK pattern"
fi

if [ "$HR_ERR" = "unconditional" ]; then
  printf 'ok   - HIGH_RISK is unconditional in this repo; path matching N/A\n'
elif [ -n "$HR_ERR" ]; then
  printf 'FAIL - %s\n' "$HR_ERR"
  FAILS=$((FAILS + 1))
else
  # escalates <path> -> "1" if the live regex matches it in the real $FILES shape
  escalates() { printf '%s' "[\"$1\"]" | grep -qiE "$HR_RE" && echo 1 || echo 0; }

  # Negative control against the reverted anchor. Derive a broken regex from
  # the LIVE one by reverting every (^|["/]) anchor to (^|/) — the exact form
  # the first fix shipped — and assert each synthesized ROOT fixture does NOT
  # match it. If a synthesized fixture ever lands on a path an unanchored
  # alternative also covers, this control fails, instead of the suite
  # silently passing under a reverted anchor.
  BROKEN_RE=$(printf '%s' "$HR_RE" | sed 's/(\^|\["\/\])/(^|\/)/g')
  if [ "$BROKEN_RE" = "$HR_RE" ]; then
    printf 'FAIL - negative control: live HIGH_RISK regex contains no (^|["/]) anchor to revert\n'
    FAILS=$((FAILS + 1))
  fi
  reverted_escalates() { printf '%s' "[\"$1\"]" | grep -qiE "$BROKEN_RE" && echo 1 || echo 0; }

  # realize_stem <fragment> — resolve one regex alternative (anchor already
  # stripped) to a literal path stem: a single leading (a|b|c) group resolves
  # to its first branch, \. unescapes to a dot. Echoes the stem, or returns 1
  # when the fragment holds regex machinery this synthesizer cannot
  # confidently realize (the caller prints a visible skip, never silence).
  realize_stem() {
    local frag="$1" inner group suffix
    case "$frag" in
      "("*)
        inner="${frag#"("}"
        group="${inner%%")"*}"
        suffix="${inner#*")"}"
        frag="${group%%"|"*}$suffix"
        ;;
    esac
    frag="${frag//\\./.}"
    case "$frag" in
      ''|*[!A-Za-z0-9._/-]*) return 1 ;;
    esac
    printf '%s' "$frag"
  }

  # root_fixture <stem> — a repository-ROOT path exercising the stem.
  root_fixture() {
    case "$1" in
      */) printf '%szz-selftest.txt' "$1" ;;
      *-) printf '%szz.sh' "$1" ;;
      *)  printf '%s-zz/selftest' "$1" ;;
    esac
  }

  # Split the live regex into its top-level alternatives (paren-depth aware;
  # a backslash escapes the character after it).
  printf '%s' "$HR_RE" | awk '{
    depth = 0; alt = ""
    for (i = 1; i <= length($0); i++) {
      c = substr($0, i, 1)
      if (c == "\\") { alt = alt c substr($0, i + 1, 1); i++; continue }
      if (c == "(") depth++
      if (c == ")") depth--
      if (c == "|" && depth == 0) { print alt; alt = ""; continue }
      alt = alt c
    }
    print alt
  }' > "$TMP/hr_alternatives.txt"

  ANCHOR='(^|["/])'
  while IFS= read -r alt; do
    [ -n "$alt" ] || continue
    case "$alt" in
      "$ANCHOR"*)
        # Anchored alternative: a repository-ROOT fixture reaches it ONLY
        # through the quote that precedes a root path in the compact JSON
        # array; the nested variant exercises the "/" half of the anchor.
        # Reverting the anchor must un-match the ROOT fixture — proven below
        # against the reverted copy instead of trusted from this comment.
        if stem=$(realize_stem "${alt#"$ANCHOR"}"); then
          fixture=$(root_fixture "$stem")
          check "ENS-4350 root fixture escalates via (^|[\"/]) anchor: $fixture  [$alt]" 1 "$(escalates "$fixture")"
          check "ENS-4350 nested fixture escalates via the / half: src/$fixture" 1 "$(escalates "src/$fixture")"
          check "ENS-4350 negative control, anchor reverted to (^|/): $fixture must NOT match" 0 "$(reverted_escalates "$fixture")"
        else
          printf 'skip - ENS-4350 anchored alternative not synthesizable: %s\n' "$alt"
        fi
        ;;
      *"^"*)
        # An anchor attempt that is not the canonical (^|["/]) form — e.g. a
        # partially-reverted (^|/) — is a defect, not a coverage gap. Fail it
        # by name; a skip here would let a partial revert pass silently.
        printf 'FAIL - ENS-4350 alternative is anchored, but not with the canonical (^|["/]) anchor: %s\n' "$alt"
        FAILS=$((FAILS + 1))
        ;;
      *)
        # Unanchored keyword alternative: real coverage, but it matches with
        # or without the anchors, so it is NOT anchor coverage — that is what
        # the anchored fixtures above are for.
        if stem=$(realize_stem "$alt"); then
          case "$stem" in
            */)  fixture="${stem}zz" ;;
            *.*) fixture="$stem" ;;
            *)   fixture="src/$stem/zz" ;;
          esac
          check "ENS-4350 keyword fixture escalates: $fixture  [$alt]" 1 "$(escalates "$fixture")"
        else
          printf 'skip - ENS-4350 unanchored alternative not synthesizable: %s\n' "$alt"
        fi
        ;;
    esac
  done < "$TMP/hr_alternatives.txt"

  # Benign negative, synthesized to match no alternative by construction: if
  # it matches the live regex anyway, the fixture synthesis collided with an
  # alternative (or the regex went overbroad) — name that, not a bare FAIL.
  BENIGN="zz-benign/zz.txt"
  if [ "$(escalates "$BENIGN")" = "0" ]; then
    printf 'ok   - ENS-4350 benign fixture does NOT escalate: %s\n' "$BENIGN"
  else
    printf 'FAIL - ENS-4350 benign synthesis collided (%s matched the live regex); regex may be overbroad\n' "$BENIGN"
    FAILS=$((FAILS + 1))
  fi
  check "ENS-4350 benign fixture does NOT match the reverted regex: $BENIGN" 0 "$(reverted_escalates "$BENIGN")"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "ALL PASS"
  exit 0
else
  echo "$FAILS FAILURE(S)"
  exit 1
fi
