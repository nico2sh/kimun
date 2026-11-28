//! Turning a fetched page into markdown: pick the main article out of an HTML
//! page, convert it, and pass plain text and markdown through.
//!
//! The HTML work is done by the `page2md` crate. It limits the nesting depth,
//! the attributes in one tag and the parse time, and it drops unsafe link and
//! image targets. This module maps its answers to [`ClipError`].

use std::time::Duration;

use page2md::{Converter, Error as ConvertError};

use super::ClipError;

/// The longest title the clip keeps, in characters.
const MAX_TITLE_CHARS: usize = 200;

/// The largest HTML body that is converted, in bytes. The fetch cap bounds
/// memory. This cap bounds conversion time, which grows with size.
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// The time page2md may spend parsing one page. A normal 1.5 MB article takes
/// about 0.1 s in a release build, so this leaves a wide margin on a slow or
/// busy machine. A hostile page still ends here, and the 30 s limit in
/// `clip_url` stays as the backstop for the work after the parse.
const PARSE_BUDGET: Duration = Duration::from_secs(10);

/// A page turned into markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// A one-line title, when the page has one.
    pub title: Option<String>,
    /// The page content as markdown.
    pub markdown: String,
}

/// Turn a fetched body into markdown.
///
/// `content_type` is the value of the `Content-Type` header, if any. `url` is
/// the address of the page, used to make relative links absolute.
pub fn extract_markdown(
    body: &str,
    content_type: Option<&str>,
    url: &str,
) -> Result<Article, ClipError> {
    let media = content_type
        .map(|c| c.split(';').next().unwrap_or("").trim().to_lowercase())
        .filter(|m| !m.is_empty());
    let article = match media.as_deref() {
        Some("text/html" | "application/xhtml+xml") => html_to_article(body, url)?,
        Some("text/markdown" | "text/plain") => passthrough(body),
        Some(other) => return Err(ClipError::UnsupportedType(other.to_string())),
        None if body.trim_start().starts_with('<') => html_to_article(body, url)?,
        None => passthrough(body),
    };
    if article.markdown.trim().is_empty() {
        return Err(ClipError::NoContent);
    }
    Ok(article)
}

fn passthrough(body: &str) -> Article {
    Article {
        title: None,
        markdown: body.to_string(),
    }
}

fn html_to_article(body: &str, url: &str) -> Result<Article, ClipError> {
    if body.trim().is_empty() {
        return Err(ClipError::NoContent);
    }
    let mut builder = Converter::builder()
        .extract_main_content(true)
        .max_input_bytes(MAX_HTML_BYTES)
        .time_budget(PARSE_BUDGET);
    // A page address that does not parse is not worth failing for: links stay as written.
    if let Ok(base) = url.parse() {
        builder = builder.base_url(base);
    }
    let converted = builder.build().convert(body).map_err(refusal)?;
    Ok(Article {
        title: converted.title.as_deref().and_then(clean_title),
        markdown: converted.markdown,
    })
}

/// The clip error for a conversion that page2md refused.
fn refusal(error: ConvertError) -> ClipError {
    match error {
        ConvertError::InputTooLarge { limit, .. } => ClipError::TooLarge { limit },
        ConvertError::TooDeep { .. } => {
            ClipError::UnsupportedType("a page that is nested too deeply".to_string())
        }
        ConvertError::TooManyAttributes { .. } => {
            ClipError::UnsupportedType("a page with too many attributes in one tag".to_string())
        }
        ConvertError::TimedOut { .. } => {
            ClipError::UnsupportedType("a page that takes too long to convert".to_string())
        }
        // The error type can grow. A refusal we do not know is still a refusal.
        _ => ClipError::UnsupportedType("a page that cannot be converted".to_string()),
    }
}

