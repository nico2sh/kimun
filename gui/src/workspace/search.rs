//! The **search browser** (M7b G4): the `SearchNotes` chord (Ctrl+K /
//! Cmd+K), leader `f g` / `f r`, the palette, and a followed `#tag` open the
//! one note picker ([`super::picker`]) over the vault's query search, with a
//! preview of the selected note beside the results (below them in a narrow
//! window).
//!
//! **Rows.** The logic's search source (`resolving_search_source`): an empty
//! query lists the recent notes, anything else runs core's query search,
//! `{note}` and the bare `<` / `>` resolving against the open note. There is
//! no `Create:` row: a query is not a note name (the finder and the FILES
//! filter offer it). Loads run off the UI thread and the newest query wins
//! (the list core's generation guard). The selection starts on the first
//! match (Enter opens the best result).
//!
//! **Preview** ([`SearchPreview`]). The selected note's text, the query's
//! needles in the match colour. A selection held for [`PREVIEW_DEBOUNCE`]
//! is read on tokio, capped at [`PREVIEW_BYTES`], split into lines,
//! matched and soft-wrapped to the pane's width there too; the UI thread only
//! draws the rows on screen. The width is measured in monospace cells after
//! layout; when it changes (narrow or wide layout, a resize) the lines
//! already read are wrapped again on tokio, the note is not read again. A newer
//! selection cancels the wait or the read and its result is never shown;
//! until the new one lands the old preview stays, dimmed. Nothing is shown
//! for no selection. The preview is a cached view: typing or
//! moving the selection does not rebuild it until its own state changes.
//!
//! **Images.** An embed line (an **Inline image**, CONTEXT.md) is followed
//! by fixed-height image rows, so the list stays uniform; it keeps its raw
//! text above them only when it holds a match (the Source reader: when it
//! lies in the marked section). The read waits for the vault's shared
//! attachment list ([`AttachmentIndex`]) and reads each image's header with
//! the note; a re-wrap at another width or height reads no file.
//!
//! **Opening.** Enter (or a double-click: a click only selects, the
//! preview follows) on a note opens it with the
//! query's needles emphasised in the editor (cleared by the first edit).
//! With unsaved edits in the open note the browser stays up while they are
//! written; a write that fails keeps the browser open and says why (the
//! note stays, edits kept).
//!
//! **Query syntax.** `F1` or the `?` button shows the reference card
//! (`logic::query::SYNTAX`) in a kit dialog over the browser: the one
//! stacked modal. Escape closes it and the browser has the keyboard again.
//!
//! The browser only reads; every write (the flush) is core's.
//!
//! **The Source reader** (M7c G10b) is this preview with a [`Mark::Section`]
//! instead of needles: the note a source came from, the lines of its
//! retrieved section (`ask::locate::section_range`, resolved on tokio once
//! the note is read) in the accent colour, scrolled to the top of the pane.
//! Another section of the note already shown is laid out again from the text
//! it read; another note says `loading…` until it lands, `failed to load
//! note` if it cannot be read (the TUI's words).

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, ScrollEvent, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Task, UniformListScrollHandle,
    Window, canvas, div, font, px, rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::locate;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::preview::{split_ranges, wrap_ranges};
use kimun_notes::logic::query::SYNTAX;
use kimun_notes::logic::session::FlushResult;
use kimun_notes::logic::text_match::match_ranges;
use kimun_notes::settings::themes::Theme;
use tokio::task::JoinError;

use super::Workspace;
use super::attachment::expand_tabs;
use super::files::ellipsis;
use super::modal::{self, ModalKind, Slot};
use super::note::{Closing, flush_problem};
use crate::attachment_index::AttachmentIndex;
use crate::grid::CellMetrics;
use crate::image_core::{Embed, ImagePane, embeds_of, natural_of};
use crate::inline_image::{Failure, ImageState, ImageStore, fit, rows_for};
use crate::theme;

/// Element ids, for tests.
pub(crate) const PREVIEW_ID: &str = "search-preview";
pub(crate) const PREVIEW_ROW_ID: &str = "search-preview-row";
pub(crate) const PREVIEW_TRUNCATED_ID: &str = "search-preview-truncated";
pub(crate) const QUERY_HELP_ID: &str = "query-help";

/// How long a selection must hold before its note is read for the preview:
/// arrowing through the results reads nothing until it stops.
pub(crate) const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);
/// The most of a note the preview reads into lines; the rest is cut and
/// said so (a huge note would cost memory and time for lines nobody scrolls
/// to).
pub(crate) const PREVIEW_BYTES: usize = 256 * 1024;
/// One preview row's height, in rems.
const LINE_REMS: f32 = 1.375;
/// The columns a note is wrapped to before the pane has been measured (the
/// first frame); the measured width re-wraps it.
const FALLBACK_COLUMNS: usize = 80;
/// The room images are fitted to before the pane has been measured; the
/// measured one re-wraps a note with images.
const FALLBACK_PANE: ImagePane = ImagePane {
    column_px: 640.0,
    pane_px: 600.0,
    row_px: 22.0,
};
/// What the Source reader says while its note is read, and when it cannot
/// be (the TUI's words).
pub(crate) const READER_LOADING: &str = "loading…";
pub(crate) const READER_FAILED: &str = "failed to load note";

/// What a preview marks in its note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Every match of these needles (the search browser, FIND).
    Needles(Arc<[String]>),
    /// One source's retrieved section (the Source reader).
    Section(SectionMark),
}

/// A source's section, as `ask::locate::section_range` finds it in the
/// note: the source's ordinal (its identity within the note), its innermost
/// heading and its chunk text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SectionMark {
    pub(crate) ordinal: usize,
    pub(crate) heading: Arc<str>,
    pub(crate) chunk: Arc<str>,
}

// ---- What a preview load delivers --------------------------------------------

/// One line of the note (tabs widened), with where the needles matched in it.
struct NoteLine {
    text: String,
    hits: Vec<(usize, usize)>,
    /// The line's bytes in the note as read (before the tabs widen).
    span: Range<usize>,
}

/// What a preview row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// A piece of the note line's text.
    Text,
    /// Slice `index` (one row high) of image `image` of [`Wrapped::images`].
    Image { image: usize, index: usize },
    /// The one-line note of image `image`, which cannot be drawn.
    Note { image: usize },
}

