use crate::utils::md::{render::Renderer, CowStr};

pub use crate::utils::md::{ElementAttributes, HtmlElement, LinkDescription, Options};

use dioxus::prelude::*;
use pulldown_cmark::Parser;

pub type HtmlCallback<T> = Callback<T, Element>;

const MARKDOWN: Asset = asset!("/assets/styling/markdown.css");

#[derive(Clone)]
pub struct MdContext;

impl MdContext {
    pub fn set_frontmatter(props: &mut MdProps, frontmatter: String) {
        if let Some(x) = props.frontmatter.as_mut() {
            x.set(frontmatter)
        }
    }

    /// creates a html element, with default attributes
    pub fn el(e: HtmlElement, inside: Element) -> Element {
        MdContext::el_with_attributes(e, inside, Default::default())
    }

    /// renders an empty view
    pub fn el_empty() -> Element {
        MdContext::el_fragment(vec![])
    }

    pub fn render_tasklist_marker(m: bool) -> Element {
        let attributes = ElementAttributes {
            ..Default::default()
        };
        MdContext::el_input_checkbox(m, attributes)
    }

    pub fn render_rule() -> Element {
        let attributes = ElementAttributes {
            ..Default::default()
        };
        MdContext::el_hr(attributes)
    }

    pub fn render_code(s: CowStr<'_>) -> Element {
        let attributes = ElementAttributes {
            ..Default::default()
        };
        MdContext::el_with_attributes(HtmlElement::Code, MdContext::el_text(s), attributes)
    }

    pub fn render_text(s: CowStr<'_>) -> Element {
        let attributes = ElementAttributes {
            ..Default::default()
        };
        MdContext::el_with_attributes(HtmlElement::Span, MdContext::el_text(s), attributes)
    }

    pub fn render_link(props: &MdProps, link: LinkDescription<Element>) -> Result<Element, String> {
        if let Some(links) = props.render_links {
            Ok(links(link))
        } else {
            Ok(if link.image {
                MdContext::el_img(link.url, link.title)
            } else {
                MdContext::el_a(link.content, link.url)
            })
        }
    }

    pub fn el_with_attributes(
        e: HtmlElement,
        inside: Element,
        attributes: ElementAttributes,
    ) -> Element {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();

        match e {
            HtmlElement::Div => {
                rsx! {
                    div { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Span => {
                rsx! {
                    span { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Paragraph => {
                rsx! {
                    p { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::BlockQuote => {
                rsx! {
                    blockquote {  style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Ul => {
                rsx! {
                    ul {  style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Ol(x) => {
                rsx! {
                    ol {
                        style: "{style}",
                        class: "{class}",
                        start: x as i64,
                        {inside}
                    }
                }
            }
            HtmlElement::Li => {
                rsx! {
                    li { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(1) => {
                rsx! {
                    h1 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(2) => {
                rsx! {
                    h2 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(3) => {
                rsx! {
                    h3 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(4) => {
                rsx! {
                    h4 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(5) => {
                rsx! {
                    h5 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(6) => {
                rsx! {
                    h6 { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Heading(_) => panic!(),
            HtmlElement::Table => {
                rsx! {
                    table { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Thead => {
                rsx! {
                    thead { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Trow => {
                rsx! {
                    tr { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Tcell => {
                rsx! {
                    td { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Italics => {
                rsx! {
                    i { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Bold => {
                rsx! {
                    b { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::StrikeThrough => {
                rsx! {
                    s { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Pre => {
                rsx! {
                    p { style: "{style}", class: "{class}", {inside} }
                }
            }
            HtmlElement::Code => {
                rsx! {
                    code { style: "{style}", class: "{class}", {inside} }
                }
            }
        }
    }

    pub fn el_span_with_inner_html(inner_html: String, attributes: ElementAttributes) -> Element {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();
        rsx! {
            span {
                dangerous_inner_html: "{inner_html}",
                style: "{style}",
                class: "{class}",
            }
        }
    }

    pub fn el_hr(attributes: ElementAttributes) -> Element {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();
        rsx!(hr {
            style: "{style}",
            class: "{class}"
        })
    }

    pub fn el_br() -> Element {
        rsx!(br {})
    }

    pub fn el_fragment(children: Vec<Element>) -> Element {
        rsx! {
            {children.into_iter()}
        }
    }

    pub fn el_a(children: Element, href: String) -> Element {
        rsx! {
            a { href: "{href}", {children} }
        }
    }

    pub fn el_img(src: String, alt: String) -> Element {
        rsx!(img {
            src: "{src}",
            alt: "{alt}"
        })
    }

    pub fn el_text(text: CowStr<'_>) -> Element {
        rsx! {
            {text.as_ref()}
        }
    }

    pub fn el_input_checkbox(checked: bool, attributes: ElementAttributes) -> Element {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();
        rsx!(input {
            r#type: "checkbox",
            checked,
            style: "{style}",
            class: "{class}",
        })
    }
}

#[derive(Clone, PartialEq, Props)]
pub struct MdProps {
    src: String,

    /// links in the markdown
    render_links: Option<HtmlCallback<LinkDescription<Element>>>,

    /// the name of the theme used for syntax highlighting.
    /// Only the default themes of [syntect::Theme] are supported
    #[props(default = "base16-ocean.light".to_string())]
    pub syntax_theme: String,

    /// wether to enable wikilinks support.
    /// Wikilinks look like [[shortcut link]] or [[url|name]]
    #[props(default = false)]
    wikilinks: bool,

    /// wether to convert soft breaks to hard breaks.
    #[props(default = true)]
    hard_line_breaks: bool,

    /// pulldown_cmark options.
    /// See [`Options`][pulldown_cmark_wikilink::Options] for reference.
    parse_options: Option<Options>,

    frontmatter: Option<Signal<String>>,
}

#[allow(non_snake_case)]
pub fn Markdown(props: MdProps) -> Element {
    let src: String = props.src.clone();

    let parse_options_default = Options::ENABLE_GFM
        | Options::ENABLE_MATH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let options = props.parse_options.unwrap_or(parse_options_default);
    let mut stream: Vec<_> = Parser::new_ext(&src, options).into_offset_iter().collect();

    if props.hard_line_breaks {
        for (r, _) in &mut stream {
            if *r == pulldown_cmark::Event::SoftBreak {
                *r = pulldown_cmark::Event::HardBreak
            }
        }
    }

    let elements = Renderer::new(props, &mut stream.into_iter()).collect::<Vec<_>>();
    let child = MdContext::el_fragment(elements);

    rsx! {
        document::Link { rel: "stylesheet", href: MARKDOWN }
        div { class: "markdown",
            {child}
        }
    }
}