/// One line, no leading `#`, at most [`MAX_TITLE_CHARS`] characters.
fn clean_title(raw: &str) -> Option<String> {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = one_line.trim_start_matches(['#', ' ']);
    let cut: String = trimmed.chars().take(MAX_TITLE_CHARS).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    const URL: &str = "https://example.com/post/1";
    const HTML: Option<&str> = Some("text/html");

    fn ok(body: &str) -> Article {
        extract_markdown(body, HTML, URL).unwrap()
    }

    /// Run the conversion on its own thread and fail, not hang, when it takes
    /// more than a minute.
    fn bounded(body: String) -> Result<Article, ClipError> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(extract_markdown(&body, HTML, URL));
        });
        rx.recv_timeout(Duration::from_secs(60))
            .expect("no answer within 60 s")
    }

    fn refused(body: String) -> String {
        match bounded(body) {
            Err(ClipError::UnsupportedType(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn article_page() -> String {
        let para = "This is a long paragraph about the topic, with enough words to look like \
                    real content for the scoring, and a few commas, here and there. ";
        format!(
            "<html><head><title>My Post</title></head><body>\
             <nav><ul><li>NAVTEXT home</li><li>NAVTEXT about</li></ul></nav>\
             <article><h1>Big Heading</h1><p>{p}</p><p>{p}</p><p>{p}</p></article>\
             <footer>FOOTERTEXT copyright</footer></body></html>",
            p = para.repeat(3)
        )
    }

    #[test]
    fn article_drops_navigation_and_footer() {
        let a = ok(&article_page());
        assert!(a.markdown.contains("Big Heading"), "{}", a.markdown);
        assert!(a.markdown.contains("real content"));
        assert!(!a.markdown.contains("NAVTEXT"), "{}", a.markdown);
        assert!(!a.markdown.contains("FOOTERTEXT"), "{}", a.markdown);
        assert_eq!(a.title.as_deref(), Some("My Post"));
    }

    #[test]
    fn page_without_article_falls_back_to_body() {
        let a = ok("<html><head><title>Tiny</title></head><body><p>Hello there</p></body></html>");
        assert!(a.markdown.contains("Hello there"), "{}", a.markdown);
        assert_eq!(a.title.as_deref(), Some("Tiny"));
    }

    #[test]
    fn markdown_and_plain_pass_through() {
        for ct in ["text/markdown", "text/plain"] {
            let a = extract_markdown("# Hi\n\n[[x]] <b>raw</b>\n", Some(ct), URL).unwrap();
            assert_eq!(a.markdown, "# Hi\n\n[[x]] <b>raw</b>\n");
            assert_eq!(a.title, None);
        }
    }

    #[test]
    fn unsupported_type() {
        assert_eq!(
            extract_markdown("%PDF", Some("application/pdf"), URL),
            Err(ClipError::UnsupportedType("application/pdf".into()))
        );
    }

    #[test]
    fn content_type_parameters_and_case() {
        for ct in ["text/html; charset=utf-8", "TEXT/HTML", " Text/Html ;x=y"] {
            let a = extract_markdown(
                "<html><body><p>Hello there</p></body></html>",
                Some(ct),
                URL,
            )
            .unwrap();
            assert!(a.markdown.contains("Hello there"), "{ct}");
            assert!(!a.markdown.contains('<'), "{ct}");
        }
    }

    #[test]
    fn empty_and_whitespace_bodies() {
        for body in ["", "  \n\t "] {
            for ct in [HTML, Some("text/plain"), Some("text/markdown"), None] {
                assert_eq!(
                    extract_markdown(body, ct, URL),
                    Err(ClipError::NoContent),
                    "{body:?} {ct:?}"
                );
            }
        }
        for body in ["<html><body>   </body></html>", "<p></p>"] {
            for ct in [HTML, None] {
                assert_eq!(
                    extract_markdown(body, ct, URL),
                    Err(ClipError::NoContent),
                    "{body:?} {ct:?}"
                );
            }
        }
    }

    #[test]
    fn script_and_style_do_not_leak() {
        let a = ok(
            "<html><head><style>.a{color:red}</style><script>var HEAD_JS=1;</script></head>\
             <body><p>Visible text</p><script>var LEAKJS=1;</script><style>.b{x:y}</style>\
             <noscript>NOSCRIPTTEXT</noscript><iframe>IFRAMETEXT</iframe>\
             <template>TEMPLATETEXT</template><svg><text>SVGTEXT</text></svg></body></html>",
        );
        assert!(a.markdown.contains("Visible text"));
        // page2md keeps the fallback text of an iframe, as plain text: it is
        // the text the page shows to a reader that cannot load the frame.
        for leaked in [
            "HEAD_JS",
            "LEAKJS",
            "color:red",
            "x:y",
            "NOSCRIPTTEXT",
            "TEMPLATETEXT",
            "SVGTEXT",
        ] {
            assert!(!a.markdown.contains(leaked), "{leaked} in {}", a.markdown);
        }
    }

    #[test]
    fn wiki_brackets_do_not_panic() {
        let a = ok("<html><body><p>See [[x]] and [[y|z]] here</p></body></html>");
        assert!(a.markdown.contains('x'));
    }

    #[test]
    fn relative_links_become_absolute() {
        let a = ok("<html><body><p>Go <a href=\"/other\">there</a></p></body></html>");
        assert!(
            a.markdown.contains("https://example.com/other"),
            "{}",
            a.markdown
        );
    }

    #[test]
    fn a_page_address_that_does_not_parse_leaves_links_as_written() {
        let a = extract_markdown(
            "<html><body><p>Go <a href=\"/other\">there</a></p></body></html>",
            HTML,
            "not a url",
        )
        .unwrap();
        assert!(a.markdown.contains("(/other)"), "{}", a.markdown);
    }

    #[test]
    fn title_is_one_line() {
        let a = ok("<html><head><title>One\n\ttwo \r\n  three</title></head>\
                    <body><p>Hello there</p></body></html>");
        assert_eq!(a.title.as_deref(), Some("One two three"));
    }

    #[test]
    fn clean_title_rules() {
        assert_eq!(clean_title("# Hi"), Some("Hi".into()));
        assert_eq!(clean_title("## # Hi there"), Some("Hi there".into()));
        assert_eq!(clean_title("a\n\tb\r\nc"), Some("a b c".into()));
        assert_eq!(clean_title("#"), None);
        assert_eq!(clean_title("###  \n"), None);
        assert_eq!(clean_title("  \t\n"), None);
        assert_eq!(clean_title(""), None);
        let long = "a".repeat(500);
        assert_eq!(clean_title(&long).unwrap().chars().count(), 200);
        let wide = "\u{20AC}".repeat(300);
        let t = clean_title(&wide).unwrap();
        assert_eq!(t.chars().count(), 200);
        assert!(t.chars().all(|c| c == '\u{20AC}'));
    }

    #[test]
    fn title_of_only_hashes_is_none() {
        let a = ok("<html><head><title>###</title></head><body><p>Hello there</p></body></html>");
        assert_eq!(a.title, None);
    }

    #[test]
    fn absent_type_sniffs_html_or_text() {
        let a =
            extract_markdown("<html><body><p>Hello there</p></body></html>", None, URL).unwrap();
        assert!(a.markdown.contains("Hello there"));
        assert!(!a.markdown.contains("<p>"));
        let t = extract_markdown("just plain words", None, URL).unwrap();
        assert_eq!(t.markdown, "just plain words");
        assert_eq!(t.title, None);
        let l =
            extract_markdown("\n  <html><body><p>Hi there</p></body></html>", None, URL).unwrap();
        assert!(!l.markdown.contains("<p>"));
    }

    #[test]
    fn robust_on_odd_input() {
        for body in [
            "<",
            "<<<<>>>>",
            "<div><p><b>unclosed",
            "<html><body><p>bad \u{FFFD}\u{FFFD} chars \u{FFFD}</p></body></html>",
            "<a href=\"",
            "<!--",
            "<script>",
            "\u{20AC}<\u{20AC}>\u{20AC}</\u{20AC}",
            "<d\u{20AC}iv>\u{1F600}</\u{1F600}>",
            "<a title='\u{20AC}",
            "<p>hi</p><div class=\"x",
        ] {
            let _ = extract_markdown(body, HTML, URL);
            let _ = extract_markdown(body, None, URL);
        }
    }

    #[test]
    fn html_over_2_mib_is_too_large() {
        let limit = 2 * 1024 * 1024;
        let body = format!("<p>{}</p>", "a".repeat(limit));
        assert_eq!(
            extract_markdown(&body, HTML, URL),
            Err(ClipError::TooLarge { limit })
        );
        let plain = "a".repeat(limit + 1);
        assert!(extract_markdown(&plain, Some("text/plain"), URL).is_ok());
    }

    #[test]
    fn deep_nesting_is_refused_fast() {
        let why = refused("<div>".repeat(300_000));
        assert!(why.contains("nested too deeply"), "{why}");
        let why = refused(format!("{}x", "<b><div></b>".repeat(100_000)));
        assert!(why.contains("nested too deeply"), "{why}");
    }

    #[test]
    fn a_tag_with_many_attributes_is_refused_fast() {
        let mut body = String::from("<p ");
        for i in 0..3000 {
            body.push_str(&format!("a{i}=1 "));
        }
        body.push_str(">text</p>");
        let why = refused(body);
        assert!(why.contains("too many attributes"), "{why}");
    }

    #[test]
    fn nested_objects_around_heavy_text_convert_fast() {
        let body = format!(
            "<article>{}{}{}</article>",
            "<object>".repeat(140),
            "&".repeat(180_000),
            "</object>".repeat(140)
        );
        bounded(body).unwrap();
    }

    #[test]
    fn a_large_code_block_with_entities_converts() {
        let code = "if a &lt; b &amp;&amp; c {}\n".repeat(60_000);
        let body = format!(
            "<article><p>An introduction, long enough to read.</p>\
             <pre><code>{code}</code></pre></article>"
        );
        bounded(body).unwrap();
    }

    #[test]
    fn a_realistic_large_article_converts() {
        let para = "<p>Tom &amp; Jerry&nbsp;said &lt;hi&gt; and left, then came back.</p>";
        let body = format!(
            "<html><head><title>T</title></head><body><article>{}</article></body></html>",
            para.repeat(1_500_000 / para.len())
        );
        let a = bounded(body).unwrap();
        assert_eq!(a.title.as_deref(), Some("T"));
    }

    /// Every link or image target in `md` uses http, https, mailto, or no
    /// scheme.
    fn assert_safe_targets(md: &str) {
        let targets = regex::Regex::new(r"\]\(\s*<?([^)>\s]*)").unwrap();
        for c in targets.captures_iter(md) {
            let t = c[1].to_ascii_lowercase().replace('\\', "");
            let head = t.split(['/', '?', '#']).next().unwrap_or("");
            if let Some((scheme, _)) = head.split_once(':') {
                assert!(matches!(scheme, "http" | "https" | "mailto"), "{t} in {md}");
            }
        }
    }

    #[test]
    fn unsafe_link_targets_lose_the_target() {
        for href in [
            "javascript:alert(1)",
            " JaVa&#x09;script:b",
            "java&colon;script:q",
            "vbscript:c",
            "data:text/html,x",
            "file:///etc/passwd",
        ] {
            let a = ok(&format!(
                "<html><body><p>Go <a href=\"{href}\">THERE</a></p></body></html>"
            ));
            assert!(a.markdown.contains("THERE"), "{href}: {}", a.markdown);
            assert!(!a.markdown.contains("]("), "{href}: {}", a.markdown);
        }
    }

    #[test]
    fn safe_link_targets_stay() {
        let a = ok("<html><body><p><a href=\"https://a.example/p\">h</a> \
                    <a href=\"mailto:a@b.c\">m</a> <a href=\"/rel\">r</a></p></body></html>");
        assert!(
            a.markdown.contains("(https://a.example/p)"),
            "{}",
            a.markdown
        );
        assert!(a.markdown.contains("(mailto:a@b.c)"), "{}", a.markdown);
        assert!(
            a.markdown.contains("(https://example.com/rel)"),
            "{}",
            a.markdown
        );
    }

    #[test]
    fn a_page_base_cannot_change_how_links_resolve() {
        let a = ok("<html><head><base href=\"javascript:alert(1)//\"></head>\
                    <body><p><a href=\"/x\">x</a></p></body></html>");
        assert_safe_targets(&a.markdown);
        assert!(
            a.markdown.contains("(https://example.com/x)"),
            "{}",
            a.markdown
        );
    }

    #[test]
    fn page_text_cannot_form_a_link() {
        let a = ok("<html><body><p>`x` [e](javascript:z)</p>\
                    <p><a href=\"/ok\" title='\") [z](javascript:q) (\"'>ok</a></p></body></html>");
        assert_safe_targets(&a.markdown);
    }
}
