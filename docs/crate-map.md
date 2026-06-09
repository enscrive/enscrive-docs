# Crate Map — Module-Level Reference

> Line-by-line audit of every `.rs` file on `main` (v0.0.2).  
> Status labels: **shipped** · **partial** (present but incomplete) · **dead** (compiled in, no callers)

---

## `crates/core` — `enscrive-docs-core`

Library crate. Published separately so other Rust applications can use the Enscrive
API client and config loader without pulling in the CLI or renderer.

### `src/client.rs` — `EnscriveClient`

**Status: shipped**

The only HTTP client in the codebase. Wraps `reqwest::Client` with:

- `X-API-Key` header on every request
- Optional `X-Embedding-Provider-Key` header (BYOK embedding)
- 120-second timeout
- Base URL normalization (trailing `/` stripped)

**Methods:**

| Method | HTTP call | Notes |
|---|---|---|
| `list_corpora()` | `GET /v1/corpora` | Returns `Vec<CorpusDetail>` |
| `get_corpus(id)` | `GET /v1/corpora/{id}` | |
| `create_corpus(req)` | `POST /v1/corpora` | |
| `delete_corpus(id)` | `DELETE /v1/corpora/{id}` | |
| `list_voices()` | `GET /v1/voices` | Returns `Vec<VoiceDetail>` |
| `get_voice(id)` | `GET /v1/voices/{id}` | |
| `create_voice(req)` | `POST /v1/voices` | |
| `update_voice(id, req)` | `PUT /v1/voices/{id}` | Full-replace semantics |
| `delete_voice(id)` | `DELETE /v1/voices/{id}` | |
| `ingest(req)` | `POST /v1/ingest` | Buffered — waits for all progress events |
| `ingest_stream(req)` | `POST /v1/ingest` + `Accept: text/event-stream` | SSE streaming via tokio mpsc channel |
| `search(query)` | `POST /v1/search` | Plain corpus search |
| `search_with_voice(body)` | `POST /v1/voices/search` | Voice-tuned search (primary path) |
| `ping()` | `GET /v1/corpora` | Returns `StatusCode` — used as health check |

`ingest_stream()` is implemented and exported but **not called** by any command in
the current CLI — `ingest::run()` uses the buffered `ingest()` method. The streaming
variant exists for future use (progress bars, large corpus streaming).

### `src/config.rs` — `Config` loader

**Status: shipped (with dead fields)**

Loads and serializes `enscrive-docs.toml`. All `resolved_*` methods implement the
5-level precedence chain (CLI flag > env var > config > profile > default).

**Config structs and their status:**

| Struct | Fields | Status |
|---|---|---|
| `EnscriveAuthConfig` | `profile`, `api_key`, `endpoint`, `embedding_provider_key` | All shipped |
| `SiteConfig` | `title`, `description`, `base_url`, `base_path`, `default_version` | Shipped except `default_version` (no multi-version support yet) |
| `ThemeConfig` | `variant`, `accent_color`, `logo_path`, `custom_css`, `template_dir` | `logo_path` and `template_dir` are dead (parsed, never read in serve) |
| `CorpusConfig` | `name`, `voice`, `path`, `glob`, `url_prefix`, `embedding_model`, `dimensions`, `description` | All shipped |
| `VoiceConfig` | `name`, `chunking_strategy`, `parameters`, `template_id`, `score_threshold`, `default_limit`, `description` | All shipped |
| `VersionConfig` | `slug`, `corpora`, `default` | Dead — parsed, never read by any command handler |
| `SearchConfig` | `default_voice`, `results_per_page`, `include_snippets` | `include_snippets` is dead (parsed, never read) |
| `ServeConfig` | `port` | Shipped |
| `ReturnConfig` | `url`, `label` | Shipped — renders optional "Return to app" link |

**Tests:** 4 unit tests in `mod tests` cover parsing, `return_to` block, default
label, and endpoint override precedence.

### `src/types.rs` — API types

**Status: shipped**

Mirrors a subset of `enscrive-developer/crates/types-api/src/lib.rs`. Every field
maps 1:1 to the upstream JSON contract.

Key types: `CorpusDetail`, `VoiceDetail`, `VoiceConfigApi`, `CreateVoiceApiRequest`,
`UpdateVoiceApiRequest`, `CreateCorpusRequest`, `IngestDocument`, `IngestRequest`,
`IngestProgressEvent`, `SearchQuery`, `SearchWithVoiceBody`, `SearchResults`,
`SearchResultItem`, `SearchFilter`.

`IngestRequest` includes `no_batch: Option<bool>` and `sync: Option<bool>` that map
to upstream batch/async control knobs. `sync: Some(true)` is hardcoded in
`ingest::run()`. `no_batch` is never set (always `None`). Advanced search parameters
(`hybrid_alpha`, `resolution`, `oversample_factor`, `granularity`) exist in
`SearchQuery` and `SearchWithVoiceBody` but have no config surface in
`enscrive-docs.toml`.

### `src/error.rs` — `EnscriveError`

**Status: shipped**

