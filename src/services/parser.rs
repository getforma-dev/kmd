//! Markdown → HTML rendering pipeline.
//!
//! Uses pulldown-cmark for CommonMark parsing and syntect for server-side
//! syntax highlighting of fenced code blocks.  Mermaid blocks are passed
//! through as `<pre class="mermaid">` for client-side rendering.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::sync::LazyLock;
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;

/// Lazily initialised syntax set (loaded once, shared forever).
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

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

    html_output
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
