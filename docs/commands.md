# Command Reference

> All commands verified against `main` branch source code.  
> Status labels: **shipped** = present and functional on `main` · **partial** = present but incomplete · **deferred** = planned, not wired

## Global Flags

These flags apply to every subcommand:

| Flag | Env var | Config key | Default | Description |
|---|---|---|---|---|
| `--config` / `-c` | — | — | `./enscrive-docs.toml` | Path to config file |
| `--api-key` | `ENSCRIVE_API_KEY` | `[enscrive] api_key` | — | Enscrive API key |
| `--endpoint` | `ENSCRIVE_BASE_URL` | `[enscrive] endpoint` | `https://api.enscrive.io` | API base URL |
| `--embedding-provider-key` | `ENSCRIVE_EMBEDDING_PROVIDER_KEY` | `[enscrive] embedding_provider_key` | — | BYOK forwarded as `X-Embedding-Provider-Key` |
| `--profile` | `ENSCRIVE_PROFILE` | — | — | Named profile from `~/.config/enscrive/profiles.toml` |

---

## `enscrive-docs init`

**Status: shipped**

Scaffolds an `enscrive-docs.toml` in the current directory (or at `--config`).

```bash
enscrive-docs init
enscrive-docs init --theme enscrive      # scaffold with brand theme
enscrive-docs init --force               # overwrite existing config
enscrive-docs init --no-profile-detect   # skip ~/.config/enscrive/profiles.toml probe
```

**What it does:**

1. Checks for an existing `enscrive-docs.toml` (fails unless `--force`).
2. Probes `~/.config/enscrive/profiles.toml` for the first profile name (auto-fills
   `[enscrive] profile = ...` in the scaffold so credentials don't need to be
   duplicated for users who already have `enscrive-cli` configured).
3. Writes a complete commented TOML scaffold with one corpus (`guides`), one voice
   (`guides-voice`), and `[search]`, `[theme]`, `[serve]` defaults.

**Output:** `scaffolded ./enscrive-docs.toml` plus a next-steps prompt.

---

## `enscrive-docs bootstrap`

**Status: shipped**

Idempotently creates missing voices and corpora, then runs the first ingest.

```bash
enscrive-docs bootstrap
enscrive-docs bootstrap --skip-ingest    # provision only; skip first ingest
```

**What it does:**

1. Lists `GET /v1/voices` and creates any `[[voices]]` entries that are missing in
   the tenant (`POST /v1/voices`). Existing voices are skipped.
2. Lists `GET /v1/corpora` and creates any `[[corpora]]` entries that are missing
   (`POST /v1/corpora`, using `embedding_model` from config). Existing corpora are
   skipped. Fails with a clear error if `embedding_model` is absent for a corpus
   that doesn't exist yet.
3. Unless `--skip-ingest`, runs `ingest` (see below).

**Idempotency:** Safe to re-run against an existing tenant — every create is
guarded by a pre-existing-check against the live tenant state.

---

## `enscrive-docs ingest`

**Status: shipped (with caveats — see F-02)**

Walks configured corpus directories and pushes documents to Enscrive.

```bash
enscrive-docs ingest
enscrive-docs ingest --corpus guides          # ingest one corpus only
enscrive-docs ingest --dry-run                # walk files; do not POST
enscrive-docs ingest --force                  # ⚠️ declared but NOT IMPLEMENTED (F-02)
```

**What it does:**

1. Loads config and (unless `--dry-run`) fetches corpus IDs and voice IDs from the
   tenant.
2. For each configured corpus, walks the `path` directory using `walkdir`, filtering
   by the glob extension (e.g. `**/*.md` → only `.md` files).
3. Per file: reads content, computes a SHA-256 fingerprint, constructs an
   `IngestDocument` with `id = relative/path/from/root.md`.
4. POSTs `IngestRequest` to `/v1/ingest` with:
   - `corpus_id`, `voice_id` (resolved by name)
   - `documents[]` — content + fingerprint
   - `sync: true` — waits for chunking + embedding
5. Prints per-corpus success/failure counts.

**Fingerprint behavior:** The SHA-256 fingerprint is sent per document. Enscrive
uses it server-side to skip re-embedding unchanged documents. You pay embedding
cost only for new or modified files.

**`--dry-run`:** Does not contact the Enscrive API. Walks the file tree and prints
what would be ingested (paths, byte sizes, fingerprint prefixes). Useful for
verifying corpus path configuration offline.

**⚠️ Finding F-02 — `--force` is a no-op:** The flag is parsed by clap but
`args.force` is never read in `ingest::run()`. There is no client-side mechanism to
bypass the fingerprint; the server always controls deduplication.

**Glob support note:** `derive_extension_from_glob()` extracts only the last file
extension from the configured glob. `**/*.md` → `.md` filter works correctly.
Complex glob patterns (`*.{md,mdx}`, `**/*`) fall back to walking all files.

---

## `enscrive-docs serve`

**Status: shipped**

Starts the HTTP documentation server.

```bash
enscrive-docs serve
enscrive-docs serve --port 8080
enscrive-docs serve --bind 0.0.0.0 --port 80
enscrive-docs serve --base-path /docs       # reverse-proxy subpath
```

**What it does:**

1. Loads config, resolves API key + endpoint.
2. Fetches corpus IDs and voice IDs. If the Enscrive upstream is unreachable at
   startup, logs a warning and starts in **docs-only mode** (markdown is served,
   `/search` fails at request time).
3. Walks all corpus markdown paths, renders each `.md` file to HTML via the
   `pulldown-cmark` pipeline, and stores pages in an `ArcSwap<HashMap>` in-memory
   cache.
4. Binds the Axum server and serves until `SIGTERM`/`SIGINT`.

