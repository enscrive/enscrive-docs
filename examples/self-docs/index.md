---
title: enscrive-docs
description: Retrieval-native documentation backed by Enscrive neural search.
order: 0
---

# enscrive-docs

`enscrive-docs` is a single Rust binary that turns any markdown directory into a polished
documentation site backed by [Enscrive](https://enscrive.io) collections, voices, and neural
search. It is built for an audience of humans, AI agents, and any HTTP consumer in equal
measure.

This site is itself served by `enscrive-docs serve` against an Enscrive collection
containing the markdown you are reading right now. Press `⌘K` (or `Ctrl+K`) and search
for anything — the results are scored by an embedding model, not a keyword matcher.

## What it is

A single binary that:

- Walks a configured directory of markdown files.
- Pushes each file into an Enscrive collection, chunked according to a configured voice.
- Serves the rendered HTML, a JSON `/search` endpoint, an `/llms.txt` index, and a
  `sitemap.xml` from one process with no external runtime dependencies.

## What it isn't

- A general-purpose static-site generator. Hugo, Astro, and Mintlify are excellent
  at that. enscrive-docs is the right call when you want neural search on your `/docs`
  endpoint as a first-class capability, not a bolt-on widget.
- A managed SaaS. The tool is BYO-hosting; you run the binary anywhere that can speak
  HTTPS.
- A WYSIWYG editor. Markdown files on disk are the source of truth.

## Get started

The fastest path is the [Quickstart](/quickstart). For deeper topics, jump straight to
[Configuration](/configuration), [Voices](/voices), [Themes](/themes), or
[Serving](/serving). The whole site is searchable — try the palette before reading
linearly.

## Where to find help

- Source and issues: [github.com/enscrive/enscrive-docs](https://github.com/enscrive/enscrive-docs)
- Enscrive platform: [enscrive.io](https://enscrive.io)
- License: Apache 2.0. Trademark policy is in
  [TRADEMARKS.md](https://github.com/enscrive/enscrive-docs/blob/main/TRADEMARKS.md).
