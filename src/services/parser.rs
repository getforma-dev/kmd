//! Markdown → HTML rendering pipeline.
//!
//! Uses pulldown-cmark for CommonMark parsing and syntect for server-side
//! syntax highlighting of fenced code blocks.  Mermaid blocks are passed
//! through as `<pre class="mermaid">` for client-side rendering.
//!
//! The final HTML output is sanitized with `ammonia` to prevent XSS from
//! malicious markdown files (e.g. cloned repos with `<script>` tags or
//! `onerror` handlers in READMEs).

use ammonia::Builder;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::collections::HashSet;
use std::sync::LazyLock;
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;

/// Lazily initialised syntax set (loaded once, shared forever).
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Lazily initialised HTML sanitizer.
///
/// Allows safe HTML tags/attributes needed for rendered markdown + syntax
/// highlighting, but strips `<script>`, event handlers (`onerror`, `onclick`,
/// etc.), `<iframe>`, `<object>`, and other XSS vectors.
static SANITIZER: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let mut builder = Builder::default();

    // Allow tags that pulldown-cmark and syntect produce
    let mut tags: HashSet<&str> = HashSet::new();
    for tag in &[
        "a", "abbr", "b", "blockquote", "br", "code", "dd", "del", "details",
        "div", "dl", "dt", "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr",
        "i", "img", "input", "ins", "kbd", "li", "mark", "ol", "p", "pre",
        "q", "s", "samp", "small", "span", "strong", "sub", "summary", "sup",
        "table", "tbody", "td", "tfoot", "th", "thead", "tr", "tt", "u", "ul",
        "var",
    ] {
        tags.insert(tag);
    }
    builder.tags(tags);

    // Allow class and id for syntax highlighting and heading anchors
    let generic_attrs: HashSet<&str> = ["class", "id"].into_iter().collect();
    builder.generic_attributes(generic_attrs);

    // Allow specific attributes on specific tags
    let mut tag_attrs = std::collections::HashMap::new();
    tag_attrs.insert("a", ["href", "title"].into_iter().collect::<HashSet<&str>>());
    tag_attrs.insert("img", ["src", "alt", "title", "width", "height"].into_iter().collect());
    tag_attrs.insert("td", ["colspan", "rowspan", "align"].into_iter().collect());
    tag_attrs.insert("th", ["colspan", "rowspan", "align"].into_iter().collect());
    tag_attrs.insert("input", ["type", "checked", "disabled"].into_iter().collect());
    builder.tag_attributes(tag_attrs);

    // Allow data: URIs for images (base64-embedded images in markdown)
    builder.url_schemes(["http", "https", "mailto", "data"].into_iter().collect());

    // Strip dangerous content rather than escaping it
    builder.strip_comments(true);

    builder
});

/// Render a markdown string to HTML.
///
/// - Fenced code blocks with a recognised language are syntax-highlighted
///   using syntect's `ClassedHTMLGenerator` (CSS class output).
/// - Fenced blocks tagged `mermaid` are wrapped in `<pre class="mermaid">`
///   so the client-side mermaid.js library can pick them up.
/// - Everything else goes through the normal pulldown-cmark HTML pipeline.
pub fn render_markdown(content: &str) -> String {
    let opts = Options::all();
    let parser = Parser::new_ext(content, opts);

    let mut html_output = String::with_capacity(content.len() * 2);

    // State machine for intercepting code blocks
    let mut in_code_block = false;
    let mut code_lang: Option<String> = None;
    let mut code_buf = String::new();

    // We collect events and transform them, then push HTML manually.
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_buf.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang_str = lang.as_ref().trim();
                        if lang_str.is_empty() {
                            None
                        } else {
                            Some(lang_str.to_string())
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Event::Text(text) if in_code_block => {
                code_buf.push_str(&text);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let rendered = render_code_block(code_lang.as_deref(), &code_buf);
                html_output.push_str(&rendered);
                code_lang = None;
                code_buf.clear();
            }
            // For all other events, render through the standard HTML pipeline.
            other => {
                // Use pulldown-cmark's built-in HTML renderer for a single event.
                pulldown_cmark::html::push_html(&mut html_output, std::iter::once(other));
            }
        }
    }

    // Sanitize the final HTML to strip XSS vectors (script tags, event
    // handlers, iframes, etc.) while preserving safe markdown output.
    SANITIZER.clean(&html_output).to_string()
}

/// Render a single code block to HTML.
fn render_code_block(lang: Option<&str>, code: &str) -> String {
    // Mermaid blocks: pass raw content to the client
    if lang == Some("mermaid") {
        let escaped = html_escape(code);
        return format!("<pre class=\"mermaid\">{escaped}</pre>\n");
    }

    let ss = &*SYNTAX_SET;

    // Try to find a syntax definition for the language tag
    let syntax = lang
        .and_then(|l| ss.find_syntax_by_token(l))
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut generator =
        ClassedHTMLGenerator::new_with_class_style(syntax, ss, ClassStyle::Spaced);

    for line in code.lines() {
        // ClassedHTMLGenerator expects lines WITH newlines
        let line_with_nl = format!("{line}\n");
        // Ignore highlight errors gracefully — fall through to plain text
        let _ = generator.parse_html_for_line_which_includes_newline(&line_with_nl);
    }

    let highlighted = generator.finalize();

    let lang_class = lang
        .map(|l| format!(" language-{l}"))
        .unwrap_or_default();

    format!("<pre><code class=\"highlight{lang_class}\">{highlighted}</code></pre>\n")
}