/// One wrapped row as the preview draws it: a piece of a note line, or one
/// row of an image under its embed line.
struct PreviewRow {
    /// The note line it is a piece of, and the piece's first byte there.
    /// An image row sorts after its line's text: its `start` is the line's
    /// length plus its index plus one.
    line: usize,
    start: usize,
    /// Empty for an image row or a note row.
    text: SharedString,
    /// The needle matches in this piece (a match across a row edge is cut).
    hits: Vec<Range<usize>>,
    kind: RowKind,
}

/// An image the rows draw (an **Inline image**, CONTEXT.md).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreviewImage {
    /// The attachment's file, when the embed resolved.
    pub(crate) file: Option<PathBuf>,
    /// The file name as written, and the alt text.
    pub(crate) name: String,
    pub(crate) alt: String,
    /// The size it is drawn at, in device pixels.
    pub(crate) fitted: (f32, f32),
    /// Its image rows (0 for one that cannot be drawn), the first of them.
    pub(crate) rows: usize,
    pub(crate) first_row: usize,
    /// Why it cannot be drawn: its one row is a [`RowKind::Note`].
    pub(crate) failed: Option<Failure>,
}

/// Which embed lines keep their raw text above their image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RawRule {
    /// A line holding a needle match (the Preview pane).
    Hits,
    /// A line inside this byte range of the note: the marked section (the
    /// Source reader).
    Section(Range<usize>),
}

impl RawRule {
    fn shows_raw(&self, line: &NoteLine) -> bool {
        match self {
            RawRule::Hits => !line.hits.is_empty(),
            RawRule::Section(range) => line.span.start < range.end && range.start < line.span.end,
        }
    }
}

/// What a wrap needs to lay out a note's images, found once as the note is
/// read: a re-wrap at another width or height reads no file.
#[derive(Clone, Debug)]
pub(crate) struct ImagePlan {
    /// The embeds of the (capped) text, at most [`PREVIEW_EMBEDS`].
    pub(crate) embeds: Vec<Embed>,
    /// Each embed's natural size from its file's header; `None` when it
    /// cannot be drawn, and then why in `failures`.
    pub(crate) naturals: Vec<Option<(u32, u32)>>,
    pub(crate) failures: Vec<Option<Failure>>,
    /// The room the images are fitted to.
    pub(crate) pane: ImagePane,
    pub(crate) raw: RawRule,
}

impl ImagePlan {
    /// No images: the rows are the text's alone.
    fn none(pane: ImagePane) -> Self {
        Self {
            embeds: Vec::new(),
            naturals: Vec::new(),
            failures: Vec::new(),
            pane,
            raw: RawRule::Hits,
        }
    }
}

/// The note's lines wrapped to a column count.
struct Wrapped {
    columns: usize,
    /// The room its images are fitted to.
    pane: ImagePane,
    rows: Vec<PreviewRow>,
    /// The row holding the first match, scrolled to when the preview lands.
    first_hit: Option<usize>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the image rows are painted from the next task")
    )]
    images: Vec<PreviewImage>,
}

/// `lines` wrapped to `columns`, each embed line of `images` followed by
/// its image's rows (or the one note row of an image that cannot be drawn)
/// and keeping its own text rows only where `images.raw` says; the row of
/// `first_match` (line, byte) found. Pure; runs on tokio.
fn wrap_lines(
    lines: &[NoteLine],
    columns: usize,
    first_match: Option<(usize, usize)>,
    images: &ImagePlan,
) -> Wrapped {
    let mut rows = Vec::with_capacity(lines.len());
    let mut drawn = Vec::new();
    // The embeds come in line order.
    let mut embeds = images.embeds.iter().enumerate().peekable();
    for (i, line) in lines.iter().enumerate() {
        while embeds.next_if(|(_, embed)| embed.row < i).is_some() {}
        let Some((e, embed)) = embeds.next_if(|(_, embed)| embed.row == i) else {
            push_text_rows(&mut rows, i, line, columns);
            continue;
        };
        let raw = images.raw.shows_raw(line);
        if raw {
            push_text_rows(&mut rows, i, line, columns);
        }
        let image = drawn.len();
        let mut drawn_image = PreviewImage {
            file: embed.file.clone(),
            name: embed.name.clone(),
            alt: embed.alt.clone(),
            fitted: (0.0, 0.0),
            rows: 0,
            first_row: rows.len(),
            failed: None,
        };
        match images.naturals.get(e).copied().flatten() {
            Some(natural) => {
                let pane = images.pane;
                drawn_image.fitted = fit(natural, pane.column_px, pane.pane_px);
                drawn_image.rows = rows_for(drawn_image.fitted.1, pane.row_px);
                let end = line.text.len();
                rows.extend((0..drawn_image.rows).map(|index| PreviewRow {
                    line: i,
                    start: end + index + 1,
                    text: SharedString::default(),
                    hits: Vec::new(),
                    kind: RowKind::Image { image, index },
                }));
            }
            // The raw text says it all: nothing is added.
            None if raw => continue,
            None => {
                let failure = images.failures.get(e).copied().flatten();
                drawn_image.failed = Some(failure.unwrap_or(Failure::Missing));
                rows.push(PreviewRow {
                    line: i,
                    start: 0,
                    text: SharedString::default(),
                    hits: Vec::new(),
                    kind: RowKind::Note { image },
                });
            }
        }
        drawn.push(drawn_image);
    }
    let first_hit = first_match.map(|(line, at)| row_of(&rows, line, at));
    Wrapped {
        columns,
        pane: images.pane,
        rows,
        first_hit,
        images: drawn,
    }
}

/// The text rows of note line `i`, wrapped to `columns`.
fn push_text_rows(rows: &mut Vec<PreviewRow>, i: usize, line: &NoteLine, columns: usize) {
    for range in wrap_ranges(&line.text, columns) {
        let hits = split_ranges(&line.hits, range.clone())
            .into_iter()
            .map(|(start, end)| start..end)
            .collect();
        rows.push(PreviewRow {
            line: i,
            start: range.start,
            text: line.text[range].to_string().into(),
            hits,
            kind: RowKind::Text,
        });
    }
}

/// The row showing byte `at` of note line `line`.
fn row_of(rows: &[PreviewRow], line: usize, at: usize) -> usize {
    rows.partition_point(|r| (r.line, r.start) <= (line, at))
        .saturating_sub(1)
}

