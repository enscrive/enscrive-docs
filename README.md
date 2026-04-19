# enscrive-docs

> Turn any markdown directory into a retrieval-native documentation site backed
> by Enscrive — for humans, agents, and HTTP consumers.

`enscrive-docs` is a single Rust binary that ingests your markdown into
[Enscrive](https://enscrive.io) collections, configures voices for tuned
neural search, and serves the docs as polished HTML, a JSON search API,
an `/llms.txt` index, and a sitemap — all from one process, with zero
runtime dependencies beyond the public Enscrive API.

**Status:** pre-alpha, in active development. Not yet published to crates.io.

## Why

Coding agents in 2026 don't read your docs page-by-page — they neural-search
them. `enscrive-docs` makes that search a first-class capability of your
`/docs` endpoint, not an afterthought. Humans see a polished site; agents
query a JSON endpoint backed by Enscrive's neural search.

## Install (when shipped)

```bash
# Rust ecosystem
cargo install enscrive-docs

# Universal install script
curl -fsSL https://docs.enscrive.io/install | sh

# macOS / Linux Homebrew
brew install enscrive/tap/enscrive-docs

# Or download a pre-built binary from
# https://github.com/enscrive/enscrive-docs/releases
```

## Quickstart (planned)

```bash
enscrive-docs init                    # scaffold enscrive-docs.toml
enscrive-docs ingest                  # push your markdown to Enscrive
enscrive-docs serve --port 8080       # serve HTML + /search JSON + /llms.txt
```

## Repository layout

```
enscrive-docs/
├── crates/core/      # HTTP client, types, config (library)
├── crates/render/    # markdown -> HTML, theming (library)
└── crates/cli/       # the binary
```

The `core` and `render` crates can be embedded as libraries in other Rust
projects; the `cli` crate is what ships as the `enscrive-docs` binary.

## License

Apache 2.0 — see [LICENSE](./LICENSE).

The names and logos remain trademarks of Enscrive — see
[TRADEMARKS.md](./TRADEMARKS.md).

## Links

- Project home & docs: https://docs.enscrive.io (coming soon)
- Enscrive platform: https://enscrive.io
- Issues: https://github.com/enscrive/enscrive-docs/issues
