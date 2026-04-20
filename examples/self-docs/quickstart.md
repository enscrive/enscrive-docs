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

## 2. Scaffold

```bash
cd my-app
enscrive-docs init
```

This writes an `enscrive-docs.toml` next to your project. Open it and:

- set the `path` on `[[collections]]` to your markdown directory
- set `embedding_model` on `[[collections]]` (e.g. `"openai/text-embedding-3-small"`) so
  `bootstrap` can create the collection if it doesn't already exist
- leave the default `[[voices]]` block as-is for now — you can tune it later via
  `enscrive-docs voice tune`

A `score_threshold` of `0.0` surfaces all matches. You can raise it later once the
collection has enough content to filter against. See [Voices](/voices) for the full
configuration surface.

## 3. Bootstrap

```bash
enscrive-docs bootstrap
```

One command creates the voice, creates the collection, and runs the first ingest. It's
idempotent — re-running against an existing tenant skips any voice or collection that
already exists and just re-ingests. Pass `--skip-ingest` if you only want to provision
the voice + collection without pushing content yet.

Enscrive deduplicates by content fingerprint, so `enscrive-docs ingest` on unchanged
files is effectively free.

## 4. Serve

```bash
enscrive-docs serve
```

The site is now live at <http://localhost:3737/>. Press `⌘K` (or `Ctrl+K`) and search.
Click any result and the browser scrolls to and highlights the matching passage.

## Iterating

As your docs evolve:

- `enscrive-docs ingest` pushes new or changed files; unchanged files skip.
- `enscrive-docs voice tune <voice>` opens the voice config in `$EDITOR`, validates
  your edits, and `PUT`s the new version. Use this to dial in `score_threshold`,
  chunking parameters, or the template prompt once you see real search behavior.
- `enscrive-docs reset --yes` deletes and recreates the collection (then re-runs
  `bootstrap`). Reach for it only when the corpus has drifted enough that the cleanest
  rebuild beats incremental fixes.

## What's next

- [Configuration](/configuration) — every key in `enscrive-docs.toml`.
- [Themes](/themes) — neutral default, brand variant, custom CSS, full template
  override.
- [Watch mode](/serving) — auto-reload on file save.
- [Search](/search) — the CLI subcommand and the HTTP `/search` endpoint.
- [Troubleshooting](/troubleshooting) — common pitfalls and their fixes.
