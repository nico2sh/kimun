use core::ops::Range;

use core::marker::PhantomData;

use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Tag, TagEnd};

#[derive(Eq, PartialEq)]
enum MathMode {
    Inline,
    Display,
}

use super::HtmlElement::*;
use super::{Context, ElementAttributes, HtmlError, LinkDescription};

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

/// `highlight_code(content, ss, ts)` render the content `content`
/// with syntax highlighting
fn highlight_code(theme_name: Option<&str>, content: &str, kind: &CodeBlockKind) -> Option<String> {
    let lang = match kind {
        CodeBlockKind::Fenced(x) => x,
        CodeBlockKind::Indented => return None,
    };

    let theme_name = theme_name.unwrap_or("base16-ocean.light");
    let theme = THEME_SET
        .themes
        .get(theme_name)
        .expect("unknown theme")
        .clone();

    syntect::html::highlighted_html_for_string(
        content,
        &SYNTAX_SET,
        SYNTAX_SET.find_syntax_by_token(lang)?,
        &theme,
    )
    .ok()
}

/// renders a source code in a code block, with syntax highlighting if possible.
/// `cx`: the current markdown context
/// `source`: the source to render
/// `range`: the position of the code in the original source
fn render_code_block<'a, 'callback, F: Context<'a, 'callback>>(
    cx: F,
    source: String,
    k: &CodeBlockKind,
) -> F::View {
    let code_attributes = ElementAttributes {
        ..Default::default()
    };

    match highlight_code(cx.props().theme, &source, k) {
        None => cx.el_with_attributes(Code, cx.el_text(source.into()), code_attributes),
        // None => cx.el_with_attributes(
        //     Code,
        //     cx.el(Code, cx.el_text(source.into())),
        //     code_attributes,
        // ),
        Some(x) => cx.el_span_with_inner_html(x, code_attributes),
    }
}

fn render_maths<'a, 'callback, F: Context<'a, 'callback>>(
    _cx: F,
    _content: &str,
    _display_mode: MathMode,
    _range: Range<usize>,
) -> Result<F::View, HtmlError> {
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
pub struct Renderer<'a, 'callback, 'c, I, F>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
    'callback: 'a,
    F: Context<'a, 'callback>,
{
    __marker: PhantomData<&'callback ()>,
    /// the markdown context
    cx: F,
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

impl<'a, 'callback, I, F> Iterator for Renderer<'a, 'callback, '_, I, F>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
    'callback: 'a,
    F: Context<'a, 'callback>,
{
    type Item = F::View;

    fn next(&mut self) -> Option<Self::Item> {
        use Event::*;
        let (item, range): (Event<'a>, Range<usize>) = self.stream.next()?;
        let range = range.clone();

        let cx = self.cx;

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
            Text(s) => Ok(cx.render_text(s)),
            Code(s) => Ok(cx.render_code(s)),
            InlineHtml(s) => Ok(self
                .cx
                .el_span_with_inner_html(s.to_string(), Default::default())),
            Html(s) => Ok(self
                .cx
                .el_span_with_inner_html(s.to_string(), Default::default())),
            FootnoteReference(_) => Err(HtmlError::not_implemented("footnotes refs")),
            SoftBreak => Ok(cx.el_text(" ".into())),
            HardBreak => Ok(self.cx.el_br()),
            Rule => Ok(cx.render_rule()),
            TaskListMarker(m) => Ok(cx.render_tasklist_marker(m)),
            InlineMath(content) => render_maths(self.cx, &content, MathMode::Inline, range),
            DisplayMath(content) => render_maths(self.cx, &content, MathMode::Display, range),
        };

        Some(rendered.unwrap_or_else(|e| {
            self.cx.el_with_attributes(
                Span,
                self.cx
                    .el_fragment(vec![self.cx.el_text(e.to_string().into()), self.cx.el_br()]),
                ElementAttributes {
                    classes: vec!["markdown-error".to_string()],
                    ..Default::default()
                },
            )
        }))
    }
}

impl<'a, 'callback, 'c, I, F> Renderer<'a, 'callback, 'c, I, F>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
    F: Context<'a, 'callback>,
{
    /// creates a new renderer from a stream of events.
    /// It returns an iterator of [`F::View`]
    pub fn new(cx: F, events: &'c mut I) -> Self {
        Self {
            __marker: PhantomData,
            cx,
            stream: events,
            column_alignment: None,
            cell_index: 0,
            end_tag: None,
            current_component: None,
        }
    }

    /// renders events in a new renderer,
    /// recursively, until the end of the tag
    fn children(&mut self, tag: Tag<'a>) -> F::View {
        let sub_renderer = Renderer {
            __marker: PhantomData,
            cx: self.cx,
            stream: self.stream,
            column_alignment: self.column_alignment.clone(),
            cell_index: 0,
            end_tag: Some(tag.to_end()),
            current_component: self.current_component.clone(),
        };
        self.cx.el_fragment(sub_renderer.collect())
    }

    /// extract the text from the next text event
    fn children_text(&mut self, tag: Tag<'a>) -> Option<String> {
        let text = match self.stream.next() {
            Some((Event::Text(s), _)) => Some(s.to_string()),
            None => None,
            _ => panic!("expected string event, got something else"),
        };

        self.assert_closing_tag(tag.to_end());
        text
    }

    // check that the closing tag is what was expected
    fn assert_closing_tag(&mut self, end: TagEnd) {
        let end_tag = &self
            .stream
            .next()
            .expect("this event should be the closing tag")
            .0;
        assert_eq!(end_tag, &Event::End(end));
    }

    fn render_tag(&mut self, tag: Tag<'a>) -> Result<F::View, HtmlError> {
        let mut cx = self.cx;
        Ok(match tag.clone() {
            Tag::HtmlBlock => self.children(tag),
            Tag::Paragraph => cx.el(Paragraph, self.children(tag)),
            Tag::Heading { level, .. } => cx.el(Heading(level as u8), self.children(tag)),
            Tag::BlockQuote(_) => cx.el(BlockQuote, self.children(tag)),
            Tag::CodeBlock(k) => {
                render_code_block(cx, self.children_text(tag).unwrap_or_default(), &k)
            }
            Tag::List(Some(n0)) => cx.el(Ol(n0 as i32), self.children(tag)),
            Tag::List(None) => cx.el(Ul, self.children(tag)),
            Tag::Item => cx.el(Li, self.children(tag)),
            Tag::Table(align) => {
                self.column_alignment = Some(align);
                cx.el(Table, self.children(tag))
            }
            Tag::TableHead => cx.el(Thead, self.children(tag)),
            Tag::TableRow => cx.el(Trow, self.children(tag)),
            Tag::TableCell => {
                let align = self.column_alignment.clone().unwrap()[self.cell_index];
                self.cell_index += 1;
                cx.el_with_attributes(
                    Tcell,
                    self.children(tag),
                    ElementAttributes {
                        style: Some(align_string(align).to_string()),
                        ..Default::default()
                    },
                )
            }
            Tag::Emphasis => cx.el(Italics, self.children(tag)),
            Tag::Strong => cx.el(Bold, self.children(tag)),
            Tag::Strikethrough => cx.el(StrikeThrough, self.children(tag)),
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
                cx.render_link(description).map_err(HtmlError::Link)?
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
                cx.render_link(description).map_err(HtmlError::Link)?
            }
            Tag::FootnoteDefinition(_) => {
                return Err(HtmlError::not_implemented("footnote not implemented"))
            }
            Tag::MetadataBlock { .. } => {
                if let Some(text) = self.children_text(tag) {
                    cx.set_frontmatter(text)
                }
                cx.el_empty()
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
