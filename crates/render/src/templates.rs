//! Askama template wiring. The `.html` template files live in
//! `crates/render/templates/` and are compiled into this crate at build
//! time, so the binary ships with no template files on disk.

use crate::page::PageMeta;
use askama::Template;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct NavItem {
    pub title: String,
    pub url: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageContext {
    pub site_title: String,
    pub site_description: String,
    pub base_path: String,
    pub asset_base: String,
    pub theme_css_path: String,
    pub theme_variables: String,
    pub custom_css: String,
    pub page_title: String,
    pub page_description: String,
    pub page_html: String,
    pub page_anchors_html: String,
    pub nav: Vec<NavItem>,
    /// Inject the dev-mode SSE reload listener. Only set true under
    /// `enscrive-docs watch`; production `serve` leaves this false.
    pub watch_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexContext {
    pub site_title: String,
    pub site_description: String,
    pub base_path: String,
    pub asset_base: String,
    pub theme_css_path: String,
    pub theme_variables: String,
    pub custom_css: String,
    pub nav: Vec<NavItem>,
    pub watch_mode: bool,
}

#[derive(Template)]
#[template(path = "page.html", escape = "none")]
struct PageTemplate<'a> {
    ctx: &'a PageContext,
}

#[derive(Template)]
#[template(path = "index.html", escape = "none")]
struct IndexTemplate<'a> {
    ctx: &'a IndexContext,
}

pub fn render_page(ctx: &PageContext) -> Result<String, askama::Error> {
    PageTemplate { ctx }.render()
}

pub fn render_index(ctx: &IndexContext) -> Result<String, askama::Error> {
    IndexTemplate { ctx }.render()
}

/// Build a nav-sidebar entry list from page metadata. Sorted by frontmatter
/// `order` (ascending) then by title; ties broken by slug.
pub fn build_nav(pages: &[PageMeta], base_path: &str, current_slug: Option<&str>) -> Vec<NavItem> {
    let mut sorted: Vec<&PageMeta> = pages.iter().collect();
    sorted.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    sorted
        .into_iter()
        .map(|m| NavItem {
            title: m.title.clone(),
            url: m.url(base_path),
            current: current_slug == Some(m.slug.as_str()),
        })
        .collect()
}

/// Render a simple table-of-contents <ul> for h2/h3 anchors.
pub fn render_anchor_list(anchors: &[crate::markdown::HeadingAnchor]) -> String {
    if anchors.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"toc\">");
    for a in anchors {
        out.push_str(&format!(
            "<li class=\"toc-l{lvl}\"><a href=\"#{slug}\">{text}</a></li>",
            lvl = a.level,
            slug = ammonia::clean(&a.slug),
            text = ammonia::clean(&a.text),
        ));
    }
    out.push_str("</ul>");
    out
}
