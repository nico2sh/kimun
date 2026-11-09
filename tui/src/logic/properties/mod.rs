//! The note's frontmatter properties, toolkit-free: the add/edit form's field
//! model ([`PropertyForm`]), the write plan with every refusal and no-op rule
//! ([`plan_write`]), the write / delete / load jobs that report through
//! [`OverlayData`], and property write errors classified from core's
//! `VaultError`. The TUI's properties dialog renders and handles keys on top
//! of these; a GUI dialog calls the same functions.

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::{
    FrontmatterFormat, PropertyEntry, PropertyInput, PropertyKind, PropertyValue,
    is_list_property_key, property_keys_match,
};

use crate::logic::message::{AppMessage, AppTx, OverlayData};

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Write errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PropertyWriteError {
    /// Core refused the value (`VaultError::InvalidProperty`).
    Invalid(String),
    /// The note's frontmatter block does not parse; the dialog turns read-only.
    Malformed(String),
    /// Anything else (IO, lock timeout).
    Other(String),
}

impl PropertyWriteError {
    pub(crate) fn from_vault(e: kimun_core::error::VaultError) -> Self {
        use kimun_core::error::{FSError, VaultError};
        match &e {
            VaultError::InvalidProperty { message, .. } => Self::Invalid(message.clone()),
            VaultError::FSError(FSError::InvalidFrontmatter { .. }) => {
                Self::Malformed(e.to_string())
            }
            _ => Self::Other(e.to_string()),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(m) | Self::Malformed(m) | Self::Other(m) => m,
        }
    }
}

// ---------------------------------------------------------------------------
// List rows
// ---------------------------------------------------------------------------

/// (key, type, value) as the property list shows an entry: a bare key (YAML
/// `due:`) has type `—` and an empty value.
pub fn row_cells(entry: &PropertyEntry) -> (String, String, String) {
    let (key, value) = entry;
    match value {
        Some(v) => (key.clone(), v.kind().as_str().to_string(), v.to_string()),
        None => (key.clone(), "—".to_string(), String::new()),
    }
}

// ---------------------------------------------------------------------------
// Form field model
// ---------------------------------------------------------------------------

/// The type cycler's choices in order; `None` = auto (let core decide against
/// the vault's type).
pub const KIND_CHOICES: [Option<PropertyKind>; 7] = [
    None,
    Some(PropertyKind::Text),
    Some(PropertyKind::Number),
    Some(PropertyKind::Bool),
    Some(PropertyKind::Date),
    Some(PropertyKind::DateTime),
    Some(PropertyKind::List),
];

/// The add/edit form's fields, without widgets or key handling. A front end
/// keeps `key` and `value` in step with its inputs before asking anything of
/// the model ([`input_values`](Self::input_values), [`plan_write`],
/// [`show_write_error`](Self::show_write_error)).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyForm {
    /// `None` for Add; the original key for Edit.
    pub orig_key: Option<String>,
    /// The original value's kind (Edit), shown as `(currently <kind>)`.
    pub current_kind: Option<PropertyKind>,
    /// The original entry had no value (YAML `due:`).
    pub was_bare: bool,
    /// The Value text the Edit form was pre-filled with (`None` for Add or a
    /// bare key): saving it unchanged under auto writes nothing.
    pub orig_text: Option<String>,
    /// The Key field's text, as typed.
    pub key: String,
    /// The Value field's text, as typed.
    pub value: String,
    /// Index into [`KIND_CHOICES`]; 0 = auto.
    kind_idx: usize,
    /// Shown under Value: core's refusal or a local check.
    pub error: Option<String>,
    /// Offered after an `auto` save was refused: retry forced to this kind.
    pub store_anyway: Option<PropertyKind>,
}

impl PropertyForm {
    /// An empty Add form.
    pub fn add() -> Self {
        Self::default()
    }

    /// An Edit form for `key`, pre-filled with `value`'s text (`None` = a
    /// bare key: empty Value, `was_bare`).
    pub fn edit(key: &str, value: Option<&PropertyValue>) -> Self {
        let mut form = Self {
            orig_key: Some(key.to_string()),
            key: key.to_string(),
            ..Self::default()
        };
        match value {
            Some(v) => {
                form.current_kind = Some(v.kind());
                // An empty list displays as `[]`; pre-fill it as empty.
                let text = match v {
                    PropertyValue::List(items) if items.is_empty() => String::new(),
                    other => other.to_string(),
                };
                form.orig_text = Some(text.clone());
                form.value = text;
            }
            None => form.was_bare = true,
        }
        form
    }

    /// The chosen type; `None` = auto.
    pub fn kind(&self) -> Option<PropertyKind> {
        KIND_CHOICES[self.kind_idx]
    }

