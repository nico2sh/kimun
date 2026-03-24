use pulldown_cmark::LinkType;
pub use pulldown_cmark::{CowStr, Options};

use std::{collections::BTreeMap, fmt::Display};

pub mod render;

#[derive(Default)]
pub struct ElementAttributes {
    pub classes: Vec<String>,
    pub style: Option<String>,
}

pub enum HtmlElement {
    Div,
    Span,
    Paragraph,
    BlockQuote,
    Ul,
    Ol(i32),
    Li,
    Heading(u8),
    Table,
    Thead,
    Trow,
    Tcell,
    Italics,
    Bold,
    StrikeThrough,
    Pre,
    Code,
}

pub struct StyleLink {
    pub rel: &'static str,
    pub href: &'static str,
    pub integrity: &'static str,
    pub crossorigin: &'static str,
}

/// the description of a link, used to render it with a custom callback.
/// See [pulldown_cmark::Tag::Link] for documentation
pub struct LinkDescription<V> {
    /// the url of the link
    pub url: String,

    /// the html view of the element under the link
    pub content: V,

    /// the title of the link.
    /// If you don't know what it is, don't worry: it is ofter empty
    pub title: String,

    /// the type of link
    pub link_type: LinkType,

    /// wether the link is an image
    pub image: bool,
}

pub enum HtmlError {
    NotImplemented(String),
    Link(String),
    Syntax(String),
    CustomComponent { name: String, msg: String },
    UnAvailable(String),
    Math,
}

impl Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            HtmlError::Math => "invalid math".to_string(),
            HtmlError::NotImplemented(s) => format!("`{s}`: not implemented"),
            HtmlError::CustomComponent { name, msg } => {
                format!("Custom component `{name}` failed: `{msg}`")
            }
            HtmlError::Syntax(s) => format!("syntax error: {s}"),
            HtmlError::Link(s) => format!("invalid link: {s}"),
            HtmlError::UnAvailable(s) => s.to_string(),
        };
        write!(f, "{}", string)
    }
}

#[derive(PartialEq)]
/// the arguments given to a markdown component.
/// `attributes`: a map of (attribute_name, attribute_value) pairs
/// `children`: the interior markdown of the component
///
/// For example,
/// ```md
/// <MyBox color="blue" size="5">
///
/// **hey !**
///
/// </MyBox>
/// ```
///
/// Will be translated to
/// ```rust
/// MdComponentProps {
///     attributes: BTreeMap::from([("color", "blue"), ("size", "5")]),
///     children: ... // html view of **hey**
/// }
/// ```
pub struct MdComponentProps<V> {
    pub attributes: BTreeMap<String, String>,
    pub children: V,
}

impl<V> MdComponentProps<V> {
    /// returns the attribute string corresponding to the key `name`.
    /// returns None if the attribute was not provided
    pub fn get(&self, name: &str) -> Option<String> {
        self.attributes.get(name).cloned()
    }

    /// returns the attribute corresponding to the key `name`, once parsed.
    /// If the attribute doesn't exist or if the parsing fail, returns an error.
    pub fn get_parsed<T>(&self, name: &str) -> Result<T, String>
    where
        T: std::str::FromStr,
        T::Err: core::fmt::Debug,
    {
        match self.attributes.get(name) {
            Some(x) => x.clone().parse().map_err(|e| format!("{e:?}")),
            None => Err(format!("please provide the attribute `{name}`")),
        }
    }

    /// same thing as `get_parsed`, but if the attribute doesn't exist,
    /// return None
    pub fn get_parsed_optional<T>(&self, name: &str) -> Result<Option<T>, String>
    where
        T: std::str::FromStr,
        T::Err: core::fmt::Debug,
    {
        match self.attributes.get(name) {
            Some(x) => match x.parse() {
                Ok(a) => Ok(Some(a)),
                Err(e) => Err(format!("{e:?}")),
            },
            None => Ok(None),
        }
    }
}

pub struct MarkdownProps {
    pub hard_line_breaks: bool,
    pub wikilinks: bool,
    pub parse_options: Option<pulldown_cmark::Options>,
    pub theme: Option<&'static str>,
}
