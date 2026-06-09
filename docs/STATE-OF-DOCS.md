# State of `enscrive-docs` — Platform Documentation

> **Audit date:** 2026-06-09 · **Branch:** `main` · **Version:** 0.0.2  
> **Auditor:** independent code review (ENS-607)

This document anchors a line-by-line code audit of the `enscrive-docs` repository.
It answers the founder's open question about dog-food integration, scores the tool
against the platform mission, and catalogs every finding from the audit.
Linked documents cover architecture, crate internals, the command reference, and the
HTTP API surface.

---

## Executive Assessment

**`enscrive-docs` is a coherent, honest implementation of its stated purpose.**

The tool does exactly what the README claims: it walks a directory of markdown files,
pushes them into Enscrive corpora and voices via the public `/v1` API, and serves a
polished HTML documentation site with a neural search overlay.  
The code is clean Rust, well-structured across three crates, and the existing tests
cover the most critical logic paths (markdown rendering, URL generation, config
parsing, text fragment encoding, asset embedding).

**The dog-food answer is YES — unambiguously.** See §2 below.

The tool is **pre-alpha and not yet launch-ready** for v1. A small cluster of dead
config fields, one non-functional CLI flag, and two deferred features (TOC anchor
IDs, multi-version) create a gap between what the config schema promises and what
the binary delivers today. These are documented here; none are architectural.

---

## Doc Set

| Document | Contents |
|---|---|
| **[STATE-OF-DOCS.md](STATE-OF-DOCS.md)** | This file — audit, assessment, findings |
| **[architecture.md](architecture.md)** | Crate map, ingest + search pipeline diagrams |
| **[commands.md](commands.md)** | Full command reference with walkthroughs |
| **[api-surface.md](api-surface.md)** | HTTP routes, request/response shapes |
| **[crate-map.md](crate-map.md)** | Module-level map of every `.rs` file |

---

## Assessment Rubric

### 1 · Mission Alignment

**Score: HIGH**

The platform mission is to give AI and humans limitless persistent memory via a pure
embedding + neural-search engine unified by Agent Voices.

`enscrive-docs` fits cleanly: it is a first-class *showcase* and *dog-food* of that
platform, turning any markdown directory into a neural-search-native documentation
site. The positioning in README and examples is accurate. The code implements what it
advertises with no off-platform shortcuts.

The tool serves three audiences the platform targets: humans (HTML + theme), agents
(JSON format endpoints, `/llms.txt`, CORS-open `/search`), and HTTP consumers (every
surface is an HTTP API).

### 2 · Contract Truth — Does It Use the Real Enscrive Search Path?

**Score: YES — FULL INTEGRATION, no bypasses**

This is the core question. The answer is **yes, completely, at every layer**:

#### Ingest path (verified in `crates/cli/src/commands/ingest.rs`)

1. `build_documents()` reads each `.md` file from disk and attaches a SHA-256
   fingerprint (via `sha2` + `hex`). No embedding happens here.
2. An `IngestRequest` is POSTed to **`POST /v1/ingest`** on `api.enscrive.io` with:
   - `corpus_id` — resolved by listing `/v1/corpora` and matching by name
   - `voice_id` — resolved by listing `/v1/voices` and matching by name
   - `documents[]` — raw markdown content + fingerprint per file
   - `sync: Some(true)` — caller waits for chunking + embedding to complete
