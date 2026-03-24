pub mod note_browse_entry;
pub mod note_list_loader;

use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::components::note_list::note_browse_entry::NoteBrowseEntry;
use crate::components::note_list::note_list_loader::LoadState;
use crate::settings::AppSettings;
use crate::themes::Theme;
use crate::utils::sparse_vector::SparseVector;

#[derive(Clone, PartialEq, Props)]
pub struct NoteListProps<H>
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    entries: Signal<Vec<NoteBrowseEntry>>,
    active_path: VaultPath,
    element_action: H,
    selector_handler: SelectorHandler,
    #[props(default = false)]
    compact: bool,
    #[props(optional)]
    load_state: Option<Signal<LoadState>>,
}

pub trait NoteElementActions: Clone + PartialEq {
    fn on_hover(&self, entry: &NoteBrowseEntry) -> Element;
    fn on_select(&mut self, entry: &NoteBrowseEntry);
}

#[derive(Clone, PartialEq)]
pub struct SelectorHandler {
    entries: Signal<Vec<NoteBrowseEntry>>,
    selected: Signal<Option<usize>>,
    manually_selected: Signal<usize>,
}

impl SelectorHandler {
    pub fn build(entries: Signal<Vec<NoteBrowseEntry>>) -> Self {
        Self {
            entries,
            selected: use_signal(|| None),
            manually_selected: use_signal(|| 0),
        }
    }

    pub fn set_selected(&self, value: Option<usize>) {
        let mut selected = self.selected;
        *selected.write() = value;
    }

    pub fn get_selected(&self) -> Option<usize> {
        self.selected.read().to_owned()
    }

    pub fn select_next(&self) {
        let max_items = self.entries.peek().len();
        let new_selected = if max_items == 0 {
            None
        } else if let Some(ref current_selected) = self.get_selected() {
            let current_selected = current_selected.to_owned();
            if current_selected < max_items - 1 {
                Some(current_selected + 1)
            } else {
                Some(0)
            }
        } else {
            Some(0)
        };
        if let Some(sel) = new_selected {
            let mut manually_selected = self.manually_selected;
            manually_selected.set(sel);
        }
        self.set_selected(new_selected);
    }

    pub fn select_prev(&self) {
        let max_items = self.entries.peek().len();
        let new_selected = if max_items == 0 {
            None
        } else if let Some(current_selected) = self.get_selected() {
            if current_selected > 0 {
                Some(current_selected - 1)
            } else {
                Some(max_items - 1)
            }
        } else {
            Some(0)
        };
        if let Some(sel) = new_selected {
            let mut manually_selected = self.manually_selected;
            manually_selected.set(sel);
        }
        self.set_selected(new_selected);
    }
}

// Memoized list item component - each item subscribes to selected signal independently
// This allows only the previously selected and newly selected items to re-render
#[derive(Clone, PartialEq, Props)]
struct NoteListItemProps<H>
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    entry: NoteBrowseEntry,
    index: usize,
    is_active: bool,
    theme: Theme,
    item_class: String,
    element_action: H,
    selected_signal: Signal<Option<usize>>,
    selector_handler: SelectorHandler,
    select_by_mouse: Signal<bool>,
    row_mounts: Signal<SparseVector<Rc<MountedData>>>,
}

#[component]
fn NoteListItem<H>(props: NoteListItemProps<H>) -> Element
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    let NoteListItemProps {
        entry,
        index,
        is_active,
        theme,
        item_class,
        mut element_action,
        selected_signal,
        selector_handler,
        select_by_mouse,
        mut row_mounts,
    } = props;

    // Memoize selection check - only re-render when THIS item's selection status changes
    // Not when the selection signal changes to a different item
    let is_selected_memo = use_memo(move || selected_signal() == Some(index));
    let is_selected = is_selected_memo();

    let cls = format!(
        "{}{}",
        item_class,
        if is_selected {
            " selected"
        } else if is_active {
            " active"
        } else {
            ""
        },
    );

    let border_color = if is_selected {
        theme.accent_yellow.to_string()
    } else if is_active {
        theme.accent_green.to_string()
    } else {
        "transparent".to_string()
    };

    let entry_action = entry.clone();

    rsx! {
        div {
            class: "{cls}",
            border_bottom_color: "{theme.border_light}",
            border_left_color: "{border_color}",
            background_color: if is_selected { "{theme.bg_hover}" } else { "transparent" },
            id: "element-{index}",
            onmounted: move |e| {
                row_mounts.write().insert(index, e.data());
            },
            onmouseover: move |_e| {
                if *select_by_mouse.peek() {
                    selector_handler.set_selected(Some(index));
                }
            },
            onclick: move |e| {
                info!("Clicked element");
                e.stop_propagation();
                element_action.on_select(&entry_action);
            },
            {entry.get_view(&theme)}

            if is_selected {
                {element_action.on_hover(&entry)}
            }
        }
    }
}

