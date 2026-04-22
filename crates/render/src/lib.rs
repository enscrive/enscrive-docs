//! Markdown rendering, theming, and embedded asset access for enscrive-docs.
//!
//! This crate is consumed by the CLI binary's `serve` subcommand and can be
//! embedded directly by other Rust applications that want to render markdown
//! docs without the CLI.

pub mod assets;
pub mod markdown;
pub mod page;
pub mod templates;
pub mod theme;

pub use assets::{embedded_asset, EmbeddedAssets};
pub use markdown::{render_markdown, HeadingAnchor, RenderedMarkdown};
pub use page::{Page, PageMeta};
pub use templates::{
    build_nav, render_anchor_list, render_index, render_page, IndexContext, NavItem, PageContext,
    ReturnLink,
};
pub use theme::{Theme, ThemeVariant};
