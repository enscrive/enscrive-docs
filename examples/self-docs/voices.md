---
title: Voices
description: Tuning chunking strategies and ranking behavior per collection.
order: 40
---

# Voices

A *voice* in Enscrive is a named bundle of retrieval-tuning parameters: how documents
are chunked at ingest, what threshold the search applies, what default limit it returns.
`enscrive-docs` references voices by name in the config and resolves them against your
Enscrive tenant on every operation.

If you take one thing from this page: voices are how you make search good. The CLI
configuration is mostly plumbing — the voice is the lever.

## The relationship between voices and collections

A voice is independent of a collection — multiple collections can share one voice, and
one collection can be searched with different voices. In `enscrive-docs.toml` you
associate one voice with each collection (the "default voice for this collection's
content"); the search subcommand and `/search` endpoint pick that voice automatically
unless you override.

```toml
[[collections]]
name = "guides"
voice = "guides-voice"             # default voice for searches against this collection
path = "./docs/guides"

[[voices]]
name = "guides-voice"
chunking_strategy = "baseline"
score_threshold = 0.0
default_limit = 10
```

## `chunking_strategy`

Controls how each markdown document is split into chunks at ingest time. The chunk is
the unit of embedding and the unit of retrieval.

| Strategy | When to pick it |
|---|---|
| `baseline` | Token-bounded paragraphs. The right default for most prose docs. |
| `paragraphs` | Strict paragraph splitting (one chunk per paragraph). Good for concept-dense docs. |
| `tone_segments` | Segments by detected tone shifts. Good for mixed prose + code + headings. |
| `llm_chunking` | LLM-driven semantic splits. More expensive at ingest, often higher recall. |

For new docs sites, start with `baseline` and only revisit if search relevance feels
off. The other strategies have stronger opinions; you want to feel the limits of the
default before reaching for them.

## `parameters`

Strategy-specific knobs, expressed as a map of strings.

```toml
[[voices]]
name = "guides-voice"
chunking_strategy = "baseline"
parameters = {
  min_tokens = "256",              # don't emit chunks below this size
  max_tokens = "512",              # split anything larger
}
```

Larger chunks preserve more context per result; smaller chunks improve precision (you
land closer to the matching passage). 256–512 tokens is a sensible default for prose
documentation.

## `score_threshold`

Minimum semantic similarity (0.0 to 1.0) required for a result to appear. The most
important knob and the most common source of "search returns nothing" frustration.

```toml
[[voices]]
score_threshold = 0.0              # surface all matches with their natural scores
```

**Start with `0.0`.** Raise the threshold only when:

1. Your collection is large enough that low-relevance noise is a real problem.
2. You have measured what threshold value corresponds to the floor of "useful" results.

A threshold of `0.5` against a collection of fewer than ~50 documents will routinely
return zero results because semantic scores for short documents and short queries
genuinely live in the 0.1–0.4 range. The fix is data, not a higher threshold.

## `default_limit`

Maximum results the search returns when the request does not specify `?limit=`.

```toml
default_limit = 10                 # /search returns up to 10 results
```

`enscrive-docs search "query"` and `/search?q=query` both honor this default; both also
accept `--limit N` and `?limit=N` as overrides.

## `description` and `tags`

Free-form metadata that surfaces in the Enscrive web UI. Useful when you have several
voices and want to remind future-you why each one exists.

```toml
description = "Tuned for long-form guide prose with code blocks"
tags = ["docs", "guides"]
```

## Iterating on voice tuning

The fastest loop:

1. Edit `enscrive-docs.toml`.
2. Re-create the voice via `enscrive voices update --config-file new-config.json`
   (Enscrive's CLI accepts a JSON file containing the voice config).
3. Run `enscrive-docs search "your query" --format json` to see the new ranking.
4. Repeat.

A future `enscrive-docs eval` subcommand will run Enscrive's own eval campaigns against
your voice and surface precision/recall metrics directly. That is the proper way to
tune; for now, the manual loop above is the fastest feedback path.
