//! Markdown -> HTML pipeline. Mirrors the pattern used by the existing
//! Enscrive developer-portal docs renderer (pulldown-cmark + ammonia).

use gray_matter::engine::YAML;
use gray_matter::Matter;
use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};

/// Output of rendering a single markdown document.
pub struct RenderedMarkdown {
    pub html: String,
    pub frontmatter: Frontmatter,
    pub anchors: Vec<HeadingAnchor>,
    /// Plain text of the document's first `# heading` if present. Stripped
    /// from `html` so callers can render the title once (template-driven)
    /// without it being duplicated by the markdown body.
    pub leading_h1: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub draft: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeadingAnchor {
    pub level: u32,
    pub text: String,
    pub slug: String,
}

/// Parse the frontmatter (if any), render the markdown body to sanitized
/// HTML, and collect anchors for h2/h3 headings (used to build a TOC).
pub fn render_markdown(raw: &str) -> RenderedMarkdown {
    let matter = Matter::<YAML>::new();
    let parsed = matter.parse(raw);
    let frontmatter: Frontmatter = parsed
        .data
        .as_ref()
        .and_then(|d| d.deserialize().ok())
        .unwrap_or_default();
    let body = parsed.content;

    let anchors = collect_anchors(&body);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(&body, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);

    let sanitized = ammonia::Builder::default()
        .add_tags(["pre", "code"])
        .add_generic_attributes(["id", "class"])
        .clean(&html_out)
        .to_string();

    let (html, leading_h1) = strip_leading_h1(&sanitized);

    RenderedMarkdown {
        html,
        frontmatter,
        anchors,
        leading_h1,
    }
}

/// Detect a leading `<h1>...</h1>` in rendered HTML, strip it, and return
/// its plain-text content. Skips leading whitespace; tolerates attributes
/// on the h1 (as long as the start tag is `<h1` followed by `>` or
/// whitespace). Returns the original html unchanged when no leading h1 is
/// present.
fn strip_leading_h1(html: &str) -> (String, Option<String>) {
    let trimmed = html.trim_start();
    let leading_ws = html.len() - trimmed.len();
    if !trimmed.starts_with("<h1") {
        return (html.to_string(), None);
    }
    let after_h1 = match trimmed.find('>') {
        Some(i) => &trimmed[i + 1..],
        None => return (html.to_string(), None),
    };
    let close_idx = match after_h1.find("</h1>") {
        Some(i) => i,
        None => return (html.to_string(), None),
    };
    let inner_html = &after_h1[..close_idx];
    let plain_text = strip_inline_tags(inner_html).trim().to_string();
    if plain_text.is_empty() {
        return (html.to_string(), None);
    }
    let after_close_offset =
        leading_ws + (trimmed.len() - after_h1.len()) + close_idx + "</h1>".len();
    let remainder = html[after_close_offset..].trim_start_matches('\n');
    (remainder.to_string(), Some(plain_text))
}

/// Plain-text extractor: drops everything between `<` and `>`. Adequate
/// for h1 inner content, which after ammonia sanitization only contains
/// well-formed inline tags.
fn strip_inline_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Walk the parsed events looking for h2/h3 headings, slugify their text
/// (without trying to inject id="..." into the rendered HTML; that would
/// require a custom event-stream rewriter and is deferred).
fn collect_anchors(markdown: &str) -> Vec<HeadingAnchor> {
    let mut anchors = Vec::new();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    let parser = Parser::new_ext(markdown, options);

    let mut current_level: Option<u32> = None;
    let mut current_text = String::new();

    use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};
    fn level_to_u32(level: HeadingLevel) -> u32 {
        match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    }

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let lvl = level_to_u32(level);
                if matches!(lvl, 2 | 3) {
                    current_level = Some(lvl);
                    current_text.clear();
                } else {
                    current_level = None;
                }
            }
            Event::Text(t) | Event::Code(t) if current_level.is_some() => {
                current_text.push_str(&t);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_level.take() {
                    let text = current_text.trim().to_string();
                    if !text.is_empty() {
                        anchors.push(HeadingAnchor {
                            level,
                            slug: slug::slugify(&text),
                            text,
                        });
                    }
                    current_text.clear();
                }
            }
            _ => {}
        }
    }
    anchors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_basic_markdown() {
        let r = render_markdown("# Hello\n\nworld");
        assert_eq!(r.leading_h1.as_deref(), Some("Hello"));
        assert!(r.html.contains("<p>world</p>"));
    }

    #[test]
    fn parses_frontmatter() {
        let r = render_markdown(
            "---\ntitle: Hi\norder: 5\n---\n# body\n",
        );
        assert_eq!(r.frontmatter.title.as_deref(), Some("Hi"));
        assert_eq!(r.frontmatter.order, Some(5));
        assert_eq!(r.leading_h1.as_deref(), Some("body"));
    }

    #[test]
    fn collects_anchors() {
        let md = "# Title\n\n## Section A\n\nbody\n\n## Section B\n\nmore";
        let r = render_markdown(md);
        assert_eq!(r.anchors.len(), 2);
        assert_eq!(r.anchors[0].slug, "section-a");
        assert_eq!(r.anchors[1].slug, "section-b");
    }

    #[test]
    fn sanitizes_dangerous_html() {
        let r = render_markdown("<script>alert(1)</script>\n\nok");
        assert!(!r.html.contains("<script"));
        assert!(r.html.contains("ok"));
    }

    #[test]
    fn strips_leading_h1_and_returns_text() {
        let r = render_markdown("# Title Here\n\nbody paragraph");
        assert_eq!(r.leading_h1.as_deref(), Some("Title Here"));
        assert!(!r.html.contains("<h1>"), "leading h1 not stripped: {}", r.html);
        assert!(r.html.contains("body paragraph"));
    }

    #[test]
    fn keeps_subsequent_h1s() {
        let md = "# First\n\nintro\n\n# Second";
        let r = render_markdown(md);
        assert_eq!(r.leading_h1.as_deref(), Some("First"));
        assert!(r.html.contains("Second"));
        // The second h1 should still render even though the first was stripped.
        assert!(r.html.contains("<h1>"), "subsequent h1 missing: {}", r.html);
    }

    #[test]
    fn no_leading_h1_returns_none() {
        let r = render_markdown("paragraph only, no heading");
        assert_eq!(r.leading_h1, None);
        assert!(r.html.contains("paragraph only"));
    }

    #[test]
    fn frontmatter_does_not_affect_h1_stripping() {
        let r = render_markdown("---\ntitle: Frontmatter Title\n---\n# Body Title\n\nbody");
        assert_eq!(r.frontmatter.title.as_deref(), Some("Frontmatter Title"));
        assert_eq!(r.leading_h1.as_deref(), Some("Body Title"));
        assert!(!r.html.contains("<h1>"));
    }
}