/// A note read for the preview: its lines and their wrapped rows, made off
/// the UI thread.
pub(crate) struct PreviewText {
    path: VaultPath,
    name: SharedString,
    /// Kept to re-wrap when the pane's width changes (no re-read).
    lines: Arc<[NoteLine]>,
    /// Kept to re-wrap the images when the pane's width or height changes
    /// (no file read).
    plan: ImagePlan,
    wrapped: Wrapped,
    /// How many needle matches the lines hold.
    matches: usize,
    /// The first match's line and byte.
    first_match: Option<(usize, usize)>,
    /// The note was longer than [`PREVIEW_BYTES`].
    truncated: bool,
    /// The reader's: the section's byte range in the (capped) text, when
    /// it resolved, and that text, kept so another section of this note is
    /// laid out without reading it again. `None` for needles.
    #[cfg(test)]
    section: Option<Range<usize>>,
    source: Option<Arc<str>>,
}

impl PreviewText {
    /// Whether the rows are wrapped to `columns` and, for a note with
    /// images, fitted to `pane` (a note without any ignores the height).
    fn wrapped_for(&self, columns: usize, pane: ImagePane) -> bool {
        self.wrapped.columns == columns
            && (self.plan.embeds.is_empty() || self.wrapped.pane == pane)
    }
}

#[cfg(test)]
impl PreviewText {
    /// Each wrapped row's text and its marked pieces.
    pub(crate) fn lines_and_hits(&self) -> Vec<(String, Vec<String>)> {
        self.wrapped
            .rows
            .iter()
            .map(|r| {
                let hits = r.hits.iter().map(|h| r.text[h.clone()].to_string());
                (r.text.to_string(), hits.collect())
            })
            .collect()
    }

    /// The match count, the first matching row, whether it was cut.
    pub(crate) fn summary(&self) -> (usize, Option<usize>, bool) {
        (self.matches, self.wrapped.first_hit, self.truncated)
    }

    /// The columns the rows are wrapped to.
    pub(crate) fn columns(&self) -> usize {
        self.wrapped.columns
    }

    /// The reader's resolved section, as a byte range of the note.
    pub(crate) fn section(&self) -> Option<Range<usize>> {
        self.section.clone()
    }

    /// What each wrapped row shows.
    pub(crate) fn kinds(&self) -> Vec<RowKind> {
        self.wrapped.rows.iter().map(|r| r.kind).collect()
    }

    /// The note line each wrapped row belongs to.
    pub(crate) fn row_lines(&self) -> Vec<usize> {
        self.wrapped.rows.iter().map(|r| r.line).collect()
    }

    /// The images the rows draw.
    pub(crate) fn images(&self) -> &[PreviewImage] {
        &self.wrapped.images
    }

    /// The row showing byte `at` of note line `line`.
    pub(crate) fn row_at(&self, line: usize, at: usize) -> usize {
        row_of(&self.wrapped.rows, line, at)
    }

    /// Wrap again to `columns` and `pane`, as the preview's re-wrap does.
    pub(crate) fn rewrap_at(&mut self, columns: usize, pane: ImagePane) {
        let mut plan = self.plan.clone();
        plan.pane = pane;
        self.wrapped = wrap_lines(&self.lines, columns, self.first_match, &plan);
        self.plan = plan;
    }
}

/// The most embeds of one note the preview draws.
const PREVIEW_EMBEDS: usize = 100;

#[cfg(test)]
thread_local! {
    /// Image headers read on this thread (tests: a re-wrap reads none).
    static HEADER_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Image headers read on this thread so far (tests).
#[cfg(test)]
pub(crate) fn header_reads() -> usize {
    HEADER_READS.with(std::cell::Cell::get)
}

/// The images of `text` (the capped note at `note`) for the preview: its
/// first [`PREVIEW_EMBEDS`] embeds resolved against `found` (the vault and
/// its attachments; `None`: no images), and each one's header read. Runs
/// on tokio, as the note is read.
fn plan_images(
    text: &str,
    note: &VaultPath,
    found: Option<(&NoteVault, &[VaultPath])>,
    pane: ImagePane,
) -> ImagePlan {
    let Some((vault, attachments)) = found else {
        return ImagePlan::none(pane);
    };
    let embeds = embeds_of(text, note, attachments, vault, Some(PREVIEW_EMBEDS));
    if embeds.is_empty() {
        return ImagePlan::none(pane);
    }
    // A store of this read's own: the headers only, nothing decoded (the
    // app's store is the UI thread's).
    let mut store = ImageStore::default();
    let (naturals, failures) = embeds
        .iter()
        .map(|embed| header_of(embed, &mut store))
        .unzip();
    ImagePlan {
        embeds,
        naturals,
        failures,
        pane,
        raw: RawRule::Hits,
    }
}

/// The natural size of the embed's image from its file's header, or why it
/// cannot be drawn.
fn header_of(embed: &Embed, store: &mut ImageStore) -> (Option<(u32, u32)>, Option<Failure>) {
    #[cfg(test)]
    HEADER_READS.with(|reads| reads.set(reads.get() + 1));
    if let Some(natural) = natural_of(embed, store) {
        return (Some(natural), None);
    }
    let failure = match embed.file.as_ref().map(|file| store.size_of(file)) {
        Some(ImageState::Failed(failure)) => failure,
        // Not resolved to an attachment.
        _ => Failure::Missing,
    };
    (None, Some(failure))
}

/// `text` cut to at most `max` bytes on a character boundary, and whether
/// it was cut.
pub(crate) fn cap_text(mut text: String, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

/// Lay out `text` (the note at `path`) for the preview: capped, split into
/// lines (tabs widened), each matched against `needles`, wrapped to
/// `columns`. Pure; runs on tokio.
#[cfg(test)]
pub(crate) fn preview_text(
    path: VaultPath,
    text: String,
    needles: &[String],
    columns: usize,
) -> PreviewText {
    let mark = Mark::Needles(needles.into());
    marked_text(path, text, &mark, columns, FALLBACK_PANE, None)
}

/// Lay out `text` for the preview with any [`Mark`]: capped, split into
/// lines, the needles or the reader's section (resolved in the capped
/// text) marked, its images found (from `found`, the vault and its
/// attachments; `None`: none) and fitted to `pane`, wrapped to `columns`.
/// Reads the images' headers; runs on tokio.
pub(crate) fn marked_text(
    path: VaultPath,
    text: String,
    mark: &Mark,
    columns: usize,
    pane: ImagePane,
    found: Option<(&NoteVault, &[VaultPath])>,
) -> PreviewText {
    let (text, truncated) = cap_text(text, PREVIEW_BYTES);
    let plan = plan_images(&text, &path, found, pane);
    lay_out(path, text.into(), mark, truncated, columns, plan)
}

/// `text` split into lines (tabs widened) with `mark`'s hits, wrapped to
/// `columns` with the images of `plan`. A section marks each line it
/// overlaps whole (the TUI's range highlight); its first line is the one
/// scrolled to. The mark sets which embed lines keep their raw text: one
/// with a needle match, or one in the section. Pure.
fn lay_out(
    path: VaultPath,
    text: Arc<str>,
    mark: &Mark,
    truncated: bool,
    columns: usize,
    mut plan: ImagePlan,
) -> PreviewText {
    let (needles, section): (&[String], _) = match mark {
        Mark::Needles(needles) => (needles, None),
        Mark::Section(s) => (&[], locate::section_range(&text, &s.heading, &s.chunk)),
    };
    let mut matches = 0;
    let mut first_match = None;
    let mut offset = 0;
    // `str::lines` with each line's byte span.
    let lines: Arc<[NoteLine]> = text
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, raw)| {
            let line = match raw.strip_suffix('\n') {
                Some(line) => line.strip_suffix('\r').unwrap_or(line),
                None => raw,
            };
            let span = offset..offset + line.len();
            offset += raw.len();
            let text = expand_tabs(line);
            let hits = match &section {
                Some(range) if span.start < range.end && range.start < span.end => {
                    vec![(0, text.len())]
                }
                Some(_) => Vec::new(),
                None if needles.is_empty() => Vec::new(),
                None => match_ranges(&text, needles),
            };
            if let Some(&(start, _)) = hits.first() {
                matches += hits.len();
                first_match.get_or_insert((i, start));
            }
            NoteLine { text, hits, span }
        })
        .collect();
    plan.raw = match (mark, &section) {
        (Mark::Needles(_), _) => RawRule::Hits,
        (Mark::Section(_), Some(range)) => RawRule::Section(range.clone()),
        // No section resolved: no line is in it.
        (Mark::Section(_), None) => RawRule::Section(0..0),
    };
    let wrapped = wrap_lines(&lines, columns, first_match, &plan);
    let reader = matches!(mark, Mark::Section(_));
    PreviewText {
        name: path.get_parent_path().1.into(),
        path,
        lines,
        plan,
        wrapped,
        matches,
        first_match,
        truncated,
        #[cfg(test)]
        section,
        source: reader.then_some(text),
    }
}