`thiserror`-derived error enum with variants: `Http` (status + body), `Request`
(reqwest), `Parse` (serde_json), `Config` (string), `Io`, `Toml`, `Other`.

---

## `crates/render` — `enscrive-docs-render`

Library crate. Can be embedded by other Rust applications for markdown-to-HTML
rendering without the CLI.

### `src/markdown.rs` — Markdown pipeline

**Status: shipped (with one deferred item)**

`render_markdown(raw: &str) -> RenderedMarkdown` is the single entry point.
Pipeline:

1. `gray_matter` parses YAML frontmatter (`---` blocks). Unknown keys are silently
   ignored; only `title`, `description`, `order`, `draft` are deserialized.
2. `pulldown-cmark` renders the body to HTML with tables, footnotes, strikethrough,
   and task lists enabled.
3. `ammonia` sanitizes the HTML — allows `pre`, `code`, `id`, `class` attributes;
   strips `<script>`, `<iframe>`, and other dangerous tags.
4. `strip_leading_h1()` detects and removes the first `<h1>` from the rendered HTML,
   returning its plain text for use as the page title (avoids double-rendering the title
   — once in the template H1, once in the prose body).
5. `collect_anchors()` runs a second pass over the raw markdown to extract h2/h3
   heading text and slugify it for the TOC.

**⚠️ Deferred — heading IDs:** `collect_anchors()` returns slugs but does not
inject `id="..."` attributes onto `<h2>`/`<h3>` elements in the rendered HTML. The
code comment at `markdown.rs:127` explicitly notes this requires a custom
pulldown-cmark event-stream rewriter and is deferred. TOC anchor links point to
`#section-slug` but no element in the rendered HTML has that `id` — clicking them
scrolls nowhere. (Finding F-03 in STATE-OF-DOCS.md.)

**Tests:** 8 unit tests covering basic rendering, frontmatter, anchor collection,
HTML sanitization, H1 stripping, subsequent H1 preservation, and frontmatter/H1
independence.

### `src/page.rs` — `Page` + `PageMeta`

**Status: shipped**

In-memory representation of a rendered documentation page.

`PageMeta::build()` resolves the page title via: frontmatter `title` → leading H1
text → slug-derived title (spaces from `-`/`_`/`/` separators, each word capitalized).

`PageMeta::url()` builds the page URL from `base_path` + optional `url_prefix` +
slug. Tested for correctness.

**Tests:** 2 unit tests for URL construction and slug-derived title fallback.

### `src/templates.rs` — Askama wiring

**Status: shipped**

Thin wrapper around Askama templates. The `.html` files in `crates/render/templates/`
are compiled into the crate at build time; no template files are needed at runtime.

**Exported functions:**
- `render_page(ctx: &PageContext) -> Result<String, askama::Error>`
- `render_index(ctx: &IndexContext) -> Result<String, askama::Error>`
- `build_nav(pages: &[PageMeta], base_path: &str, current_slug: Option<&str>) -> Vec<NavItem>`
- `render_anchor_list(anchors: &[HeadingAnchor]) -> String` — generates `<ul class="toc">` with `<a href="#slug">` links

**Tests:** 2 unit tests verifying the `[return_to]` link renders when present and
is absent when not configured.

### `src/theme.rs` — `ThemeVariant` + `Theme`

**Status: partial — `Theme` struct is dead code**

`ThemeVariant` enum: `Neutral` (default), `Enscrive`. Used by `serve.rs` to select
the CSS asset path. `from_str_loose()` is case-insensitive; unrecognized values fall
back to `Neutral`.

`Theme` struct with `accent_color`, `custom_css`, `logo_data_url`, `css_variables()`
— **dead code.** Exported from `lib.rs` but has zero callers in the serve pipeline.
`serve.rs` implements `build_theme_variables()` directly without going through this
struct. This is a split implementation — the struct was built as a reusable
abstraction but the CLI bypasses it.

### `src/assets.rs` — `EmbeddedAssets`

**Status: shipped**

`rust-embed` derive macro on `struct EmbeddedAssets` with
`#[folder = "../../assets"]`. Compiles the entire `assets/` directory into the binary
at build time with `compression` enabled.

`embedded_asset(path: &str) -> Option<Vec<u8>>` is the single accessor.

**Tests:** 2 unit tests asserting that `themes/neutral/style.css` and `js/search.js`
are non-empty in the embedded bundle.

### `templates/_base.html`

Askama base template. Contains:
- `<head>` with theme CSS link, optional accent variable override, optional custom CSS
- Watch-mode SSE listener (injected only when `ctx.watch_mode == true`)
- Sticky header with site title brand link, ⌘K search trigger button, optional
  `[return_to]` link
- Sidebar nav (sorted `NavItem` list, `current` class on active page)
- Main content `<main id="main">` with `{% block content %}`
- Search overlay `<div id="ed-search-overlay">` wired to `search.js`
- "Powered by Enscrive · neural search" attribution in the search hint

### `templates/page.html`

Extends `_base.html`. Renders `<h1>` from `page_title`, optional TOC aside (from
`page_anchors_html`), and the sanitized markdown HTML in `.ed-prose-body`.

