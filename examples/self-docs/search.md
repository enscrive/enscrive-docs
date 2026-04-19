---
title: Search
description: The CLI subcommand, the HTTP endpoint, and how scroll-to-passage works.
order: 60
---

# Search

`enscrive-docs` exposes neural search through three surfaces, each appropriate for a
different consumer.

## The browser palette

When you load a page served by `enscrive-docs`, press `⌘K` (or `Ctrl+K`) to open the
search overlay. The implementation is ~150 lines of vanilla JavaScript baked into the
binary — no framework, no build step.

- The input is debounced 150 ms to avoid round-tripping per keystroke.
- Arrow keys navigate results, `Enter` opens the highlighted result, `Escape` closes.
- Click or `Enter` navigates to the result URL with a Text Fragments suffix
  (`#:~:text=...`) so the browser scrolls directly to the matching passage and
  highlights it in the accent color.

If a result fails to scroll, that almost always means the chunk content straddled a
code block — the browser's text matcher needs the fragment text to appear contiguously
in visible DOM. Switching the voice to `paragraphs` chunking usually fixes this.

## The HTTP endpoint

```
GET /search?q=...&voice=...&collection=...&limit=10
```

Returns:

```json
{
  "query": "trademark",
  "search_time_ms": 215,
  "total_candidates": 3,
  "results": [
    {
      "document_id": "trademarks.md",
      "score": 0.368,
      "snippet": "# Trademark Policy The names Enscrive, enscrive-docs, ...",
      "url": "/trademarks#:~:text=Trademark%20Policy%20The%20names%20Enscrive",
      "title": "Trademarks",
      "collection_id": "..."
    }
  ]
}
```

Every result includes a `url` with the Text Fragments suffix already constructed, so
agents can both display and click-through to the matching passage without doing any URL
synthesis themselves.

CORS is `Access-Control-Allow-Origin: *` so browser apps and agents can call
`/search` from any origin.

## The CLI subcommand

For agents that prefer to shell out (no running `serve` required) or for one-off
queries from the terminal:

```bash
enscrive-docs search "trademark"
enscrive-docs search "agent voice" --collection guides --limit 5
enscrive-docs search "how to install" --format json | jq
enscrive-docs search "configuration" --format md > results.md
```

Output formats:

| Format | Shape |
|---|---|
| `human` (default) | Indented per-result block with score, document, snippet |
| `json` | Structured envelope, full content + snippet, scores |
| `md` | Markdown sections with blockquoted snippets — paste-ready into chat or issues |

The CLI uses the same scope-resolution logic as the HTTP endpoint: defaults to the
first configured collection, defaults the voice to that collection's configured voice.

## How scoring works

Scores are cosine similarity between the query embedding and each chunk's embedding,
returned by the Enscrive API. Higher is more similar.

- `0.5+` is a strong match. With a small collection, scores at this level are rare.
- `0.2–0.5` is a typical "this document discusses your topic" match.
- `0.0–0.2` is a weak match — the topic is mentioned but not central.

Do not aggressively raise `score_threshold` until your collection is large enough that
the noise floor is meaningful. See [Voices](/voices) for tuning guidance.

## Voice-tuned vs. plain search

When the resolved request has a voice (the default), `enscrive-docs` calls Enscrive's
`/v1/voices/search` endpoint. This applies the voice's `chunking_strategy`,
`score_threshold`, `default_limit`, and any `parameters` to the retrieval. Plain
`/v1/search` is the fallback for requests with no voice resolvable.

This is the differentiated capability — voices are how Enscrive lets you tune
retrieval without re-ingesting. Use them.