/// Minimal HTML escaping for content placed inside `<pre>` tags.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Basic rendering
    // -----------------------------------------------------------------------

    #[test]
    fn renders_basic_markdown() {
        let html = render_markdown("# Hello\n\nWorld");
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<p>World</p>"));
    }

    #[test]
    fn renders_inline_formatting() {
        let html = render_markdown("**bold** and *italic*");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn renders_links() {
        let html = render_markdown("[click](https://example.com)");
        assert!(html.contains("<a"));
        assert!(html.contains("https://example.com"));
        assert!(html.contains("click"));
    }

    #[test]
    fn renders_images() {
        let html = render_markdown("![alt](https://example.com/img.png)");
        assert!(html.contains("<img"), "expected <img> tag in: {html}");
        assert!(
            html.contains("https://example.com/img.png"),
            "expected image URL in: {html}"
        );
        assert!(html.contains("alt"), "expected alt attribute in: {html}");
    }

    #[test]
    fn renders_tables() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let html = render_markdown(md);
        assert!(html.contains("<table"), "expected <table> tag in: {html}");
        assert!(html.contains("<th"), "expected <th> tag in: {html}");
        // Table data cells
        assert!(
            html.contains(">1<") || html.contains(">1\n"),
            "expected cell content '1' in: {html}"
        );
    }

    // -----------------------------------------------------------------------
    // Code blocks
    // -----------------------------------------------------------------------

    #[test]
    fn renders_fenced_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = render_markdown(md);
        assert!(html.contains("<pre>"));
        assert!(html.contains("<code"));
        assert!(html.contains("language-rust"));
    }

    #[test]
    fn renders_mermaid_block() {
        let md = "```mermaid\ngraph TD\n  A-->B\n```";
        let html = render_markdown(md);
        assert!(html.contains("class=\"mermaid\""));
        assert!(html.contains("graph TD"));
        // Mermaid content should be HTML-escaped
        assert!(html.contains("A--&gt;B"));
    }

    #[test]
    fn renders_unfenced_code_block() {
        let md = "```\nplain code\n```";
        let html = render_markdown(md);
        assert!(html.contains("<pre>"));
        assert!(html.contains("plain code"));
    }

    // -----------------------------------------------------------------------
    // XSS prevention (sanitizer)
    // -----------------------------------------------------------------------

    #[test]
    fn strips_script_tags() {
        let md = "Hello <script>alert('xss')</script> world";
        let html = render_markdown(md);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("alert"));
        assert!(html.contains("Hello"));
        assert!(html.contains("world"));
    }

    #[test]
    fn strips_onerror_handler() {
        let md = "<img src=x onerror=\"alert('xss')\">";
        let html = render_markdown(md);
        assert!(!html.contains("onerror"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn strips_onclick_handler() {
        let md = "<div onclick=\"alert('xss')\">click me</div>";
        let html = render_markdown(md);
        assert!(!html.contains("onclick"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn strips_javascript_uri() {
        let md = "[click](javascript:alert('xss'))";
        let html = render_markdown(md);
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn strips_iframe() {
        let md = "<iframe src=\"https://evil.com\"></iframe>";
        let html = render_markdown(md);
        assert!(!html.contains("<iframe"));
    }

    #[test]
    fn strips_object_embed() {
        let md = "<object data=\"evil.swf\"></object>";
        let html = render_markdown(md);
        assert!(!html.contains("<object"));
    }

    #[test]
    fn strips_svg_onload() {
        let md = "<svg onload=\"alert('xss')\"><circle r=10/></svg>";
        let html = render_markdown(md);
        assert!(!html.contains("onload"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn preserves_safe_html_in_markdown() {
        let md = "Text with <strong>bold</strong> and <em>italic</em>";
        let html = render_markdown(md);
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn preserves_class_attributes() {
        // Syntax highlighting classes should survive sanitization
        let md = "```rust\nlet x = 1;\n```";
        let html = render_markdown(md);
        assert!(html.contains("class="));
    }

    // -----------------------------------------------------------------------
    // html_escape
    // -----------------------------------------------------------------------

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn html_escape_preserves_normal_text() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn handles_empty_input() {
        let html = render_markdown("");
        assert!(html.is_empty() || html.trim().is_empty());
    }

    #[test]
    fn handles_deeply_nested_lists() {
        let md = "- a\n  - b\n    - c\n      - d";
        let html = render_markdown(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>"));
    }
}