3. Enscrive performs all chunking (per the voice's `chunking_strategy`) and all
   embedding (per the corpus's `embedding_model`) server-side.
4. The client receives `IngestProgressEvent` objects confirming chunks created and
   embeddings stored.

There is **no local embedding library** (no `ort`, `fastembed`, `sentence-transformers`,
`candle`, or similar) anywhere in `Cargo.lock`. The binary cannot produce embeddings
independently of the Enscrive API.

#### Search path (verified in `crates/cli/src/commands/serve.rs` + `search.rs`)

1. Every `/search?q=...` HTTP request (and every `enscrive-docs search` invocation)
   resolves a voice and calls either:
   - **`POST /v1/voices/search`** — the differentiated voice-tuned neural search path
     (used when a voice is resolvable, which is the default configuration)
   - **`POST /v1/search`** — plain corpus search (fallback only when no voice is resolved)
2. The query is never embedded locally. Enscrive embeds the query, runs ANN against the
   corpus, and returns scored `SearchResultItem` objects with content and score.
3. The serve layer enriches results with page URLs and Text Fragment suffixes, then
   returns them to the browser or API consumer.

**Conclusion:** enscrive-docs is eating its own cooking. Every embedding and every
neural search operation is delegated to the Enscrive platform via the public `/v1` API.

### 3 · Correctness & Determinism

**Score: MOSTLY CORRECT — four actionable findings**

Core behaviors (ingest, serve, search, bootstrap, reset, voice tune) are correctly
implemented and deterministic. Config resolution is well-specified (5-level
precedence, tested). The markdown pipeline (frontmatter → HTML → anchor collection)
is correct and tested.

**Findings (documented here; not fixed in this PR):**

| ID | Location | Finding |
|---|---|---|
| F-01 | `SearchConfig.include_snippets` | Config field declared, scaffolded, and documented but **never read** in serve or search pipelines. No behavioral effect. |
| F-02 | `IngestArgs.force` | `--force` CLI flag is declared and parsed, **never read** in `ingest::run()`. The "force re-ingest" behavior doesn't exist on the client side (fingerprint bypass would need to be a server-side parameter). |
| F-03 | `render_anchor_list()` + `collect_anchors()` | TOC `<a href="#section-slug">` links are generated but the rendered HTML **has no `id=` attributes** on heading elements. Clicking TOC links scrolls nowhere. Explicitly deferred in `markdown.rs:127`. |
| F-04 | `logo_path`, `template_dir` in `ThemeConfig` | Both declared in config, scaffolded by `init`, mentioned in example docs. `setup_state()` in `serve.rs` reads neither. Dead config keys. |

### 4 · Drift & Dead Code

**Score: LOW-MODERATE — several dead surfaces, no architectural drift**

| Item | Location | Status |
|---|---|---|
| `Theme` struct + `with_accent_color`, `with_custom_css`, `css_variables()` | `render/src/theme.rs` | Exported from `lib.rs`, zero callers in the serve pipeline. `serve.rs` uses `build_theme_variables()` directly. Dead exported type. |
| `_voice: &VoiceConfig` param | `ingest.rs:build_documents()` | Underscore-prefixed, unused. Voice config is only needed server-side for chunking. |
| `let _ = config_dir;` | `bootstrap.rs:128` | Vestigial binding; `config_dir` was computed but `ingest::run()` re-derives it internally. |
| `[[versions]]` config | `config.rs:VersionConfig` | In schema, parsed correctly, but `cfg.versions` is read nowhere in any command handler. Documented as deferred in `configuration.md`. |
| SSE `/_events` route in plain `serve` | `serve.rs:handle_events` | In plain `serve` mode `event_tx` is `None`; the handler opens a never-yielding broadcast receiver and keeps the connection open with keep-alive. Works correctly but the route is semantically inert outside `watch` mode. |

### 5 · V1 Launch Readiness

**Score: NOT READY — pre-alpha, specific blockers identified**

| Blocker | Severity | Notes |
|---|---|---|
| TOC anchor IDs not injected (F-03) | Medium | User-visible: TOC exists but is non-functional. Needs heading ID injection in the pulldown-cmark event stream. |
| Dead config fields confuse operators (F-01, F-04) | Medium | `include_snippets`, `logo_path`, `template_dir` do nothing. An operator who sets them expects a result and gets none — silent failure. |
| `--force` flag does nothing (F-02) | Low | Documented gap; operators who expect forced re-ingest will be confused. |
| Release matrix Linux x86_64 only | High | `release.yml` builds a single target (`x86_64-unknown-linux-gnu`). README advertises `brew install enscrive/tap/enscrive-docs` but no macOS binary is produced. The Homebrew tap would have nothing to serve. |
| No crates.io publish | Medium | README says `cargo install enscrive-docs` but the package is not on crates.io. Workaround in `troubleshooting.md`. |
| Cosign signing absent | Low | ENS-82 follow-on. Explicitly noted in `release.yml`. DEV channel only anyway. |
| `[[versions]]` unimplemented | Low | In schema and docs; does nothing at runtime. Low severity because it's explicitly marked deferred. |

---

## Flagged Cross-Repo Finding

**Proposed ADR direction:** The `IngestRequest.no_batch` field (in `types.rs:130`)
and the `SearchQuery` parameters `hybrid_alpha`, `resolution`, `oversample_factor`,
and `granularity` are tunneled to the upstream API but have no config surface in
`enscrive-docs.toml` and no CLI flags. As Enscrive's search surface expands
(LanceDB/Qdrant substrate pivot, batch async), these parameters will need a config
surface in `enscrive-docs.toml`. The current approach (zero-wiring of advanced params)
is correct for a pre-alpha but will need an ADR for the v1 config contract.
This is a cross-repo contract question (enscrive-developer ↔ enscrive-docs) and is
flagged here rather than addressed.