/// Read and lay out the note at `path` for the preview (on tokio), its
/// images resolved against `attachments` (the vault's shared list, taken
/// before the read started; `None`: no images). Every load reads each
/// embed's header again with its own store, because the app's store lives
/// on the UI thread: moving through the results rereads up to 100 headers
/// per note.
async fn load_preview(
    vault: Arc<NoteVault>,
    path: VaultPath,
    mark: Mark,
    columns: usize,
    pane: ImagePane,
    attachments: Option<Arc<Vec<VaultPath>>>,
) -> Result<PreviewText, String> {
    let text = vault
        .get_note_text(&path)
        .await
        .map_err(|e| e.to_string())?;
    let found = attachments
        .as_deref()
        .map(|list| (&*vault, list.as_slice()));
    Ok(marked_text(path, text, &mark, columns, pane, found))
}

/// Where a note that landed is scrolled to: the first match centred, the
/// reader's section at the top of the pane.
fn anchor_strategy(text: &PreviewText) -> ScrollStrategy {
    if text.source.is_some() {
        ScrollStrategy::Top
    } else {
        ScrollStrategy::Center
    }
}

/// Aborts a tokio task when dropped: a superseded preview read stops with
/// the GPUI task that awaits it.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ---- The preview -------------------------------------------------------------

/// What the preview should show: a note and what to mark in it.
type Wanted = (VaultPath, Mark);

/// The search browser's preview column. See the module docs.
pub(crate) struct SearchPreview {
    vault: Arc<NoteVault>,
    /// The pane's element id and its rows' (the search browser's, or FIND's).
    ids: (&'static str, &'static str),
    /// The selection's note and needles, `None` for no note.
    wanted: Option<Wanted>,
    /// What is on screen (possibly an older selection's, while `pending`).
    shown: Option<PreviewText>,
    /// Why the wanted note could not be read.
    error: Option<SharedString>,
    /// The debounce, then the read, for `wanted`; replaced by a newer one.
    load: Option<Task<()>>,
    /// The pane's width in monospace cells, measured each layout; 0 until
    /// the first.
    columns: usize,
    /// The room the pane gives an image (its width and height in device
    /// pixels, a row's height), measured with `columns`; `None` until the
    /// first layout.
    pane: Option<ImagePane>,
    /// Bumped by every new `wanted`: a read started for an older one (the
    /// attachment list landing late) is not started.
    generation: u64,
    /// The shown note being wrapped again to a new width or height;
    /// replaced by a newer one.
    rewrap: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    /// The reader scrolled the shown note: a re-wrap keeps their place
    /// rather than the first match.
    scrolled: bool,
    /// Redraws the pane when the theme changes: it is drawn cached, so its
    /// parent's redraw does not reach it.
    _theme: gpui_kit::Subscription,
    /// Reads started (tests: the debounce reads nothing while arrowing).
    #[cfg(test)]
    pub(crate) reads: usize,
    /// The notes whose read actually started, in order (tests).
    #[cfg(test)]
    pub(crate) read_paths: Vec<VaultPath>,
}

impl SearchPreview {
    pub(crate) fn new(vault: Arc<NoteVault>, cx: &mut Context<Self>) -> Self {
        Self {
            vault,
            ids: (PREVIEW_ID, PREVIEW_ROW_ID),
            wanted: None,
            shown: None,
            error: None,
            load: None,
            columns: 0,
            pane: None,
            generation: 0,
            rewrap: None,
            scroll: UniformListScrollHandle::new(),
            scrolled: false,
            _theme: theme::observe(cx),
            #[cfg(test)]
            reads: 0,
            #[cfg(test)]
            read_paths: Vec::new(),
        }
    }

