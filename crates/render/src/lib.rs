//! enscrive-docs-render
//!
//! Markdown-to-HTML rendering, Askama templates, and theming for enscrive-docs.
//! Designed to be embedded as a library by other Rust apps that want to render
//! markdown docs without the full CLI binary.

pub mod markdown;
pub mod theme;
