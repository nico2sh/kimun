use std::sync::Arc;

use crate::utils::md::{render::Renderer, CowStr};

pub use crate::utils::md::{ElementAttributes, HtmlElement, LinkDescription, Options};

use dioxus::{
    logger::tracing::{debug, error},
    prelude::*,
};
use kimun_core::{
    nfs::VaultPath,
    note::{LinkType, NoteLink},
    NoteVault,
};
use pulldown_cmark::Parser;
use syntect::highlighting::Style;

use super::modal::ModalType;

const MARKDOWN_SYLE: Asset = asset!("/assets/styling/markdown.css");

#[derive(Clone)]
pub struct MdContext;

impl MdContext {
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

    pub fn render_link(props: &MarkdownProps, link: LinkDescription<Element>) -> Element {
        if let Some(note_link) = props
            .note_links
            .iter()
            .find(|note_link| link.url.eq(&note_link.raw_link))
        {
            MdContext::kimun_link(props, link.content, note_link)
        } else if link.image {
            MdContext::el_img(link.title, link.url)
        } else {
            MdContext::el_a(link.content, link.url)
        }
    }

    fn kimun_link(props: &MarkdownProps, children: Element, note_link: &NoteLink) -> Element {
        rsx! {
            match &note_link.ltype {
                LinkType::Note(vault_path) => MdContext::el_note(props, children, vault_path),
                LinkType::Attachment(vault_path) => MdContext::el_attachment(props, children, vault_path),
                LinkType::Url => MdContext::el_a(children, note_link.raw_link.clone()),
            }
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

    pub fn line_to_span(ranges: Vec<(Style, &str)>) -> Element {
        rsx! {
            for (style, text) in ranges {
                span { style: "color:rgb({style.foreground.r},{style.foreground.g},{style.foreground.b});", "{text}" }
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

    pub fn el_note(props: &MarkdownProps, children: Element, note_path: &VaultPath) -> Element {
        let note_path = note_path.to_owned();
        let vault = props.vault.clone();
        let mut modal_type = props.modal_type;
        rsx! {
            span { class: "icon-note note-link",
                onclick: move |_e| {
                    match vault.open_or_search(&note_path) {
                        Ok(res) => {
                            match res.len() {
                                0 => {
                                    // Create new note
                                    navigator().replace(crate::Route::MainView { editor_path: note_path.clone(), create: true });
                                },
                                1 => {
                                    // Open note
                                    navigator().replace(crate::Route::MainView { editor_path: note_path.clone(), create: false });
                                },
                                _ => {
                                    // Show picker
                                    debug!("Show picker for {note_path}");
                                    let note_list = res.iter().map(|(data, details)| {
                                        (details.title.clone(), data.path.clone())
                                    }).collect();
                                    modal_type.set(ModalType::NotePicker { note_list });
                                }
                            }
                        },
                        Err(e) => {
                            error!("Error clicking on note: {}", e);
                        },
                    }
                },
                {children}
            }
        }
    }

    pub fn el_attachment(
        props: &MarkdownProps,
        children: Element,
        note_path: &VaultPath,
    ) -> Element {
        let full_path = props.vault.path_to_pathbuf(note_path);
        let fp = full_path.to_string_lossy();
        rsx! {
            a { class: "icon-attachment note-attachment", href: "{fp}", {children} }
        }
    }

    pub fn el_img(alt: String, src: String) -> Element {
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
pub struct MarkdownProps {
    vault: Arc<NoteVault>,

    note_md: String,
    /// links in the markdown
    note_links: Vec<NoteLink>,
    modal_type: Signal<ModalType>,

    /// the name of the theme used for syntax highlighting.
    /// Only the default themes of [syntect::Theme] are supported
    #[props(default = "base16-ocean.light".to_string())]
    pub syntax_theme: String,

    /// wether to convert soft breaks to hard breaks.
    #[props(default = true)]
    hard_line_breaks: bool,
}

#[allow(non_snake_case)]
pub fn Markdown(props: MarkdownProps) -> Element {
    let src: String = props.note_md.clone();

    let parse_options = Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_SMART_PUNCTUATION;
    let mut stream: Vec<_> = Parser::new_ext(&src, parse_options)
        .into_offset_iter()
        .collect();

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
        document::Link { rel: "stylesheet", href: MARKDOWN_SYLE }
        div { class: "markdown",
            {child}
        }
    }
}
