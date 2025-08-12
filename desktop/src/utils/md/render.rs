use core::ops::Range;

use dioxus::logger::tracing::debug;
use dioxus::prelude::Element;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Tag, TagEnd};
use syntect::util::LinesWithEndings;

#[derive(Eq, PartialEq)]
enum MathMode {
    Inline,
    Display,
}

use crate::components::preview::{MdContext, MdProps};

use super::HtmlElement::*;
use super::{ElementAttributes, HtmlError, LinkDescription};

// load the default syntect options to highlight code
lazy_static::lazy_static! {
    static ref SYNTAX_SET: SyntaxSet = {
        SyntaxSet::load_defaults_newlines()
    };
    static ref THEME_SET: ThemeSet = {
        ThemeSet::load_defaults()
    };
}

impl HtmlError {
    fn not_implemented(message: impl ToString) -> Self {
        HtmlError::NotImplemented(message.to_string())
    }
}

// fn highlight_code(theme_name: &str, content: &str, kind: &CodeBlockKind) -> Option<String> {
//     let lang = match kind {
//         CodeBlockKind::Fenced(x) => x,
//         CodeBlockKind::Indented => return None,
//     };
//
//     let theme = THEME_SET
//         .themes
//         .get(theme_name)
//         .expect("unknown theme")
//         .clone();
//
//     syntect::html::highlighted_html_for_string(
//         content,
//         &SYNTAX_SET,
//         SYNTAX_SET.find_syntax_by_token(lang)?,
//         &theme,
//     )
//     .ok()
// }

fn highlight_code_element(
    theme_name: &str,
    content: &str,
    kind: &CodeBlockKind,
) -> Option<Element> {
    let lang = match kind {
        CodeBlockKind::Fenced(x) => x,
        CodeBlockKind::Indented => return None,
    };

    let theme = THEME_SET
        .themes
        .get(theme_name)
        .expect("unknown theme")
        .clone();

    let ps = SyntaxSet::load_defaults_nonewlines();
    let syntax = ps
        .find_syntax_by_token(lang)
        .unwrap_or(ps.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, &theme);
    let mut lines = vec![];
    let mut rgb = None;
    for line in LinesWithEndings::from(content) {
        let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ps).unwrap();
        if rgb.is_none() && !ranges.is_empty() {
            let first_line = ranges.first().unwrap();
            rgb = Some((
                first_line.0.background.r,
                first_line.0.background.g,
                first_line.0.background.b,
            ));
        }
        lines.push(MdContext::line_to_span(ranges));
    }

    let attributes = match rgb {
        Some((r, g, b)) => ElementAttributes {
            classes: vec![],
            style: Some(format!("background:rgb({r}, {g}, {b})")),
        },
        None => ElementAttributes::default(),
    };

    Some(MdContext::el_with_attributes(
        Code,
        MdContext::el_fragment(lines),
        attributes,
    ))
}

/// renders a source code in a code block, with syntax highlighting if possible.
/// `cx`: the current markdown context
/// `source`: the source to render
/// `range`: the position of the code in the original source
fn render_code_block(props: &MdProps, source: String, k: &CodeBlockKind) -> Element {
    let code_attributes = ElementAttributes {
        classes: vec!["code".to_string()],
        ..Default::default()
    };

    match highlight_code_element(&props.syntax_theme, &source, k) {
        None => {
            debug!("Indented");
            MdContext::el_with_attributes(Code, MdContext::el_text(source.into()), code_attributes)
        }
        // None => cx.el_with_attributes(
        //     Code,
        //     cx.el(Code, cx.el_text(source.into())),
        //     code_attributes,
        // ),
        Some(x) => x,
    }
    // MdContext::el_with_attributes(Div, code, code_attributes)
}

fn render_maths(
    _content: &str,
    _display_mode: MathMode,
    _range: Range<usize>,
) -> Result<Element, HtmlError> {
    Err(HtmlError::UnAvailable(
        "Math was not enabled during compilation of the library. Please unable the `maths` feature"
            .into(),
    ))
}

