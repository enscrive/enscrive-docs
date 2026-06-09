# HTTP API Surface

> All routes verified against `crates/cli/src/commands/serve.rs` on `main`.  
> Status: **shipped** unless otherwise noted.

## Route Table

| Method | Route | Handler | Description |
|---|---|---|---|
| `GET` | `/` | `handle_index` | Index page — site hero + all-pages list (HTML) |
| `GET` | `/{slug}` | `handle_page` | Rendered documentation page (HTML) |
| `GET` | `/{slug}?format=md` | `handle_page` | Raw markdown source |
| `GET` | `/{slug}?format=json` | `handle_page` | Page data as JSON |
| `GET` | `/search` | `handle_search` | Neural search results (JSON) |
| `GET` | `/llms.txt` | `handle_llms_txt` | LLM-friendly page index (plain text) |
| `GET` | `/sitemap.xml` | `handle_sitemap` | XML sitemap |
| `GET` | `/healthz` | `handle_healthz` | Liveness check — responds `ok` |
| `GET` | `/_assets/*path` | `handle_asset` | Embedded static assets (CSS, JS) |
| `GET` | `/_events` | `handle_events` | SSE live reload stream (watch mode) |

When `base_path` is configured (e.g. `/docs`), all routes are prefixed: `/docs/search`,
`/docs/_assets/...`, etc. The Axum router uses `Router::nest(base_path, routes)`.

---

## `GET /search`

The primary search endpoint, consumed by the browser ⌘K palette and directly by
agents and API consumers.

### Request

```
GET /search?q=<query>[&voice=<voice_name>][&corpus=<corpus_name>][&limit=<n>]
```

| Parameter | Type | Default | Description |
|---|---|---|---|
| `q` | string | — | Query string. Empty/missing returns `{results: []}` |
| `voice` | string | corpus's configured voice → `[search] default_voice` | Voice name override |
| `corpus` | string | first `[[corpora]]` entry | Corpus name filter |
| `limit` | integer | `[search] results_per_page` (default `10`) | Max results |

### Response

```json
{
  "query": "trademark policy",
  "search_time_ms": 215,
  "total_candidates": 3,
  "results": [
    {
      "document_id": "TRADEMARKS.md",
      "score": 0.368,
      "snippet": "# Trademark Policy The names Enscrive, enscrive-docs ...",
      "url": "/trademarks#:~:text=Trademark%20Policy%20The%20names",
      "title": "Trademarks",
      "corpus_id": "cps_abc123..."
    }
  ]
}
```

| Field | Description |
|---|---|
| `query` | The query string as received |
| `search_time_ms` | Time Enscrive spent on ANN lookup |
| `total_candidates` | Number of chunks scored before limit |
| `results[].document_id` | Relative path of the source markdown file |
| `results[].score` | Cosine similarity (0–1). Higher = more relevant |
| `results[].snippet` | First 280 characters of the matching chunk |
| `results[].url` | Full URL with Text Fragment suffix (`#:~:text=...`) for scroll-to-passage |
| `results[].title` | Page title resolved from frontmatter or H1 heading |
| `results[].corpus_id` | Enscrive corpus ID of the matching document |

### Error response (502)

When the Enscrive upstream search fails:

```json
{
  "error": "upstream_search_failed",
  "detail": "<error message from Enscrive API>"
}
```

### Search path selection

1. If a voice can be resolved (via `?voice=`, corpus default, or `[search] default_voice`):
   → `POST /v1/voices/search` on `api.enscrive.io` (voice-tuned neural search)
2. Otherwise:
   → `POST /v1/search` (plain corpus search)

Voice-tuned search applies the voice's `score_threshold`, `default_limit`, and
`chunking_strategy` parameters to the retrieval. It is the differentiated capability.

### CORS

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET
```

All origins are permitted so browser apps and agents can call `/search` directly.

---

## `GET /{slug}`

Renders a documentation page.

### HTML (default)

Returns a full HTML page using the configured theme. The page includes:
- Sticky header with site title, ⌘K search trigger, optional Return link
- Left sidebar with sorted navigation
- Per-page TOC (rendered from h2/h3 anchors) — **⚠️ links are non-functional in v0.0.2** (TOC IDs not injected; see F-03 in STATE-OF-DOCS.md)
- Main content area

### `?format=md`

Returns the raw markdown source.

```
Content-Type: text/markdown; charset=utf-8
```

### `?format=json`

Returns structured page data:

```json
{
  "slug": "getting-started",
  "title": "Getting Started",
  "description": "...",
  "url": "/getting-started",
  "anchors": [
    { "level": 2, "text": "Prerequisites", "slug": "prerequisites" }
  ],
  "content_html": "<p>...</p>",
  "content_md": "# Getting Started\n..."
}
```

---

## `GET /llms.txt`

A plain-text page index following the emerging `llms.txt` convention for AI
consumption. Format:

```
# Site Title

Site description

## Pages

- [Page Title](url)
- [Another Page](url)
```

Pages are sorted by the same `order` → `title` → `slug` ordering as the sidebar nav.

---

## `GET /sitemap.xml`

Standard XML sitemap for search engines:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://docs.example.com/getting-started</loc></url>
  ...
</urlset>
```

URLs are built from `PageMeta::url()`, which applies `base_path` and any
`url_prefix` configured per corpus.

---

## `GET /healthz`

Returns `200 ok` (plain text). No authentication required. Use as a liveness probe.

---

## `GET /_events`

Server-Sent Events stream for watch-mode live reload.

- In **watch mode** (`enscrive-docs watch`): yields `event: reload` events when
  markdown files change.
- In **plain serve mode**: connection is held open with keep-alive pings; no events
  are ever emitted. The connection is effectively a no-op but doesn't error.

Browsers connect via the injected `EventSource` listener (present only in watch mode).
Calling `/_events` in plain serve mode is harmless.

---

## `GET /_assets/*path`

Serves assets embedded in the binary (via `rust-embed`). Assets are served from
the `assets/` directory at repository root.

| Asset path | MIME | Description |
|---|---|---|
| `themes/neutral/style.css` | `text/css` | Neutral theme |
| `themes/enscrive/style.css` | `text/css` | Brand theme |
| `js/search.js` | `application/javascript` | ⌘K search palette |

Cache headers: `public, max-age=3600`.

---

## Text Fragment URLs

Search results include URLs with [Text Fragment](https://wicg.github.io/scroll-to-text-fragment/)
suffixes (`#:~:text=...`) so Chromium-based browsers and Safari scroll directly to
the matching passage and highlight it in the accent color.

The fragment is extracted from the first 4–12 word phrase in the chunk content
(minimum 20 characters, max 80), with markdown sigils stripped. Spaces are
percent-encoded as `%20`; `&` and `#` are encoded to prevent fragment termination.

Firefox ignores Text Fragments and lands on the page root — this is a browser
limitation, not a bug in `enscrive-docs`.
