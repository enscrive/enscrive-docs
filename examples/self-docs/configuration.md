---
title: Configuration
description: Every section of enscrive-docs.toml explained.
order: 20
---

# Configuration

`enscrive-docs.toml` lives in the directory you run the CLI from (or wherever you point
it with `--config`). Resolution precedence for every setting, highest to lowest:

1. CLI flag (e.g. `--port 8080`)
2. Environment variable (e.g. `PORT`, `ENSCRIVE_API_KEY`)
3. Inline value in `enscrive-docs.toml`
4. Profile file at `~/.config/enscrive/profiles.toml` (for credentials only)
5. Built-in default

## `[enscrive]`

Credentials and the API endpoint.

```toml
[enscrive]
profile = "default"                 # reads ~/.config/enscrive/profiles.toml
# api_key = "..."                   # or set ENSCRIVE_API_KEY in the environment
# endpoint = "https://api.enscrive.io"
# embedding_provider_key = "..."    # optional BYOK forwarded as X-Embedding-Provider-Key
```

If you already use [`enscrive-cli`](https://github.com/enscrive/enscrive-cli), reference
its profile name here and `enscrive-docs` will pick up both the API key and the endpoint
automatically. This is the preferred pattern for local development.

## `[site]`

Static site metadata.

```toml
[site]
title = "My App Docs"
description = "Documentation for My App."
base_url = "https://app.example.com/docs"
base_path = "/docs"                # set when serving behind a reverse-proxy subpath
default_version = "latest"         # for multi-version docs (deferred)
```

`base_path` matters when you mount the docs server under a subpath of an existing app
(e.g. `app.example.com/docs/*` proxied to `localhost:3737`). All internal URLs and asset
paths render with the prefix.

## `[theme]`

Layered theming. See [Themes](/themes) for the full layering model.

```toml
[theme]
variant = "neutral"                # "neutral" (default) or "enscrive"
accent_color = "#0ea5e9"
logo_path = "./assets/logo.svg"
custom_css = "./custom.css"
template_dir = "./templates"
```

## `[[collections]]`

Each entry maps a directory of markdown files to an Enscrive collection. The collection
must already exist in your tenant — `enscrive-docs ingest` verifies this on startup and
errors helpfully if it doesn't.

```toml
[[collections]]
name = "guides"
voice = "guides-voice"
path = "./docs/guides"
glob = "**/*.md"
url_prefix = "/guides"             # /docs/guides/getting-started ...
```

You can configure multiple collections — for example, one for tutorials and one for API
reference, each with a different voice tuned for its content type.

## `[[voices]]`

Voices control how documents are chunked at ingest time and how search is ranked at
query time. Each entry must already exist in your Enscrive tenant.

```toml
[[voices]]
name = "guides-voice"
chunking_strategy = "baseline"
parameters = { min_tokens = "256", max_tokens = "512" }
score_threshold = 0.0
default_limit = 10
description = "Default voice for guide content"
```

A `score_threshold` of `0.0` surfaces all matches. Raise it once you have enough content
to filter aggressively — see [Voices](/voices) for tuning guidance.

## `[search]`

Defaults applied to the `/search` HTTP endpoint and the `enscrive-docs search` CLI.

```toml
[search]
default_voice = "guides-voice"
results_per_page = 10
include_snippets = true
```

When the user does not specify `?voice=` or `?collection=` in a search request, these
defaults plus the first configured collection are used as fallbacks.

## `[serve]`

Server-side options for `enscrive-docs serve` and `watch`.

```toml
[serve]
port = 3737                        # CLI --port and $PORT env override this
```

## `[[versions]]` (deferred)

Multi-version docs. Each version maps to a separate collection.

```toml
[[versions]]
slug = "v1"
collections = ["guides-v1", "api-reference-v1"]

[[versions]]
slug = "v2"
collections = ["guides", "api-reference"]
default = true
```

The version surface is in the v1 schema but not yet wired into the serve handlers — it
will land in a later release.
