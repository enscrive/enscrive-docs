---
title: Troubleshooting
description: Common failures and what to check first.
order: 90
---

# Troubleshooting

## "no API key found"

You have an `enscrive-docs.toml` but the CLI cannot resolve credentials. Check, in
order:

1. Is `ENSCRIVE_API_KEY` set in the current shell? `echo $ENSCRIVE_API_KEY` should
   print a non-empty value.
2. If you are using a profile, does `~/.config/enscrive/profiles.toml` contain
   `[profiles.<name>]` with `api_key = "..."`? Profile names are case-sensitive.
3. If you set `[enscrive] api_key = "..."` directly in `enscrive-docs.toml`, is the file
   actually being read? Run `enscrive-docs config` to see the resolved configuration.

## "Enscrive collection \"foo\" not found in tenant"

`enscrive-docs ingest` and `serve` verify on startup that every configured collection
exists. The message means your `enscrive-docs.toml` references a collection name that is
not in the tenant the API key authenticates against.

Either rename the entry in `[[collections]]` to match an existing collection, or create
it first:

```bash
enscrive collections create --name foo --embedding-model text-embedding-3-small
```

Same applies to voices.

## Search returns zero results for everything

The most common cause is a `score_threshold` set higher than your collection's actual
score range. Documents shorter than a few paragraphs and queries shorter than a phrase
genuinely produce semantic scores in the 0.1–0.4 range. A threshold of `0.5` will
filter all of them out.

Drop your voice's `score_threshold` to `0.0`:

```bash
enscrive voices update <voice-id> --config-file - <<EOF
{"chunking_strategy":"baseline","score_threshold":0.0,"default_limit":10}
EOF
```

Then re-run a search. If you now see results with low scores, that's the actual
relevance — your collection might just be small. Add more docs.

## Search fails with `upstream_search_failed` (HTTP 502)

The `/search` endpoint forwards Enscrive errors verbatim in `data.detail`. The two
common causes:

- **Collection not specified, multi-collection tenant**: the upstream search RPC errors
  out when no collection filter is sent against a tenant with many collections. The
  serve handler defaults to the first configured collection — if you see this, your
  config has zero `[[collections]]` entries.
- **Voice not in tenant**: the voice id the request resolved to is not present. Run
  `enscrive voices list` and confirm.

## Watch mode does not auto-refresh the browser

1. Confirm you started `enscrive-docs watch` (not `serve`). `serve` does not inject the
   SSE listener.
2. Open DevTools → Network → `_events`. You should see a long-polling `text/event-stream`
   connection with no body. If it is missing, the page was rendered by `serve` (no
   listener injected).
3. Edit a `.md` file inside a configured collection `path`. Files outside the configured
   paths are not watched.
4. Editor temp files (`.foo.swp`, `#foo#`, `~`, JetBrains `___jb_*`) are filtered out
   on purpose. The actual `.md` save event is what triggers the reload.

## Cargo install fails: "package `enscrive-docs` not found"

Pre-alpha — the crate is not yet on crates.io. Install from source:

```bash
cargo install --git https://github.com/enscrive/enscrive-docs --bin enscrive-docs
```

This is documented on [docs.enscrive.io](https://docs.enscrive.io) and will be
unnecessary once the v1 release ships to crates.io.

## "Failed to bind: Address already in use"

Another process is already using port 3737. Either stop it (`lsof -i :3737`,
`kill <pid>`) or pick a different port:

```bash
enscrive-docs serve --port 3738
```

## Where to get help

- File an issue at
  [github.com/enscrive/enscrive-docs/issues](https://github.com/enscrive/enscrive-docs/issues).
- Include the output of `enscrive-docs config` (with the API key redacted) and the
  exact command you ran.
