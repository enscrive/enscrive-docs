---
title: Themes
description: Neutral default, brand variant, and the four customization layers.
order: 30
---

# Themes

`enscrive-docs` ships two built-in themes and four layered customization points so you
can override only what you need without forking the templates.

## The two built-in variants

### `neutral` (default)

A clean, brand-agnostic theme tuned to clear the 30-second human trust gate without any
configuration. Stripe-grade typography, system font stack (no Google Fonts), light + dark
mode driven by `prefers-color-scheme`, and a configurable accent color.

This is the right choice for most deployments: anything bolted into a customer's app
should look like that customer's product, not like Enscrive.

### `enscrive`

The brand variant. Slate-950 + sky-400 palette lifted from the Enscrive developer
portal, used for our own deployments at [docs.enscrive.io](https://docs.enscrive.io) and
[api.enscrive.io/docs](https://api.enscrive.io/docs). Useful as a reference if you are
embedding the tool inside an Enscrive-branded surface.

```toml
[theme]
variant = "enscrive"               # or "neutral" (the default)
```

## The four customization layers

Each layer composes on top of the previous one. Stop at the deepest layer you need.

### Layer 0 — defaults

Zero configuration. The neutral theme is baked into the binary and works out of the box.

### Layer 1 — token overrides

Override individual design tokens via `[theme]`:

```toml
[theme]
variant = "neutral"
accent_color = "#9333ea"           # purple instead of the default blue
logo_path = "./assets/my-logo.svg" # served at /_assets/logo.svg
```

This covers ~80% of branding adjustments without touching CSS.

### Layer 2 — custom CSS injection

```toml
[theme]
custom_css = "./custom.css"
```

Your stylesheet is appended after the embedded theme CSS, so any selector you write wins
over the defaults. Useful for tuning typography, spacing, and component-level styling
without rewriting the base theme.

### Layer 3 — template override

```toml
[theme]
template_dir = "./templates"
```

Drop your own `_base.html`, `page.html`, or `index.html` in that directory and they
replace the embedded templates. Use this only when token + CSS overrides aren't enough.

## On preserving search

The `⌘K` palette and the `/search` JSON endpoint are not part of the theme — they live in
the binary's vanilla JavaScript and HTTP routes. Even with a fully custom template, the
palette continues to work as long as your template includes the search trigger button
(`<button data-search-trigger>`) and an `<input id="ed-search-input">` inside an
`<div id="ed-search-overlay" hidden>`.

The script auto-wires those elements on load and posts to `{base_path}/search`.

## When to keep going

- See [Configuration](/configuration) for every `[theme]` key.
- See [Serving](/serving) for deployment patterns that interact with `--base-path`.