### `templates/index.html`

Extends `_base.html`. Renders a hero section with the site title, tagline, and a
primary ⌘K search button; below it an "All pages" grid.

---

## `crates/cli` — `enscrive-docs` binary

### `src/main.rs`

Clap command dispatch. Builds the `Cli` struct with `GlobalArgs` flattened and a
`Command` subcommand enum. Dispatches to each command's `run()` async function.
Prints error to stderr and exits non-zero on `Err`. VERSION string from
`version.rs` (semver + git SHA + build date).

### `src/global.rs` — `GlobalArgs`

Clap `Args` struct for the five global flags. `resolved_config_path()` returns
`--config` if set, otherwise `./enscrive-docs.toml` in the current directory.

### `src/version.rs`

Constructs `VERSION` at compile time from `CARGO_PKG_VERSION`, `ENSCRIVE_GIT_SHA`,
and `ENSCRIVE_BUILD_DATE` (both set by `build.rs`).

### `build.rs`

Bakes git SHA (7 chars, with `-dirty` suffix if the tree is modified) and build date
into Cargo env vars at compile time. Uses Howard Hinnant's `civil_from_days`
algorithm for pure-Rust date conversion (no external crate). Falls back to
`GITHUB_SHA` env var in CI when `.git/` is absent.

### `src/commands/init.rs` — **shipped**

Writes a scaffolded `enscrive-docs.toml`. Probes `~/.config/enscrive/profiles.toml`
to auto-populate the `[enscrive] profile` field. See [commands.md](commands.md).

### `src/commands/bootstrap.rs` — **shipped**

Idempotent voice + corpus creation, then first ingest. See [commands.md](commands.md).

Note: `let _ = config_dir;` at line 128 is a vestigial binding — the variable is
computed but `ingest::run()` re-derives the config directory internally.

### `src/commands/ingest.rs` — **shipped (with caveats)**

See [commands.md](commands.md) for the full walkthrough.

`build_documents()` takes `_voice: &VoiceConfig` (underscore-prefixed, unused) —
voice config is only needed server-side for chunking; the parameter is vestigial.

`derive_extension_from_glob()` extracts only the file extension from a glob string.
Not a full glob engine: `**/*.md` → `.md` works; complex patterns like `*.{md,mdx}`
would yield `{md,mdx}` (incorrect match). Acceptable for the common case.

`--force` flag is parsed but `args.force` is never read. Finding F-02.

### `src/commands/serve.rs` — **shipped**

Largest file in the codebase (~993 lines). Sets up the Axum router and all route
handlers. See [api-surface.md](api-surface.md) for the route table.

Key internal functions:
- `setup_state()` — builds `AppState` (shared between serve and watch)
- `build_pages()` — walks markdown, renders, populates in-memory cache
- `rebuild_pages()` — atomically swaps the cache (called by watch)
- `make_slug()` — converts `relative/path/to/file.md` → `relative/path/to/file`
  with `/index`, `/README`, `/readme` collapse
- `build_text_fragment()` — extracts a 4–12 word phrase for `#:~:text=...` URLs
- `percent_encode_fragment()` — encodes fragment text (spaces → `%20`)

`serve.rs` has the richest test suite in the codebase (8 unit tests covering text
fragment extraction, markdown stripping, encoding edge cases).

`load_custom_css()` reads `ThemeConfig.custom_css` and injects it. This works.
`ThemeConfig.logo_path` and `template_dir` are **not read**. Finding F-04.

### `src/commands/watch.rs` — **shipped**

Wraps `serve` with a `notify::recommended_watcher`. Uses a sync `mpsc` channel from
`notify` to a blocking thread, bridged to a `tokio::sync::mpsc` channel for the
async debounce loop. Watcher stays alive inside the spawned tokio task via `_watcher`
binding.

`is_relevant()` filter: accepts `Modify(Data)`, `Modify(Any)`, `Modify(Name)`,
`Create`, `Remove(File)`, `Remove(Any)` events on `.md` files; rejects editor temp
files.

### `src/commands/search.rs` — **shipped**

Three output formatters: `print_human`, `print_markdown`, `print_json`. Uses the
same voice/corpus resolution logic as `serve.rs` for consistent behavior across
CLI and HTTP surfaces.

### `src/commands/voice.rs` — **shipped**

Only exposes `voice tune`. Future subcommands (list, show, create, delete) are
noted in a comment as future slots.

`open_in_editor()` writes to `/tmp/enscrive-docs-voice-{name}-{pid}.toml`, invokes
`$EDITOR` (default: `vi`), reads back, and cleans up. Header comment lines are
stripped before the round-trip diff comparison so an unchanged file aborts cleanly.

### `src/commands/reset.rs` — **shipped**

Requires `--yes`. Deletes corpora, then calls `bootstrap::run()`. Uses let-else and
`&&` in if-let chains (Rust 2024 edition features).

### `src/commands/config.rs` — **shipped**

Loads config and prints it as TOML. `--validate` exits `0` without printing.