#[component]
pub fn NoteList<H>(props: NoteListProps<H>) -> Element
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    let settings: Signal<AppSettings> = use_context();
    let selector_handler = props.selector_handler;
    let entries = props.entries;

    // Use peek() to avoid subscribing to entries signal for length check
    let num_entries = props.entries.peek().len();
    let active_path: VaultPath = props.active_path;
    let element_action = props.element_action;

    let mut select_by_mouse = use_signal(|| true);
    let row_mounts = use_signal(|| SparseVector::<Rc<MountedData>>::with_capacity(num_entries));

    // Chunked rendering: Show items incrementally to avoid blocking UI
    const INITIAL_CHUNK: usize = 50; // Render first 50 immediately
    const CHUNK_SIZE: usize = 100; // Then render 100 at a time
    const CHUNK_DELAY_MS: u64 = 10; // Small delay between chunks

    let mut visible_count = use_signal(|| INITIAL_CHUNK);
    let mut render_version = use_signal(|| 0u32); // Track when entries change

    // Reset visible count when entries change
    use_effect(move || {
        // Subscribe to entries to detect changes
        let total = entries().len();

        // Reset to show initial chunk
        visible_count.set(INITIAL_CHUNK.min(total));

        // Increment version to trigger incremental rendering
        let current_version = *render_version.peek();
        render_version.set(current_version.wrapping_add(1));
    });

    // Incrementally render remaining items
    _ = use_resource(move || {
        let version = render_version();
        let total = entries.peek().len();
        let start_count = visible_count();

        async move {
            let mut current = start_count;

            while current < total {
                tokio::time::sleep(std::time::Duration::from_millis(CHUNK_DELAY_MS)).await;

                // Check if version changed (new data arrived)
                if render_version() != version {
                    break;
                }

                current = (current + CHUNK_SIZE).min(total);
                visible_count.set(current);
            }
        }
    });

    _ = use_resource(move || async move {
        let r = selector_handler.manually_selected.read().to_owned();
        // Use peek() to avoid subscribing to row_mounts
        if let Some(mount) = row_mounts.peek().get(r) {
            let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
            select_by_mouse.set(false);
        }
    });

    let item_class = if props.compact {
        "note-item-compact"
    } else {
        "note-item"
    };

    // Memoize theme to avoid re-reading settings signal on every render
    let theme_memo = use_memo(move || settings.peek().get_theme());
    let theme = theme_memo();
    let selector_mouse = selector_handler.clone();

    // Check if we're in loading state
    let is_initializing = props
        .load_state
        .as_ref()
        .map(|state| matches!(state(), LoadState::Initializing))
        .unwrap_or(false);

    rsx! {
        div {
            class: "entry-list",
            id: "entryList",
            onmousemove: move |_e| {
                if !*select_by_mouse.peek() {
                    select_by_mouse.set(true);
                }
            },
            onmouseleave: move |_e| {
                if *select_by_mouse.peek() {
                    selector_mouse.set_selected(None);
                }
            },
            // Show loading message during initialization
            if is_initializing {
                div {
                    class: "loading-initial",
                    style: "padding: 2rem; text-align: center; color: {theme.text_muted};",
                    "Loading notes..."
                }
            } else {
                // Use keyed list to optimize re-renders
                // Each NoteListItem subscribes to selected_signal independently
                // Only render items up to visible_count for chunked rendering
                {
                    let all_entries = entries();
                    let visible = visible_count();
                    let items_to_render = all_entries.iter().take(visible).enumerate();

                    rsx! {
                        for (index , entry) in items_to_render {
                            {
                                let entry_path = entry.get_path().to_owned();
                                let is_active = entry_path.eq(&active_path);

                                rsx! {
                                    NoteListItem {
                                        key: "{entry_path}",
                                        entry: entry.clone(),
                                        index,
                                        is_active,
                                        theme: theme.clone(),
                                        item_class: item_class.to_string(),
                                        element_action: element_action.clone(),
                                        selected_signal: selector_handler.selected,
                                        selector_handler: selector_handler.clone(),
                                        select_by_mouse,
                                        row_mounts,
                                    }
                                }
                            }
                        }
                        // Show loading indicator if there are more items
                        if visible < all_entries.len() {
                            div {
                                class: "loading-more",
                                style: "padding: 1rem; text-align: center; color: {theme.text_muted};",
                                "Loading {all_entries.len() - visible} more..."
                            }
                        }
                    }
                }
            }
        }
    }
}