    /// The chosen type as the cycler labels it (`auto`, `text`, …).
    pub fn kind_label(&self) -> &'static str {
        self.kind().map_or("auto", PropertyKind::as_str)
    }

    /// Choose a type directly (a GUI select); clears the feedback like
    /// [`cycle_kind`](Self::cycle_kind).
    pub fn set_kind(&mut self, kind: Option<PropertyKind>) {
        self.kind_idx = KIND_CHOICES.iter().position(|k| *k == kind).unwrap_or(0);
        self.clear_feedback();
    }

    /// Step the type cycler (wrapping) and clear the feedback.
    pub fn cycle_kind(&mut self, forward: bool) {
        let n = KIND_CHOICES.len();
        self.kind_idx = if forward {
            (self.kind_idx + 1) % n
        } else {
            (self.kind_idx + n - 1) % n
        };
        self.clear_feedback();
    }

    /// Drop the error and the store-anyway offer (any edit to the form).
    pub fn clear_feedback(&mut self) {
        self.error = None;
        self.store_anyway = None;
    }

    /// The field as `PropertyInput` values: comma-split for a list (chosen,
    /// or `auto` on a key that was a list or that core always keeps as a
    /// list), one untrimmed value otherwise.
    pub fn input_values(&self) -> Vec<String> {
        let as_list = match self.kind() {
            Some(k) => k == PropertyKind::List,
            None => {
                self.current_kind == Some(PropertyKind::List)
                    || is_list_property_key(self.key.trim())
            }
        };
        if as_list {
            self.value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            vec![self.value.clone()]
        }
    }

    /// The kind the input infers to: the `Store as <kind> anyway` target.
    pub fn inferred_kind(&self) -> PropertyKind {
        let v = self.input_values();
        if v.len() > 1 {
            PropertyKind::List
        } else {
            PropertyValue::infer(v.first().map_or("", String::as_str)).kind()
        }
    }

    /// The store-anyway rule: a value core refused (`Invalid`) under `auto`
    /// offers a retry forced to the inferred kind; anything else offers none.
    pub fn store_anyway_for(&self, e: &PropertyWriteError) -> Option<PropertyKind> {
        match e {
            PropertyWriteError::Invalid(_) if self.kind().is_none() => Some(self.inferred_kind()),
            _ => None,
        }
    }

    /// A failed write lands on the form: its message under Value, plus the
    /// store-anyway offer ([`store_anyway_for`](Self::store_anyway_for)),
    /// which is returned.
    pub fn show_write_error(&mut self, e: &PropertyWriteError) -> Option<PropertyKind> {
        self.error = Some(e.message().to_string());
        self.store_anyway = self.store_anyway_for(e);
        self.store_anyway
    }

    /// A local refusal from [`plan_write`] lands on the form (no offer).
    pub fn show_refusal(&mut self, e: &FormError) {
        if let Some(msg) = e.message() {
            self.error = Some(msg);
            self.store_anyway = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Write plan
// ---------------------------------------------------------------------------

/// Why [`plan_write`] writes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    /// The key is blank after trimming: `key required`.
    KeyRequired,
    /// Add with a key the note already has (matched like core matches keys):
    /// `'<key>' already exists — edit it`.
    KeyExists(String),
    /// The value is empty and the key was not bare: `value required`.
    ValueRequired,
    /// Nothing to write — not an error: an Edit saved as pre-filled, or a
    /// bare key saved still bare under its own name. Return to the list.
    Unchanged,
}

impl FormError {
    /// The text shown under Value; `None` for [`FormError::Unchanged`].
    pub fn message(&self) -> Option<String> {
        match self {
            Self::KeyRequired => Some("key required".into()),
            Self::KeyExists(key) => Some(format!("'{key}' already exists — edit it")),
            Self::ValueRequired => Some("value required".into()),
            Self::Unchanged => None,
        }
    }
}

/// A validated write: an optional rename, then an optional set.
#[derive(Debug, Clone, PartialEq)]
pub struct WritePlan {
    /// The (trimmed) key written to.
    pub key: String,
    /// The Edit's original key when it differs from `key`: renamed first.
    pub rename_from: Option<String>,
    /// The value to set under `key`; `None` = rename only (a bare key).
    pub set: Option<PropertyInput>,
}

impl WritePlan {
    /// (old, new) when the plan renames: if the set after it fails, the
    /// rename already landed (the list must reload, the form's key is `new`).
    pub fn rename(&self) -> Option<(String, String)> {
        self.rename_from.clone().map(|old| (old, self.key.clone()))
    }
}

/// Validate `form` against the note's `entries` and plan the write. `forced`
/// overrides the type cycler (the "store anyway" retry). Rules, in order:
/// key required; Add never overwrites an existing key; an Edit saved as
/// pre-filled (same key, type auto, same text, not forced) is `Unchanged`;
/// value required unless the key was bare; a bare key left empty is renamed
/// only, or `Unchanged` when its name did not change either.
pub fn plan_write(
    entries: &[PropertyEntry],
    form: &PropertyForm,
    forced: Option<PropertyKind>,
) -> Result<WritePlan, FormError> {
    let new_key = form.key.trim().to_string();
    if new_key.is_empty() {
        return Err(FormError::KeyRequired);
    }
    // Add never overwrites: a key the note already has is edited instead.
    if form.orig_key.is_none()
        && entries
            .iter()
            .any(|(k, _)| property_keys_match(k, &new_key))
    {
        return Err(FormError::KeyExists(new_key));
    }
    // An Edit saved as pre-filled (same key, type auto, same text) would
    // only re-run inference on Display text: write nothing.
    if forced.is_none()
        && form.kind().is_none()
        && form.orig_key.as_deref() == Some(new_key.as_str())
        && form.orig_text.as_deref() == Some(form.value.as_str())
    {
        return Err(FormError::Unchanged);
    }
    let values = form.input_values();
    let value_empty = values.iter().all(|v| v.trim().is_empty());
    let renamed = form
        .orig_key
        .as_deref()
        .filter(|o| *o != new_key)
        .map(str::to_string);
    // A bare key whose value is still empty: rename only (or nothing).
    let rename_only = form.was_bare && value_empty;
    if value_empty && !rename_only {
        return Err(FormError::ValueRequired);
    }
    if rename_only && renamed.is_none() {
        return Err(FormError::Unchanged);
    }
    let set = (!rename_only).then(|| {
        let mut input = PropertyInput::new(values).forced(forced.or(form.kind()));
        // Edit under auto: the note's own kind is the guess when the vault
        // has none (core still checks it against the vault's kind).
        if let Some(kind) = form.current_kind {
            input = input.implied(kind);
        }
        input
    });
    Ok(WritePlan {
        key: new_key,
        rename_from: renamed,
        set,
    })
}

// ---------------------------------------------------------------------------
// Jobs (async; every file change is a core call)
// ---------------------------------------------------------------------------

/// Read `path`'s properties and send [`OverlayData::PropertiesLoaded`]; the
/// result is also returned.
pub async fn load(
    vault: &NoteVault,
    path: VaultPath,
    tx: &AppTx,
) -> Result<Vec<PropertyEntry>, String> {
    let result = vault.get_properties(&path).await.map_err(|e| e.to_string());
    tx.send(AppMessage::OverlayData(OverlayData::PropertiesLoaded {
        path,
        result: result.clone(),
    }))
    .ok();
    result
}

/// Read every property key in the vault and send
/// [`OverlayData::PropertyKeysLoaded`]. A failed read sends nothing (pickers
/// stay empty) and returns `None`.
pub async fn load_keys(vault: &NoteVault, tx: &AppTx) -> Option<Vec<String>> {
    let keys = vault.property_keys().await.ok()?;
    tx.send(AppMessage::OverlayData(OverlayData::PropertyKeysLoaded(
        keys.clone(),
    )))
    .ok();
    Some(keys)
}

/// Run `plan` on `path`: the rename (if any), then the set (if any). Sends
/// [`AppMessage::NoteReloadFromDisk`] whenever the file changed — the rename
/// alone counts, even if the set after it fails — and then
/// [`OverlayData::PropertyWritten`]. The reload is sent here, not by the
/// result's receiver, so the editor reloads even when the dialog is gone.
/// Ok = the flash text `property saved`.
pub async fn run_write(
    vault: &NoteVault,
    path: VaultPath,
    plan: WritePlan,
    tx: &AppTx,
) -> Result<String, PropertyWriteError> {
    let WritePlan {
        key,
        rename_from,
        set,
    } = plan;
    // Whether the file changed: the rename alone counts, even if the set
    // after it fails.
    let mut changed = false;
    let result = async {
        if let Some(old) = &rename_from {
            // A refused rename is never a type mismatch: report it as
            // `Other` so the form offers no "Store as … anyway".
            vault.rename_property(&path, old, &key).await.map_err(|e| {
                match PropertyWriteError::from_vault(e) {
                    PropertyWriteError::Invalid(m) => PropertyWriteError::Other(m),
                    other => other,
                }
            })?;
            changed = true;
        }
        if let Some(input) = &set {
            vault
                .set_property_from_input(&path, &key, input, FrontmatterFormat::Toml)
                .await
                .map_err(PropertyWriteError::from_vault)?;
            changed = true;
        }
        Ok::<_, PropertyWriteError>("property saved".to_string())
    }
    .await;
    if changed {
        tx.send(AppMessage::NoteReloadFromDisk(path.clone())).ok();
    }
    tx.send(AppMessage::OverlayData(OverlayData::PropertyWritten {
        path,
        result: result.clone(),
    }))
    .ok();
    result
}

/// Remove `key` from `path`. On success sends
/// [`AppMessage::NoteReloadFromDisk`], then always
/// [`OverlayData::PropertyWritten`]. Ok = the flash text `property removed`.
pub async fn run_delete(
    vault: &NoteVault,
    path: VaultPath,
    key: &str,
    tx: &AppTx,
) -> Result<String, PropertyWriteError> {
    let result = vault
        .remove_property(&path, key)
        .await
        .map(|_| "property removed".to_string())
        .map_err(PropertyWriteError::from_vault);
    if result.is_ok() {
        tx.send(AppMessage::NoteReloadFromDisk(path.clone())).ok();
    }
    tx.send(AppMessage::OverlayData(OverlayData::PropertyWritten {
        path,
        result: result.clone(),
    }))
    .ok();
    result
}
