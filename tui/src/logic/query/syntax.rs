//! The search query language reference card (F1 over the FIND view and the
//! Ctrl+K browser): operators, modifiers, and a few worked examples — mirrors
//! the canonical table in `docs/.../search.md`, condensed to `(keys, meaning)`
//! pairs. The full prose guide lives on the docs site.

/// One titled section of the reference card: `(heading, [(keys, meaning)])`.
pub type SyntaxSection = (&'static str, &'static [(&'static str, &'static str)]);

/// (short / long, meaning) — sourced from search.md's operator table.
const OPERATORS: &[(&str, &str)] = &[
    ("(type text)", "full-text body search"),
    ("= / name:", "by note name"),
    ("@ / in:", "by section heading"),
    ("/ / pt:", "by path / folder"),
    ("# / lb:", "by label (tag)"),
    ("< / lk:", "links TO it (backlinks)"),
    ("> / fwd:", "it links to (forward)"),
    ("% / prop:", "by property (%due, %k=v)"),
    ("^ / or:", "sort results (-^ = desc)"),
];
const MODIFIERS: &[(&str, &str)] = &[
    ("- prefix", "exclude (e.g. -#draft)"),
    ("*", "wildcard prefix (screen*)"),
    ("\" \"", "quote values with spaces"),
];
const EXAMPLES: &[(&str, &str)] = &[
    ("#finance report", "labelled finance + text"),
    ("@work -cancelled", "Work section, not cancelled"),
    ("<kimun #project", "kimun backlinks + label"),
    ("%status=done ^%due", "done, sorted by due"),
];

/// The reference card, in display order.
pub const SYNTAX: &[SyntaxSection] = &[
    ("Operators", OPERATORS),
    ("Modifiers", MODIFIERS),
    ("Examples", EXAMPLES),
];
