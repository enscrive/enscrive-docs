//! ENS-6483 credential-shape detection and text redaction, shared by
//! every site in this crate that needs to redact a decoded upstream
//! string before it reaches an error, a log, or a display — ported
//! verbatim (rules and required test vectors) from the fleet ENS-6483
//! redaction spec: `~/work/sec-wave/w3-research/ENS-6483-redaction-spec.md`
//!
//! Extracted from `client.rs` (ENS-6483 R3) once `jobs_polling.rs` needed
//! the same redaction this crate's HTTP client already had — one place
//! these rules live, used from both.
//!
//! General rule for every call site in this crate (and ctt's mirror of
//! this module): redact the DECODED value right before it's surfaced,
//! never raw JSON text — a `\uXXXX`-escaped credential survives a
//! raw-text scan untouched, since the literal bytes never spell out the
//! credential or its shape.

/// Rule 2's generic vendor/platform-key prefixes — each needs
/// [`MIN_KEY_SUFFIX_LEN`] more `[a-z0-9_-]` characters after it to count
/// (the trailing run is the entropy signal; a bare prefix with nothing
/// after it isn't itself suspicious). The fleet spec's own list is a
/// FLOOR, not an exact set: redacting more genuine credential shapes is
/// the safe direction, and the boundary anchor plus the 16-char suffix
/// requirement keep false positives on ordinary hosts negligible either
/// way. This list is therefore a superset of the spec's minimum.
const KEY_PREFIXES: &[&str] = &[
    "sk-ant-",
    "sk-proj-",
    "sk-",
    "sk_",
    "rk-",
    "pk-",
    // Slack: one letter after "xox" names the token class (a = app,
    // b = bot, p = user/legacy, r = refresh, s = workspace). Bare
    // "xox-" is deliberately excluded — with no letter to require, it
    // would match any ordinary "xox..." substring at a boundary too.
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "xoxr-",
    "xoxs-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "aiza",
    "ya29.",
    "glpat-",
    "npg_",
];
const MIN_KEY_SUFFIX_LEN: usize = 16;
const MIN_OPAQUE_RUN_LEN: usize = 32;
// This product's own key shape: `enscrive_<8 hex>_`, a fixed,
// self-contained form handled separately from the generic prefixes.
const ENSCRIVE_PREFIX: &str = "enscrive_";
const ENSCRIVE_ID_LEN: usize = 8;

fn is_boundary(c: char) -> bool {
    !c.is_ascii_alphanumeric()
}

fn is_key_suffix_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Byte spans in `lower` (already ASCII-lowercased) matched by rule 2 (a
/// boundary-anchored key prefix) or rule 3 (a 32+ char opaque run).
/// [`host_looks_credential_shaped`] is just "is this non-empty"; a
/// bounded text excerpt ([`redact_excerpt`]) needs the actual spans so it
/// can replace each match rather than blacklisting the whole text.
///
/// Boundary anchoring (not "starts a DNS label") is what makes
/// `prefix-sk-<16+ chars>` count as a match — the hyphen before `sk-` is a
/// boundary too — while a host like `network-edge.example.com` or an ALB
/// name like `my-keycloak-loadbalancer-1234567890.us-east-1.elb.amazonaws.com`
/// stays named: the `rk-`/`sk-`-shaped substrings inside them
/// ("netwo**rk-**edge") follow an alphanumeric character, not a boundary,
/// and none of their hyphen/dot-separated segments reach the suffix or
/// run-length thresholds.
fn shape_spans(lower: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();

    let mut prev_char: Option<char> = None;
    for (i, c) in lower.char_indices() {
        let at_boundary = match prev_char {
            None => true,
            Some(p) => is_boundary(p),
        };
        prev_char = Some(c);
        if !at_boundary {
            continue;
        }
        let rest = &lower[i..];

        if let Some(after) = rest.strip_prefix(ENSCRIVE_PREFIX) {
            let hex_len = after
                .chars()
                .take(ENSCRIVE_ID_LEN)
                .take_while(char::is_ascii_hexdigit)
                .count();
            if hex_len == ENSCRIVE_ID_LEN && after.as_bytes().get(ENSCRIVE_ID_LEN) == Some(&b'_') {
                let shape_len = ENSCRIVE_PREFIX.len() + ENSCRIVE_ID_LEN + 1;
                // ENS-6483 (ctt Sol round 4, M2 — same bug applied here
                // too): the fixed shape alone is enough to DETECT a
                // credential (a host-redaction boolean doesn't care how
                // long the matched span is), but TEXT redaction must
                // remove the whole token, not just its recognizable
                // prefix — continue consuming the token's remaining
                // key-alphabet characters past the shape, so the actual
                // secret material after "enscrive_<8 hex>_" is covered
                // too.
                let extra_len = after[ENSCRIVE_ID_LEN + 1..]
                    .chars()
                    .take_while(|&c| is_key_suffix_char(c))
                    .count();
                spans.push((i, i + shape_len + extra_len));
            }
        }

        for prefix in KEY_PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                let suffix_len = after.chars().take_while(|&c| is_key_suffix_char(c)).count();
                if suffix_len >= MIN_KEY_SUFFIX_LEN {
                    spans.push((i, i + prefix.len() + suffix_len));
                }
            }
        }
    }

    // Rule 3: a run of 32+ consecutive [a-z0-9_] characters; '-' and '.'
    // (like any other non-matching character) break it.
    let mut run_start: Option<usize> = None;
    let mut run_len = 0usize;
    let mut cursor = 0usize;
    for (i, c) in lower.char_indices() {
        cursor = i + c.len_utf8();
        if c.is_ascii_alphanumeric() || c == '_' {
            if run_start.is_none() {
                run_start = Some(i);
                run_len = 0;
            }
            run_len += 1;
        } else {
            if let Some(start) = run_start.take()
                && run_len >= MIN_OPAQUE_RUN_LEN
            {
                spans.push((start, i));
            }
            run_len = 0;
        }
    }
    if let Some(start) = run_start
        && run_len >= MIN_OPAQUE_RUN_LEN
    {
        spans.push((start, cursor));
    }

    spans
}