**Port resolution order:** `--port` > `$PORT` > `[serve] port` > `3737` (built-in).

**Routes:** see [api-surface.md](api-surface.md) for the full route table.

---

## `enscrive-docs watch`

**Status: shipped**

`serve` with file watching and browser live reload.

```bash
enscrive-docs watch
enscrive-docs watch --debounce-ms 500       # coarser debounce
```

**Additional behavior over `serve`:**

- Registers a `notify` watcher on every configured corpus `path`.
- On `.md` file change: debounces 250 ms (configurable), rebuilds the in-memory page
  cache, then broadcasts a `reload` event via SSE to all connected browsers.
- Injects an `EventSource("/_events")` listener into every rendered page that calls
  `window.location.reload()` on `reload` events.
- Editor temp/swap files are filtered: Vim `.swp`, Emacs `#foo#`, trailing `~`,
  JetBrains `___jb_*`.

**Note:** watch-mode cache rebuild is in-process and instant. Enscrive ingest is
**not** triggered automatically on file save — that remains a deliberate `ingest`
invocation so you don't pay embedding costs on every keystroke.

---

## `enscrive-docs search`

**Status: shipped**

One-shot neural search against configured corpora. Useful without a running server.

```bash
enscrive-docs search "trademark policy"
enscrive-docs search "getting started" --corpus guides --limit 5
enscrive-docs search "configuration" --format json | jq
enscrive-docs search "serving behind proxy" --format md > results.md
```

| Flag | Default | Description |
|---|---|---|
| `--corpus` | first configured corpus | Limit to one corpus by name |
| `--voice` | corpus's configured voice, then `[search] default_voice` | Voice to use for ranking |
| `--limit` | `10` | Max results |
| `--format` | `human` | `human` / `json` / `md` |

**Search path:** Uses `POST /v1/voices/search` when a voice is resolved (default),
falling back to `POST /v1/search` otherwise. Same logic as the HTTP server.

**Output formats:**

| Format | Description |
|---|---|
| `human` | Indented per-result block: score, document ID, snippet (240 chars) |
| `json` | Structured envelope: `query`, `search_time_ms`, `total_candidates`, `results[]` |
| `md` | Markdown sections with blockquoted snippets — paste into chat or issues |

---

## `enscrive-docs voice tune`

**Status: shipped**

Edit a voice's configuration in `$EDITOR` (default: `vi`) and PUT it back.

```bash
enscrive-docs voice tune guides-voice
EDITOR=code enscrive-docs voice tune guides-voice   # use VS Code
```

**What it does:**

1. Fetches the live voice config from `GET /v1/voices/{id}`.
2. Serializes it to TOML with a header comment block.
3. Opens a temp file (`/tmp/enscrive-docs-voice-{name}-{pid}.toml`) in `$EDITOR`.
4. On editor exit: if content is unchanged, aborts. Otherwise parses the edited TOML
   back into `VoiceConfigApi` and sends it via `PUT /v1/voices/{id}`.
5. Prints the old and new version numbers.

**The `voice tune` loop is the primary eval-tuning mechanism** until `enscrive-docs
eval` ships (planned, not yet implemented).

---

## `enscrive-docs reset`

**Status: shipped**

Deletes configured corpora and recreates them via bootstrap. **Destructive** — all
embeddings are lost.

```bash
enscrive-docs reset --yes                    # reset all configured corpora
enscrive-docs reset --yes --corpus guides    # reset one corpus
enscrive-docs reset --yes --skip-ingest      # recreate corpus; skip re-ingest
```

`--yes` is required. Without it the command fails with an error explaining what would
happen.

**What it does:**

1. Lists `/v1/corpora` and deletes every configured corpus that exists
   (`DELETE /v1/corpora/{id}`).
2. Runs `bootstrap` (which creates voices + corpora, then re-ingests unless
   `--skip-ingest`).

**Note:** The `/v1` API has no bulk "delete all documents" endpoint. Reset is the
only path to a clean-slate rebuild.

---

## `enscrive-docs config`

**Status: shipped**

Prints or validates the resolved configuration.

```bash
enscrive-docs config                         # print resolved TOML
enscrive-docs config --validate              # validate only; exit 0 if ok
```

Useful for debugging credential resolution and verifying path settings before
running ingest or serve. Output includes the file the config was loaded from.
**Redact the API key before sharing** — the command prints secrets in plaintext.

---

## Annotated First-Run Walkthrough

```bash
# 1. Scaffold config in your docs directory
cd my-project
enscrive-docs init

# Edit enscrive-docs.toml:
#   - set [[corpora]] path = "./docs"
#   - set [[corpora]] embedding_model = "openai/text-embedding-3-small"
#   - set ENSCRIVE_API_KEY in environment or [enscrive] api_key

# 2. Verify the config parses correctly
enscrive-docs config --validate

# 3. Create voices + corpus + push first batch of docs
enscrive-docs bootstrap
# Output:
#   bootstrap: reconciling voices
#     [create] voice "guides-voice" -> vce_abc123
#   bootstrap: reconciling corpora
#     [create] corpus "guides" -> cps_xyz789 (model: openai/text-embedding-3-small)
#   bootstrap: first ingest
#   [guides] 8 document(s) -> corpus cps_xyz789 (voice: guides-voice)
#     ingested: 8 ok / 0 failed (8 events total)
#   bootstrap: done.

# 4. Start the server
enscrive-docs serve
# Output: listening on http://127.0.0.1:3737/

# 5. Search from CLI
enscrive-docs search "getting started"

# 6. During authoring: live reload
enscrive-docs watch

# 7. After editing docs, re-push changed files
enscrive-docs ingest          # only changed files pay embedding cost

# 8. Tune retrieval quality
enscrive-docs voice tune guides-voice   # opens in $EDITOR
```
