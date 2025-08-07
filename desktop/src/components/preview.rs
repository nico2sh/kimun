use crate::utils::md::{markdown_component, CowStr, MarkdownProps};

pub use crate::utils::md::{Context, ElementAttributes, HtmlElement, LinkDescription, Options};

use dioxus::{logger::tracing::debug, prelude::*};

use super::text_editor::EditorContent;

pub type HtmlCallback<T> = Callback<T, Element>;

const MARKDOWN: Asset = asset!("/assets/styling/markdown.css");

#[derive(Clone, PartialEq, Default, Props)]
pub struct MdProps {
    src: Signal<EditorContent>,

    /// links in the markdown
    render_links: Option<HtmlCallback<LinkDescription<Element>>>,

    /// the name of the theme used for syntax highlighting.
    /// Only the default themes of [syntect::Theme] are supported
    theme: Option<&'static str>,

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

#[derive(Clone, Copy)]
pub struct MdContext(ReadOnlySignal<MdProps>);

impl Context<'_, 'static> for MdContext {
    type View = Element;

    type Handler<T: 'static> = EventHandler<T>;

    type MouseEvent = MouseEvent;

    fn props(self) -> MarkdownProps {
        let props = self.0();

        MarkdownProps {
            hard_line_breaks: props.hard_line_breaks,
            wikilinks: props.wikilinks,
            parse_options: props.parse_options,
            theme: props.theme,
        }
    }

    fn set_frontmatter(&mut self, frontmatter: String) {
        if let Some(x) = self.0().frontmatter.as_mut() {
            x.set(frontmatter)
        }
    }

    fn render_links(self, link: LinkDescription<Self::View>) -> Result<Self::View, String> {
        // TODO: remove the unwrap call
        Ok(self.0().render_links.as_ref().unwrap()(link))
    }

    fn call_handler<T: 'static>(callback: &Self::Handler<T>, input: T) {
        callback.call(input)
    }

    fn make_md_handler(self, stop_propagation: bool) -> Self::Handler<MouseEvent> {
        EventHandler::new(move |e: MouseEvent| {
            if stop_propagation {
                e.stop_propagation()
            }
        })
    }

    fn el_with_attributes(
        self,
        e: HtmlElement,
        inside: Self::View,
        attributes: ElementAttributes,
    ) -> Self::View {
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
                debug!("Code");
                rsx! {
                    code { style: "{style}", class: "{class}", {inside} }
                }
            }
        }
    }

    fn el_span_with_inner_html(
        self,
        inner_html: String,
        attributes: ElementAttributes,
    ) -> Self::View {
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

    fn el_hr(self, attributes: ElementAttributes) -> Self::View {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();
        rsx!(hr {
            style: "{style}",
            class: "{class}"
        })
    }

    fn el_br(self) -> Self::View {
        rsx!(br {})
    }

    fn el_fragment(self, children: Vec<Self::View>) -> Self::View {
        rsx! {
            {children.into_iter()}
        }
    }

    fn el_a(self, children: Self::View, href: String) -> Self::View {
        rsx! {
            a { href: "{href}", {children} }
        }
    }

    fn el_img(self, src: String, alt: String) -> Self::View {
        rsx!(img {
            src: "{src}",
            alt: "{alt}"
        })
    }

    fn el_text(self, text: CowStr<'_>) -> Self::View {
        rsx! {
            {text.as_ref()}
        }
    }

    fn el_input_checkbox(self, checked: bool, attributes: ElementAttributes) -> Self::View {
        let class = attributes.classes.join(" ");
        let style = attributes.style.unwrap_or_default();
        rsx!(input {
            r#type: "checkbox",
            checked,
            style: "{style}",
            class: "{class}",
        })
    }

    fn has_custom_links(self) -> bool {
        self.0().render_links.is_some()
    }
}

#[allow(non_snake_case)]
pub fn Markdown(props: MdProps) -> Element {
    let src: String = props.src.read().get_text();
    let signal: Signal<MdProps> = Signal::new(props);
    let child = markdown_component(MdContext(signal.into()), &src);
    rsx! {
        document::Link { rel: "stylesheet", href: MARKDOWN }
        div { class: "markdown",
            {child}
        }
    }
}
