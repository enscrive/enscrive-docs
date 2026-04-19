---
title: Quickstart
description: From zero to a searchable docs site in 90 seconds.
order: 10
---

# Quickstart

This walkthrough takes you from a clean directory to a running docs site with neural
search in three commands.

## Prerequisites

- An [Enscrive](https://enscrive.io) account with an API key.
- Rust 1.85+ if you are installing from crates.io. (`rustc --version` to check.)
- A directory of markdown files. If you don't have one yet, your project's `README.md`
  is a fine starting point.

## 1. Install

```bash
cargo install enscrive-docs
```

Alternatives — `curl https://docs.enscrive.io/install | sh`, `brew install
enscrive/tap/enscrive-docs`, or download a pre-built binary from
[github.com/enscrive/enscrive-docs/releases](https://github.com/enscrive/enscrive-docs/releases).

## 2. Create a collection and a voice

In your Enscrive tenant, create one collection that will hold the docs and one voice that
controls how docs are chunked and ranked:

```bash
enscrive collections create \
  --name my-app-docs \
  --embedding-model text-embedding-3-small

enscrive voices create \
  --name my-app-voice \
  --config-json '{"chunking_strategy":"baseline","score_threshold":0.0,"default_limit":10}'
```

A `score_threshold` of `0.0` surfaces all matches. You can raise it later once the
collection has enough content to filter against. See [Voices](/voices) for the full
configuration surface.

## 3. Scaffold and ingest

```bash
cd my-app
enscrive-docs init
```

This writes an `enscrive-docs.toml` next to your project. Edit the `[[collections]]` and
`[[voices]]` blocks so the names match what you just created in step 2 and the `path`
points at your markdown directory. Then:

```bash
enscrive-docs ingest
```

The CLI walks your markdown, fingerprints each file, and pushes the documents into the
collection. Re-runs are idempotent — Enscrive deduplicates by content fingerprint so
unchanged files are skipped.

## 4. Serve

```bash
enscrive-docs serve
```

The site is now live at <http://localhost:3737/>. Press `⌘K` (or `Ctrl+K`) and search.
Click any result and the browser scrolls to and highlights the matching passage.

## What's next

- [Configuration](/configuration) — every key in `enscrive-docs.toml`.
- [Themes](/themes) — neutral default, brand variant, custom CSS, full template
  override.
- [Watch mode](/serving) — auto-reload on file save.
- [Search](/search) — the CLI subcommand and the HTTP `/search` endpoint.
- [Troubleshooting](/troubleshooting) — common pitfalls and their fixes.
