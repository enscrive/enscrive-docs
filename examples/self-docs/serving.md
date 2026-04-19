---
title: Serving
description: serve, watch, and the three deployment shapes.
order: 50
---

# Serving

`enscrive-docs` ships two server subcommands: `serve` for production and `watch` for
local authoring.

## `enscrive-docs serve`

```bash
enscrive-docs serve
enscrive-docs serve --port 8080
enscrive-docs serve --bind 0.0.0.0 --port 80
enscrive-docs serve --base-path /docs
```

On startup the binary:

1. Loads `enscrive-docs.toml` and resolves the API key + endpoint.
2. Lists collections and voices from your Enscrive tenant and verifies that every
   `[[collections]]` and `[[voices]]` entry resolves to a real ID.
3. Walks each collection's `path` for matching markdown files and renders them into an
   in-memory cache.
4. Binds the listener and serves.

Pages are rendered once; subsequent requests are served from the cache. The
`/search` endpoint round-trips Enscrive on every request (you cannot pre-cache search
results without sacrificing freshness).

### Port resolution

The default port is **3737** — chosen to dodge the crowded 8080 / 3000 / 8000 cluster
that fights with Tomcat, Node, Python, MAMP, and a dozen other dev tools. Resolution
order, highest to lowest:

1. `--port 8080` flag
2. `PORT` environment variable (so you can deploy unmodified to Heroku, Railway, Fly,
   Cloud Run, Amplify Hosting compute, etc.)
3. `[serve] port = ...` in `enscrive-docs.toml`
4. Built-in default `3737`

### Routes

| Route | Purpose |
|---|---|
| `GET /` | Index page (hero + page list) |
| `GET /{slug}` | Rendered HTML page |
| `GET /{slug}?format=md` | Raw markdown (handy for agents) |
| `GET /{slug}?format=json` | `{title, slug, content_html, content_md, anchors[]}` |
| `GET /search?q=...&voice=...&collection=...&limit=10` | JSON search results |
| `GET /llms.txt` | LLM-friendly index of all pages |
| `GET /sitemap.xml` | Standard XML sitemap |
| `GET /healthz` | Liveness check (`200 ok`) |
| `GET /_assets/*` | Static assets served from the embedded bundle |

## `enscrive-docs watch`

```bash
enscrive-docs watch
enscrive-docs watch --debounce-ms 250
```

Same as `serve` but with a `notify`-based filesystem watcher and an SSE `/_events`
endpoint. On every markdown change inside a configured collection's `path`:

1. The change is debounced (default 250 ms) to coalesce editor save bursts.
2. The page cache is rebuilt in memory.
3. Connected browsers receive an SSE `reload` event and refresh.

The HTML cache reload is in-process and instant. The Enscrive ingest is **not**
automatically re-run on file change — that stays a deliberate `enscrive-docs ingest`
invocation so you do not pay surprise embedding costs while iterating on copy. When you
are happy with the prose, run `ingest` once to push.

Editor temp/swap files (`.foo.swp`, `#foo#`, `~`, JetBrains `___jb_*`) are filtered out
of the watcher so save-storm noise does not trigger spurious reloads.

## Deployment shapes

Three patterns work day-one:

### 1. Subdomain (recommended)

Run the binary on a dedicated host, point a DNS record at it.

```
docs.app.example.com  →  enscrive-docs serve --bind 0.0.0.0 --port 80
```

Cleanest separation, no reverse proxy required.

### 2. Subpath behind a reverse proxy

Mount the docs at `/docs/*` of your existing app.

```
app.example.com/docs/*  →  reverse-proxy  →  localhost:3737
```

Run `enscrive-docs serve --base-path /docs` so all internal URLs and asset paths render
with the prefix.

### 3. Sidecar in a container

Drop the binary into your existing container or compose stack alongside the main app.
A minimal Dockerfile:

```dockerfile
FROM rust:1.85-slim AS build
RUN cargo install enscrive-docs
FROM debian:stable-slim
COPY --from=build /usr/local/cargo/bin/enscrive-docs /usr/local/bin/
COPY enscrive-docs.toml docs/ /app/
WORKDIR /app
CMD ["enscrive-docs", "serve", "--bind", "0.0.0.0", "--port", "8080"]
```

Or pull a pre-built binary from
[GitHub Releases](https://github.com/enscrive/enscrive-docs/releases) into a `scratch`
or `alpine` image — the binary is statically linked and has no system dependencies
beyond `libc`.