    /// The preview with its own element ids (`pane`, and `rows` for each
    /// row): FIND's, which may show beside the search browser's.
    pub(crate) fn with_ids(mut self, pane: &'static str, rows: &'static str) -> Self {
        self.ids = (pane, rows);
        self
    }

    /// Scroll the shown note by `rows` wrapped rows (negative: up), as the
    /// wheel would; the reader's place is kept on a re-wrap.
    pub(crate) fn scroll_rows(&mut self, rows: i32, cx: &mut Context<Self>) {
        let Some(count) = self.shown.as_ref().map(|s| s.wrapped.rows.len()) else {
            return;
        };
        let state = self.scroll.0.borrow();
        let Some(size) = state.last_item_size else {
            return;
        };
        if count == 0 || size.contents.height <= px(0.) {
            return;
        }
        let row = size.contents.height / count as f32;
        let handle = &state.base_handle;
        let mut offset = handle.offset();
        let lowest = -handle.max_offset().y;
        offset.y = (offset.y - row * rows as f32).min(px(0.)).max(lowest);
        handle.set_offset(offset);
        drop(state);
        self.scrolled = true;
        cx.notify();
    }

    /// The first wrapped row in view (tests).
    #[cfg(test)]
    pub(crate) fn top_row(&self) -> usize {
        let state = self.scroll.0.borrow();
        let count = self.shown.as_ref().map_or(0, |s| s.wrapped.rows.len());
        match state.last_item_size {
            Some(size) if count > 0 && size.contents.height > px(0.) => {
                let row = size.contents.height / count as f32;
                (-state.base_handle.offset().y / row).round().max(0.) as usize
            }
            _ => 0,
        }
    }