/// Rule 2 + rule 3: does `host_lower` (already ASCII-lowercased) contain a
/// credential-shaped substring anywhere?
pub(crate) fn host_looks_credential_shaped(host_lower: &str) -> bool {
    !shape_spans(host_lower).is_empty()
}

/// Byte spans in `lower` (already ASCII-lowercased) where any (non-empty)
/// entry of `credentials` occurs, case-insensitively.
fn credential_spans(lower: &str, credentials: &[&str]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for cred in credentials {
        if cred.is_empty() {
            continue;
        }
        let cred_lower = cred.to_ascii_lowercase();
        let mut start = 0usize;
        while let Some(pos) = lower[start..].find(&cred_lower) {
            let abs_start = start + pos;
            let abs_end = abs_start + cred_lower.len();
            spans.push((abs_start, abs_end));
            start = abs_end;
        }
    }
    spans
}

fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if spans.is_empty() {
        return spans;
    }
    spans.sort_unstable_by_key(|&(s, _)| s);
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Replace every rule-1 (live credential, case-insensitive) or rule-2/3
/// (key-shape) span in `text` with `[redacted]`, then bound the result to
/// `max_chars` characters. Used for a non-3xx error body excerpt — a
/// redirect's body must never be read at all, but a 4xx/5xx body, a
/// parse-failure message, or a decoded upstream field (a job's
/// `error_message`, say) can otherwise echo a credential straight back.
pub(crate) fn redact_excerpt(text: &str, credentials: &[&str], max_chars: usize) -> String {
    let lower = text.to_ascii_lowercase();
    let mut spans = credential_spans(&lower, credentials);
    spans.extend(shape_spans(&lower));
    let spans = merge_spans(spans);

    let mut out = String::new();
    let mut last = 0usize;
    for (start, end) in spans {
        out.push_str(&text[last..start]);
        out.push_str("[redacted]");
        last = end;
    }
    out.push_str(&text[last..]);

    out.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ENS-6483 (Sol round 4, M2 — the bug ctt's redact.rs and this
    /// crate's client.rs shared before this fix): an `enscrive_<8 hex>_`
    /// token's SHAPE recognition stops right after the fixed
    /// prefix+hex+underscore (that's all `host_looks_credential_shaped`
    /// needs for a boolean check), but the actual secret material
    /// continues past that point in the real key format — TEXT redaction
    /// must consume the whole token, not just its recognizable shape
    /// prefix, or the tail of a real key survives redaction untouched.
    /// The token is exactly 31 characters total (18 for the shape + a
    /// 13-char suffix) — one under rule 3's 32-char opaque-run threshold,
    /// so rule 3 can't be what's silently covering the gap.
    #[test]
    fn redact_excerpt_redacts_the_whole_enscrive_token_not_just_its_shape_prefix() {
        let token = "enscrive_deadbeef_qrstuvwxyzabc";
        assert_eq!(token.len(), 31, "fixture must stay under the 32-char opaque-run threshold");
        let text = format!("upstream said: invalid key {token} for this request");
        let out = redact_excerpt(&text, &[], 200);
        assert!(!out.contains(token), "the full token leaked into: {out}");
        assert!(
            !out.contains("qrstuvwxyzabc"),
            "the token's suffix (the actual secret material past the \
             enscrive_<8 hex>_ shape) leaked into: {out}"
        );
        assert!(out.contains("[redacted]"), "expected a redaction marker: {out}");
    }
}
