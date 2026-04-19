# Self-docs

This directory holds the canonical documentation for `enscrive-docs` itself, written in
markdown. It is intended to be ingested into an Enscrive collection and served by
`enscrive-docs serve` running against that collection — i.e. the tool's own docs are
served by the tool itself.

The deployment at [docs.enscrive.io](https://docs.enscrive.io) currently runs the
[`docs-enscrive-io`](https://github.com/enscrive/docs-enscrive-io) Astro marketing site
at the index. Once the bootstrap is complete, the deeper paths
(`/quickstart`, `/configuration`, `/themes`, `/voices`, `/serving`, `/search`,
`/troubleshooting`) will be served by `enscrive-docs serve` against this directory's
content.

## Contents

| File | What it covers |
|---|---|
| `index.md` | Welcome, what enscrive-docs is and isn't, where to start |
| `quickstart.md` | Three-command path from zero to a running site |
| `configuration.md` | Every section of `enscrive-docs.toml` |
| `themes.md` | Neutral default, brand variant, four customization layers |
| `voices.md` | Chunking strategies, score thresholds, tuning guidance |
| `serving.md` | `serve`, `watch`, port resolution, deployment shapes |
| `search.md` | Browser palette, `/search` JSON, CLI subcommand, scoring |
| `troubleshooting.md` | Common failures and what to check first |

## Local preview

To render this collection through the tool, point an `enscrive-docs.toml` at this
directory:

```toml
[enscrive]
profile = "default"

[site]
title = "enscrive-docs"
description = "Retrieval-native documentation backed by Enscrive neural search."

[[collections]]
name = "enscrive-docs-self"
voice = "docs-default"
path = "."
glob = "**/*.md"

[[voices]]
name = "docs-default"
chunking_strategy = "baseline"
score_threshold = 0.0
default_limit = 10
```

Then `enscrive-docs ingest && enscrive-docs serve`.