/// `align_string(align)` gives the css string
/// that is used to align text according to `align`
fn align_string(align: Alignment) -> &'static str {
    match align {
        Alignment::Left => "text-align: left",
        Alignment::Right => "text-align: right",
        Alignment::Center => "text-align: center",
        Alignment::None => "",
    }
}

/// Manage the creation of a [`F::View`]
/// from a stream of markdown events
pub struct Renderer<'a, 'c, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    /// the markdown context
    props: MdProps,
    /// the stream of markdown [`Event`]s
    stream: &'c mut I,
    /// the alignment settings inside the current table
    column_alignment: Option<Vec<Alignment>>,
    /// the current horizontal index of the cell we are in.
    /// TODO: remove it
    cell_index: usize,
    /// the root tag that this renderer is rendering
    end_tag: Option<TagEnd>,
    /// the current component we are inside of.
    /// custom components doesn't allow nesting.
    current_component: Option<String>,
}

impl<'a, I> Iterator for Renderer<'a, '_, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = Element;

    fn next(&mut self) -> Option<Self::Item> {
        use Event::*;
        let (item, range): (Event<'a>, Range<usize>) = self.stream.next()?;
        let range = range.clone();

        // let props = self.props;

        let rendered = match item {
            Start(t) => self.render_tag(t),
            End(end) => {
                // check if the closing tag is the tag that was open
                // when this renderer was created
                match self.end_tag {
                    Some(t) if t == end => return None,
                    Some(t) => panic!("{end:?} is a wrong closing tag, expected {t:?}"),
                    None => panic!("didn't expect a closing tag"),
                }
            }
            Text(s) => Ok(MdContext::render_text(s)),
            Code(s) => Ok(MdContext::render_code(s)),
            InlineHtml(s) => Ok(MdContext::el_span_with_inner_html(
                s.to_string(),
                Default::default(),
            )),
            Html(s) => Ok(MdContext::el_span_with_inner_html(
                s.to_string(),
                Default::default(),
            )),
            FootnoteReference(_) => Err(HtmlError::not_implemented("footnotes refs")),
            SoftBreak => Ok(MdContext::el_text(" ".into())),
            HardBreak => Ok(MdContext::el_br()),
            Rule => Ok(MdContext::render_rule()),
            TaskListMarker(m) => Ok(MdContext::render_tasklist_marker(m)),
            InlineMath(content) => render_maths(&content, MathMode::Inline, range),
            DisplayMath(content) => render_maths(&content, MathMode::Display, range),
        };

        Some(rendered.unwrap_or_else(|e| {
            MdContext::el_with_attributes(
                Span,
                MdContext::el_fragment(vec![
                    MdContext::el_text(e.to_string().into()),
                    MdContext::el_br(),
                ]),
                ElementAttributes {
                    classes: vec!["markdown-error".to_string()],
                    ..Default::default()
                },
            )
        }))
    }
}