    /// The path of the note shown, and whether it is the wanted one (tests).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<(VaultPath, bool)> {
        self.shown
            .as_ref()
            .map(|s| (s.path.clone(), !self.is_pending()))
    }

    /// The shown rows' text and their highlighted pieces (tests).
    #[cfg(test)]
    pub(crate) fn shown_lines(&self) -> Vec<(String, Vec<String>)> {
        self.shown
            .as_ref()
            .map(PreviewText::lines_and_hits)
            .unwrap_or_default()
    }

    /// The note line each shown row is a piece of (tests).
    #[cfg(test)]
    pub(crate) fn row_lines(&self) -> Vec<usize> {
        self.shown
            .as_ref()
            .map(PreviewText::row_lines)
            .unwrap_or_default()
    }

    /// What each shown row shows (tests).
    #[cfg(test)]
    pub(crate) fn shown_kinds(&self) -> Vec<RowKind> {
        self.shown
            .as_ref()
            .map(PreviewText::kinds)
            .unwrap_or_default()
    }

    /// The shown note's images (tests).
    #[cfg(test)]
    pub(crate) fn shown_images(&self) -> Vec<PreviewImage> {
        self.shown
            .as_ref()
            .map(|s| s.images().to_vec())
            .unwrap_or_default()
    }

    /// The room the shown rows' images are fitted to, and the one the pane
    /// measured (tests).
    #[cfg(test)]
    pub(crate) fn panes(&self) -> (Option<ImagePane>, Option<ImagePane>) {
        (self.shown.as_ref().map(|s| s.wrapped.pane), self.pane)
    }

    /// The shown note's first matching row, the columns it is wrapped to
    /// and the columns the pane measured (tests).
    #[cfg(test)]
    pub(crate) fn wrap_state(&self) -> (Option<usize>, Option<usize>, usize) {
        let shown = self.shown.as_ref();
        (
            shown.and_then(|s| s.wrapped.first_hit),
            shown.map(PreviewText::columns),
            self.columns,
        )
    }

    /// Whether the note on screen is not (yet) the wanted one, or is still
    /// being wrapped to the pane's width (tests).
    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        match (&self.wanted, &self.shown) {
            (None, _) => false,
            (Some((path, _)), Some(shown)) => {
                self.load.is_some() || &shown.path != path || self.rewrap.is_some()
            }
            (Some(_), None) => self.error.is_none(),
        }
    }

    /// The columns to wrap to: the measured width, or a guess before it.
    fn wrap_columns(&self) -> usize {
        match self.columns {
            0 => FALLBACK_COLUMNS,
            n => n,
        }
    }

    /// The room to fit images to: the measured one, or a guess before it.
    fn wrap_pane(&self) -> ImagePane {
        self.pane.unwrap_or(FALLBACK_PANE)
    }

    /// Show `path` (the selected note; `None`: nothing) with `needles`
    /// marked. The same note and needles again change nothing; anything else
    /// waits [`PREVIEW_DEBOUNCE`], then reads off the UI thread, replacing
    /// any wait or read still under way.
    pub(crate) fn show(
        &mut self,
        path: Option<VaultPath>,
        needles: Arc<[String]>,
        cx: &mut Context<Self>,
    ) {
        self.show_marked(path, Mark::Needles(needles), cx);
    }

    /// [`Self::show`] with any [`Mark`] (the Source reader's section). For a
    /// section of the note already shown, its text is laid out again on
    /// tokio without reading it; a section of another note clears the pane
    /// (`loading…`) until that note lands.
    pub(crate) fn show_marked(
        &mut self,
        path: Option<VaultPath>,
        mark: Mark,
        cx: &mut Context<Self>,
    ) {
        let wanted = path.map(|p| (p, mark));
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted.clone();
        self.error = None;
        self.generation += 1;
        let Some((path, mark)) = wanted else {
            self.load = None;
            self.rewrap = None;
            self.shown = None;
            cx.notify();
            return;
        };
        if let Mark::Section(_) = &mark {
            let cached = self
                .shown
                .as_ref()
                .filter(|shown| shown.path == path)
                .and_then(|shown| {
                    Some((shown.source.clone()?, shown.truncated, shown.plan.clone()))
                });
            if let Some((text, truncated, mut plan)) = cached {
                let columns = self.wrap_columns();
                // The same note's images: their headers are not read again.
                plan.pane = self.wrap_pane();
                self.load = Some(cx.spawn(async move |this, cx| {
                    let job = {
                        let (path, mark) = (path.clone(), mark.clone());
                        tokio::spawn(async move {
                            Ok(lay_out(path, text, &mark, truncated, columns, plan))
                        })
                    };
                    let mut job = AbortOnDrop(job);
                    let laid_out = (&mut job.0).await;
                    this.update(cx, |preview, cx| preview.loaded((path, mark), laid_out, cx))
                        .ok();
                }));
                cx.notify();
                return;
            }
            // Another note: none of the old one stays up.
            self.shown = None;
            self.rewrap = None;
        }
        let generation = self.generation;
        self.load = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
            this.update(cx, |preview, cx| preview.read_when_listed(generation, cx))
                .ok();
        }));
        cx.notify();
    }

    /// The debounce ran out: read the wanted note once the vault's
    /// attachment list is there, so its images resolve without the read
    /// walking the vault. Always through `request`, never `get` alone: a
    /// fresh list answers on the next tick, a refreshed (stale) one only
    /// once it is walked again, so a file added since is found.
    fn read_when_listed(&mut self, generation: u64, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let mut index = cx.default_global::<AttachmentIndex>().clone();
        let preview = cx.weak_entity();
        let vault = self.vault.clone();
        index.request(&vault, cx, move |cx| {
            preview
                .update(cx, |preview, cx| preview.read(generation, cx))
                .ok();
        });
    }

    /// The debounce of the wanted note ran out now (tests: no timer).
    #[cfg(test)]
    pub(crate) fn debounce_ended(&mut self, cx: &mut Context<Self>) {
        self.read_when_listed(self.generation, cx);
    }

    /// Read the wanted note on tokio, unless another one is wanted since
    /// `generation`.
    fn read(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        let Some((path, mark)) = self.wanted.clone() else {
            return;
        };
        #[cfg(test)]
        self.read_paths.push(path.clone());
        let attachments = cx.default_global::<AttachmentIndex>().get(&self.vault);
        // The width and height as of now: a later change re-wraps what lands.
        let (columns, pane) = (self.wrap_columns(), self.wrap_pane());
        let vault = self.vault.clone();
        self.load = Some(cx.spawn(async move |this, cx| {
            let mut read = AbortOnDrop(tokio::spawn(load_preview(
                vault,
                path.clone(),
                mark.clone(),
                columns,
                pane,
                attachments,
            )));
            let loaded = (&mut read.0).await;
            this.update(cx, |preview, cx| preview.loaded((path, mark), loaded, cx))
                .ok();
        }));
    }

    /// Whether the preview marks a source's section (the Source reader).
    fn is_reader(&self) -> bool {
        matches!(self.wanted, Some((_, Mark::Section(_))))
    }

    /// The reader's resolved section in the note shown, if any (tests).
    #[cfg(test)]
    pub(crate) fn shown_section(&self) -> Option<Range<usize>> {
        self.shown.as_ref().and_then(PreviewText::section)
    }

    /// What the pane says instead of a note, if anything (tests).
    #[cfg(test)]
    pub(crate) fn notice(&self) -> Option<String> {
        match (&self.shown, &self.error) {
            (_, Some(error)) => Some(error.to_string()),
            (None, None) if self.is_reader() => Some(READER_LOADING.to_string()),
            _ => None,
        }
    }

    /// A read finished: show it if it is still the wanted note's.
    fn loaded(
        &mut self,
        wanted: Wanted,
        loaded: Result<Result<PreviewText, String>, JoinError>,
        cx: &mut Context<Self>,
    ) {
        if self.wanted.as_ref() != Some(&wanted) {
            return;
        }
        self.load = None;
        match loaded.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(text) => {
                let to = text.wrapped.first_hit.unwrap_or(0);
                self.scroll.scroll_to_item(to, anchor_strategy(&text));
                self.scrolled = false;
                self.shown = Some(text);
                // The pane changed width during the read.
                self.rewrap(cx);
            }
            Err(e) => {
                self.shown = None;
                self.rewrap = None;
                self.error = Some(match wanted.1 {
                    Mark::Section(_) => READER_FAILED.into(),
                    Mark::Needles(_) => format!("Couldn't read {}: {e}", wanted.0).into(),
                });
            }
        }
        cx.notify();
    }

    /// The pane measured `columns` cells wide with room `pane` for images:
    /// re-wrap the shown note when that differs from what it is wrapped to.
    fn set_measure(&mut self, columns: usize, pane: ImagePane, cx: &mut Context<Self>) {
        if columns == self.columns && Some(pane) == self.pane {
            return;
        }
        self.columns = columns;
        self.pane = Some(pane);
        self.rewrap(cx);
        cx.notify();
    }

    /// Wrap the shown note's lines again to the pane's width and height, on
    /// tokio (the note and its images' files are not read again). The latest
    /// size wins.
    fn rewrap(&mut self, cx: &mut Context<Self>) {
        let (columns, pane) = (self.wrap_columns(), self.wrap_pane());
        let Some(shown) = self
            .shown
            .as_ref()
            .filter(|s| !s.wrapped_for(columns, pane))
        else {
            self.rewrap = None;
            return;
        };
        let lines = shown.lines.clone();
        let first_match = shown.first_match;
        let mut plan = shown.plan.clone();
        plan.pane = pane;
        self.rewrap = Some(cx.spawn(async move |this, cx| {
            let job = {
                let lines = lines.clone();
                tokio::spawn(async move { wrap_lines(&lines, columns, first_match, &plan) })
            };
            let mut job = AbortOnDrop(job);
            let wrapped = (&mut job.0).await;
            this.update(cx, |preview, cx| preview.rewrapped(&lines, wrapped, cx))
                .ok();
        }));
    }

    /// A re-wrap finished: show it if it is still the shown note at the
    /// pane's width, with the same text in view.
    fn rewrapped(
        &mut self,
        lines: &Arc<[NoteLine]>,
        wrapped: Result<Wrapped, JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Ok(wrapped) = wrapped else {
            return;
        };
        let current = self
            .shown
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(&s.lines, lines));
        let images = self
            .shown
            .as_ref()
            .is_some_and(|s| !s.plan.embeds.is_empty());
        if !current
            || wrapped.columns != self.wrap_columns()
            || (images && wrapped.pane != self.wrap_pane())
        {
            return;
        }
        self.rewrap = None;
        // Taken from the old rows: the top row's note position, which the
        // new rows (however many images above it grew or shrank) find again.
        let anchor = self.view_anchor();
        let Some(shown) = self.shown.as_mut() else {
            return;
        };
        shown.plan.pane = wrapped.pane;
        shown.wrapped = wrapped;
        if let Some((line, at, strategy)) = anchor {
            self.scroll
                .scroll_to_item(row_of(&shown.wrapped.rows, line, at), strategy);
        }
        cx.notify();
    }

    /// What the preview has in view, as a note position (line, byte) and
    /// how it is placed: a scroll not yet applied (the first match as the
    /// note landed); the first match (or the top) until the reader scrolls;
    /// else the row at the top of the pane, where the reader scrolled to.
    fn view_anchor(&self) -> Option<(usize, usize, ScrollStrategy)> {
        let shown = self.shown.as_ref()?;
        let rows = &shown.wrapped.rows;
        let state = self.scroll.0.borrow();
        if let Some(deferred) = state.deferred_scroll_to_item {
            let row = rows.get(deferred.item_index)?;
            return Some((row.line, row.start, deferred.strategy));
        }
        if !self.scrolled {
            return Some(match shown.first_match {
                Some((line, at)) => (line, at, anchor_strategy(shown)),
                None => (0, 0, ScrollStrategy::Top),
            });
        }
        // The rows are still the old ones here: the offset counts them.
        let size = state.last_item_size?;
        if rows.is_empty() || size.contents.height <= px(0.) {
            return None;
        }
        let row_height = size.contents.height / rows.len() as f32;
        let top = (-state.base_handle.offset().y / row_height).max(0.) as usize;
        let row = &rows[top.min(rows.len() - 1)];
        Some((row.line, row.start, ScrollStrategy::Top))
    }

    fn render_header(&self, cx: &gpui_kit::App) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let (name, count): (SharedString, Option<SharedString>) = match &self.shown {
            Some(shown) => {
                let needles = matches!(&self.wanted, Some((_, Mark::Needles(n))) if !n.is_empty());
                let count = needles.then(|| match shown.matches {
                    1 => "1 match".into(),
                    n => format!("{n} matches").into(),
                });
                (shown.name.clone(), count)
            }
            None => ("Preview".into(), None),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .text_sm()
            .border_b_1()
            .border_color(theme::divider(t))
            .child(
                ellipsis(name)
                    .flex_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::bright(t)),
            )
            .when_some(count, |row, count| {
                row.child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child(count),
                )
            })
            .into_any_element()
    }

    /// An invisible element over the rows' area that measures its width in
    /// monospace cells after layout, and the room it gives an image (its
    /// width and height and a row's height, in device pixels), and re-wraps
    /// on a change (a narrow or wide layout, the pane's expand state, a
    /// window resize). Nothing happens while the size holds.
    fn measure(&self, cell_width: f32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let measured = (self.columns, self.pane);
        let preview = cx.entity().downgrade();
        canvas(
            move |bounds, window, cx| {
                let columns = (f32::from(bounds.size.width) / cell_width).floor().max(1.) as usize;
                let scale = window.scale_factor();
                let pane = ImagePane {
                    column_px: columns as f32 * cell_width * scale,
                    pane_px: f32::from(bounds.size.height) * scale,
                    row_px: LINE_REMS * f32::from(window.rem_size()) * scale,
                };
                if (columns, Some(pane)) != measured {
                    cx.defer(move |cx| {
                        preview
                            .update(cx, |preview, cx| preview.set_measure(columns, pane, cx))
                            .ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

impl Render for SearchPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let (family, size) = {
            let kit = cx.theme();
            (kit.mono_font_family.clone(), kit.mono_font_size)
        };
        // One monospace cell, from the text system: the wrap's column width.
        let cell_width =
            CellMetrics::from_text_system(window.text_system(), &font(family.clone()), size, size)
                .width();
        let body = match (&self.shown, &self.error) {
            (_, Some(error)) => div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme::error(t))
                .child(error.clone())
                .into_any_element(),
            (Some(shown), None) => {
                // The reader's section in the accent colour (the TUI's
                // range highlight); needles in the match colour.
                let mark = if shown.source.is_some() {
                    HighlightStyle {
                        color: Some(theme::accent(t).into()),
                        ..Default::default()
                    }
                } else {
                    HighlightStyle {
                        color: Some(theme::search_match(t).into()),
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    }
                };
                let row_id = self.ids.1;
                uniform_list(
                    SharedString::from(format!("{}-lines", self.ids.0)),
                    shown.wrapped.rows.len(),
                    cx.processor(move |this, range: Range<usize>, _, _| {
                        let Some(shown) = this.shown.as_ref() else {
                            return Vec::new();
                        };
                        range
                            .filter_map(|ix| Some((ix, shown.wrapped.rows.get(ix)?)))
                            .map(|(ix, row)| {
                                // An image row or a note row is blank for
                                // now: it keeps the row height.
                                let text = (row.kind == RowKind::Text).then(|| {
                                    StyledText::new(row.text.clone()).with_highlights(
                                        row.hits.iter().map(|hit| (hit.clone(), mark)),
                                    )
                                });
                                // Already wrapped to the pane: never wrapped
                                // again, never cut with an ellipsis.
                                div()
                                    .id((row_id, ix))
                                    .test_support()
                                    .h(rems(LINE_REMS))
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .children(text)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .on_scroll(cx.listener(|this, _: &ScrollEvent, _, _| {
                    this.scrolled = true;
                }))
                .size_full()
                .into_any_element()
            }
            (None, None) if self.is_reader() => div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme::muted(t))
                .child(READER_LOADING)
                .into_any_element(),
            (None, None) => div().into_any_element(),
        };
        // Another note's text while the selected one is read: dimmed. The
        // same note being re-marked for a new query stays as it is.
        let stale = match (&self.wanted, &self.shown) {
            (Some((path, _)), Some(shown)) => &shown.path != path,
            _ => false,
        };
        div()
            .id(self.ids.0)
            .test_support()
            .role(Role::Document)
            .aria_label("Preview")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_header(cx))
            .when(self.shown.as_ref().is_some_and(|s| s.truncated), |pane| {
                pane.child(
                    div()
                        .id(PREVIEW_TRUNCATED_ID)
                        .test_support()
                        .role(Role::Note)
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child("Preview cut short. Open the note to see all of it."),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .py_1()
                    .font_family(family)
                    .text_size(size)
                    .text_color(theme::foreground(t))
                    .when(stale, |body| body.opacity(0.5))
                    .child(
                        // The rows' area, measured without the padding.
                        div()
                            .relative()
                            .size_full()
                            .child(self.measure(cell_width, cx))
                            .child(body),
                    ),
            )
    }
}

// ---- The query-syntax reference ----------------------------------------------

/// The reference card's body: each section of [`SYNTAX`], its keys beside
/// what they do.
fn query_help_body(t: &Theme) -> AnyElement {
    div()
        .id(QUERY_HELP_ID)
        .test_support()
        .role(Role::Document)
        .aria_label("Query syntax")
        .flex()
        .flex_col()
        .gap_3()
        .text_sm()
        .children(SYNTAX.iter().map(|(heading, rows)| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::muted(t))
                        .child(*heading),
                )
                .children(rows.iter().map(|(keys, meaning)| {
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .w(rems(9.))
                                .flex_none()
                                .text_color(theme::key_cap(t))
                                .child(*keys),
                        )
                        .child(div().text_color(theme::foreground(t)).child(*meaning))
                }))
        }))
        .into_any_element()
}

// ---- The workspace's side ----------------------------------------------------

/// The query-syntax dialog's width, in rems.
const HELP_REMS: f32 = 30.;

/// The search query for a label: `#label`, quoted as one term (a
/// frontmatter label may hold spaces).
pub(crate) fn label_query(label: &str) -> String {
    format!("#{}", kimun_core::quote_query_term(label))
}

impl Workspace {
    /// Open the search browser on `query` (empty: the recent notes).
    /// Refused while a modal is open; says so without a workspace.
    pub(crate) fn open_search(
        &mut self,
        query: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let open = self
            .active_state()
            .notes
            .as_ref()
            .and_then(|h| h.note.clone());
        let recents = self
            .settings
            .as_ref()
            .map(|s| {
                s.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .current_last_paths()
            })
            .unwrap_or_default();
        let source = Arc::new(kimun_notes::logic::pickers::resolving_search_source(
            vault.clone(),
            recents,
            open.clone(),
        ));
        let preview = cx.new(|cx| SearchPreview::new(vault, cx));
        self.open_search_picker(source, query.unwrap_or_default(), open, preview, window, cx);
    }

    /// A followed label (`#tag`): the search browser on that label.
    pub(crate) fn search_label(&mut self, name: &str, cx: &mut Context<Self>) {
        let query = label_query(name);
        self.with_window(cx, move |workspace, window, cx| {
            workspace.open_search(Some(query), window, cx)
        });
    }

    /// `F1` or the browser's `?` button: the query syntax over the browser.
    /// From FIND (no modal open), the query syntax alone, as the modal.
    pub(crate) fn open_query_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let slot = match self.open_modal(window, cx) {
            Some(ModalKind::Search) if !self.query_help && self.query_builder.is_none() => {
                self.query_help = true;
                Slot::Stacked
            }
            None if !modal::dialog_open(window, cx) => Slot::Own(ModalKind::QueryHelp),
            _ => return,
        };
        let workspace = cx.entity().downgrade();
        let theme = theme::current(cx);
        self.show_modal(
            slot,
            HELP_REMS,
            |_, _| {},
            move |dialog, _, _| {
                let workspace = workspace.clone();
                dialog
                    .title("Query syntax")
                    // Help to read, nothing to confirm: the kit's close button stays.
                    .close_button(true)
                    .child(query_help_body(&theme))
                    .on_close(move |_, _, cx| {
                        workspace
                            .update(cx, |workspace, _| workspace.query_help = false)
                            .ok();
                    })
            },
            window,
            cx,
        );
    }

    /// Enter on a note in the search browser: open it with `emphasis`. With
    /// unsaved edits in the open note, they are written first while the
    /// browser waits (true: the browser is busy until then); a failed write
    /// keeps the browser open with the reason.
    pub(crate) fn search_open(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The browser's source lists notes only (logic `SearchNotesProvider`:
        // indexed notes and the recents). So `open_chosen` below always takes the note branch,
        // which focuses the editor; a folder would show FILES unfocused.
        debug_assert!(path.is_note(), "the search browser lists notes only");
        let quiet = self.search_flush.is_some()
            || self.note_held()
            || self.closing != Closing::Open
            || self
                .active_state()
                .notes
                .as_ref()
                .is_none_or(|h| h.is_switching() || h.is_open_wanted());
        if !path.is_note() || quiet || !self.can_save() || !self.editor_dirty(cx) {
            // Nothing to write first (or an open already under way, which
            // writes it): the open note's switch does the rest.
            self.close_modal(window, cx);
            self.open_chosen(path, emphasis, window, cx);
            return false;
        }
        // No tick from here: the flush is the only write. The landing is
        // the flushed group's, whichever group is active by then.
        let group = self.layout.active_group();
        let Some(flush) = self.begin_note_flush_in(group, cx) else {
            self.close_modal(window, cx);
            self.open_chosen(path, emphasis, window, cx);
            return false;
        };
        self.search_flush = Some(cx.spawn_in(window, async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.search_flushed(group, path, emphasis, flushed, window, cx)
            })
            .ok();
        }));
        true
    }

    /// The write before a search open ended: open the note, or keep the
    /// browser with the reason.
    fn search_flushed(
        &mut self,
        group: GroupId,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        flushed: Result<FlushResult, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_flush = None;
        let outcome = self.apply_flush(group, flushed, cx);
        self.restart_tick_unless_read_only_in(group);
        self.block_input_in(group, false, cx);
        let picker = self.picker.as_ref().and_then(|p| p.upgrade());
        match flush_problem(&outcome) {
            Some(problem) => {
                let note = self
                    .host_in(group)
                    .and_then(|h| h.note.clone())
                    .unwrap_or_else(VaultPath::root);
                let reason = format!("Couldn't save {note}: {problem}. Nothing was opened.");
                match picker {
                    Some(picker) => {
                        picker.update(cx, |picker, cx| picker.set_busy(false, Some(reason), cx))
                    }
                    None => self.flash_error(reason, cx),
                }
            }
            None => {
                if self.open_modal(window, cx) == Some(ModalKind::Search) {
                    self.close_modal(window, cx);
                }
                self.open_chosen(path, emphasis, window, cx);
            }
        }
        self.file_op_ended(cx);
    }
}