impl<'a, 'c, I> Renderer<'a, 'c, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    /// creates a new renderer from a stream of events.
    /// It returns an iterator of [`F::View`]
    pub fn new(props: MdProps, events: &'c mut I) -> Self {
        Self {
            props,
            stream: events,
            column_alignment: None,
            cell_index: 0,
            end_tag: None,
            current_component: None,
        }
    }

    /// renders events in a new renderer,
    /// recursively, until the end of the tag
    fn children(&mut self, tag: Tag<'a>) -> Element {
        let sub_renderer = Renderer {
            props: self.props.clone(),
            stream: self.stream,
            column_alignment: self.column_alignment.clone(),
            cell_index: 0,
            end_tag: Some(tag.to_end()),
            current_component: self.current_component.clone(),
        };
        MdContext::el_fragment(sub_renderer.collect())
    }

    /// extract the text from the next text event
    fn children_text(&mut self, tag: Tag<'a>) -> Option<String> {
        let mut text = "".to_string();
        loop {
            let text_stream = self.stream.next();
            match text_stream {
                Some((Event::Text(s), _)) => text.push_str(&s),
                None => {}
                Some(e) => {
                    assert_eq!(&e.0, &Event::End(tag.to_end()));
                    break;
                }
            }
        }
        // let text = match self.stream.next() {
        //     Some((Event::Text(s), _)) => Some(s.to_string()),
        //     None => None,
        //     _ => panic!("expected string event, got something else"),
        // };

        // self.assert_closing_tag(tag.to_end());
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    // check that the closing tag is what was expected
    // fn assert_closing_tag(&mut self, end: TagEnd) {
    //     let end_tag = &self
    //         .stream
    //         .next()
    //         .expect("this event should be the closing tag")
    //         .0;
    //     assert_eq!(end_tag, &Event::End(end));
    // }

    fn render_tag(&mut self, tag: Tag<'a>) -> Result<Element, HtmlError> {
        Ok(match tag.clone() {
            Tag::HtmlBlock => self.children(tag),
            Tag::Paragraph => MdContext::el(Paragraph, self.children(tag)),
            Tag::Heading { level, .. } => MdContext::el(Heading(level as u8), self.children(tag)),
            Tag::BlockQuote(_) => MdContext::el(BlockQuote, self.children(tag)),
            Tag::CodeBlock(k) => render_code_block(
                &self.props.clone(),
                self.children_text(tag).unwrap_or_default(),
                &k,
            ),
            Tag::List(Some(n0)) => MdContext::el(Ol(n0 as i32), self.children(tag)),
            Tag::List(None) => MdContext::el(Ul, self.children(tag)),
            Tag::Item => MdContext::el(Li, self.children(tag)),
            Tag::Table(align) => {
                self.column_alignment = Some(align);
                MdContext::el(Table, self.children(tag))
            }
            Tag::TableHead => MdContext::el(Thead, self.children(tag)),
            Tag::TableRow => MdContext::el(Trow, self.children(tag)),
            Tag::TableCell => {
                let align = self.column_alignment.clone().unwrap()[self.cell_index];
                self.cell_index += 1;
                MdContext::el_with_attributes(
                    Tcell,
                    self.children(tag),
                    ElementAttributes {
                        style: Some(align_string(align).to_string()),
                        ..Default::default()
                    },
                )
            }
            Tag::Emphasis => MdContext::el(Italics, self.children(tag)),
            Tag::Strong => MdContext::el(Bold, self.children(tag)),
            Tag::Strikethrough => MdContext::el(StrikeThrough, self.children(tag)),
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let description = LinkDescription {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    content: self.children(tag),
                    link_type,
                    image: true,
                };
                MdContext::render_link(&self.props, description).map_err(HtmlError::Link)?
            }
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let description = LinkDescription {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    content: self.children(tag),
                    link_type,
                    image: false,
                };
                MdContext::render_link(&self.props, description).map_err(HtmlError::Link)?
            }
            Tag::FootnoteDefinition(_) => {
                return Err(HtmlError::not_implemented("footnote not implemented"))
            }
            Tag::MetadataBlock { .. } => {
                if let Some(text) = self.children_text(tag) {
                    MdContext::set_frontmatter(&mut self.props, text)
                }
                MdContext::el_empty()
            }
            Tag::DefinitionList => {
                return Err(HtmlError::not_implemented(
                    "definition list not implemented",
                ))
            }
            Tag::DefinitionListTitle => {
                return Err(HtmlError::not_implemented(
                    "definition list not implemented",
                ))
            }
            Tag::DefinitionListDefinition => {
                return Err(HtmlError::not_implemented(
                    "definition list not implemented",
                ))
            }
            Tag::Superscript => {
                return Err(HtmlError::not_implemented("superscript not implemented"))
            }
            Tag::Subscript => return Err(HtmlError::not_implemented("subscript not implemented")),
        })
    }
}
