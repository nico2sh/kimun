# The Wiki section of the desktop app: design

Date: 2026-11-28. Status: for approval.

This design builds on ADR 0068. Item 15 of that ADR says the GUI gets a Preferences row and a command palette entry. Both run `wiki init`. The app never prompts for it.

## Intent

A user of the desktop app can set up a wiki without a terminal. The user can also change the layout of the wiki. The reason is reach. The wiki feature exists in the CLI and in the MCP server. A user who lives in the desktop app cannot turn it on.

The user asked for flexibility. The layout must not be fixed to one method. The user chooses five values:

- the folder of raw sources
- the folder of wiki pages
- the log note
- the schema note
- the properties that every wiki page needs

These five values are the whole declaration in `.kimun/wiki.toml`. The checks and the source protection already understand them. Nothing else must change for the flexibility to be real.

## Goals

- A Wiki section in Preferences. It shows the state of the current vault and edits the five values.
- One button. It sets up a new wiki. After a change, it applies the new layout.
- A command palette entry that sets up a wiki with the default layout.
- Logic with no GUI code in it. The terminal app can offer the same section later.

## Non-goals

- Moving or renaming notes. A change of folder changes the declaration and nothing else.
- New folder roles, such as several source folders or a reference folder that is not protected. This needs a new declaration format and a new ADR. It is a separate change.
- A Wiki section in the terminal app.
- Running `wiki check`, `wiki clip`, or `wiki stamp` from the desktop app.
- Editing the text of the schema note. The user opens it like any note.
- A CLI or MCP command that changes the declaration. They keep `wiki init`.

## Anti-goals

- The app never asks the user to set up a wiki. A vault that is not a wiki shows no prompt, no badge, and no hint outside the Wiki section and the palette.
- The app never changes source protection without saying so. When the sources folder changes, the section says which folder stops being protected and which folder starts. It says this before the user applies.
- The app never deletes or moves a note or a folder for this feature.

## Approaches

The user picked the first approach.

1. Add a core function `wiki_configure`. It takes a full declaration, checks it, creates what is missing, and writes the file. The section edits five fields. It acts at once through its own button, outside the Preferences draft, as the Workspaces and Indexing sections do. This fits because the declaration travels with the notes. It is not app configuration (see the glossary entry Wiki declaration). The cost is one core function, one logic model, one section, and one palette row.
2. Put the wiki fields into the Preferences draft, and save them with the Save button. Rejected. One Save must then write a file inside a vault and write the app configuration in the same action. The draft must also hold data of one vault.
3. Open `.kimun/wiki.toml` as text. Rejected. The file is hidden, and the user gets no checks.

For a change of folders, the user chose to change the declaration and move nothing. The other choice was to move the whole folder. It was rejected because it touches sources, which Kimün otherwise protects.

## Design

### Core

Add the method `NoteVault::wiki_configure(&self, declaration: WikiDeclaration) -> Result<WikiInitReport, VaultError>`.

- First it runs `declaration.validate()`. A failure gives the same `InvalidWikiDeclaration` error as a bad file.
- It checks each required property name. The name must be a valid property key by the rule that `set_property` uses. It removes duplicates and empty names. An empty list is allowed.
- When items are missing, it creates them: the sources folder, the wiki folder, the schema note, and the log note. It uses the same templates as `wiki_init`. The schema template gets the new folder names and the new list of required properties. This is my call.
- It keeps every folder and note that exists. The report has one entry for each item, `created` or `kept`, as `wiki_init` gives.
- It writes `.kimun/wiki.toml` last. If a step fails, the old declaration stays in place. The folders that it already made stay too. They are empty and harmless.
- It does not read the old file. A user can replace a broken declaration by applying a good one. This is the way out for a vault whose file has a mistake.
- It never moves, renames, or deletes anything.
- The log and schema notes can be anywhere outside the sources folder, as `validate()` allows. If their parent folder is missing, the core makes it. The report still has one entry for each of the four items, with no extra line for a parent folder. A test must prove this for a log such as `/meta/log.md`. This is my call.
- `wiki_init` and `wiki_configure` share one private function. The function creates the layout that a declaration names. This gives one implementation of the folders, the templates, and the report.
- Source protection follows the new declaration at once, because the check reads the file.

### The schema template

Today `schema_template` in `core/src/wiki.rs` names `summary`, `sources`, `updated`, and `type` in fixed text. It does not read `required_properties`. If a user changes the list, a new schema note teaches the agent a different list than `wiki check` enforces. So the template builds its "Page format" block from `declaration.required_properties`. It keeps `type` as an optional property. It describes the three default names as today and gives any other name a generic line. The new text keeps the style of the old text. This is my call. A schema note that exists is never changed, so the effect note in the logic layer warns about it.

### Logic

The logic goes in the `kimun_notes::logic` module, with no GUI types. A new `wiki_settings` model holds what the section shows and the rules. The GUI only draws it.

- The state is one of four: no vault is open, the vault is not a wiki, the vault is a wiki with a declaration, or the declaration has an error with its message. The state comes from `wiki_declaration()`.
- The draft has five text fields: sources folder, wiki folder, log note, schema note, and required properties. The required properties are one text field, and the names are split on commas and spaces.
- When the vault is not a wiki, the draft starts as the defaults from `WikiDeclaration::defaults()`. One click then works with no typing.
- The log and schema paths follow the wiki folder until the user edits that field. They are `<wiki>/log.md` and `<wiki>/schema.md`. This is my call. It matches how `wiki init --wiki` places them.
- The model turns the draft into a `WikiDeclaration`. It makes the paths absolute and flat, as `with_folders` does.
- The first rule message from `validate()` is the field error. When there is an error, Apply is not possible. A draft that equals the saved declaration still allows Apply. In that case Apply repairs the layout: it creates any folder or note that someone deleted, and the result lines show what it created and what it kept. This is my call.
- A property name that is not a valid key is not checked in the logic layer. The user sees the core error after the apply. This keeps the key rule in core. This is my call.
- The effect note comes from the old and the new declaration. If the sources folder changes, the note says: "Notes in /sources stay where they are and are no longer protected. /raw is the new protected folder."
- If the wiki folder, the log, or the schema changes, the note has one line. The line says that the old notes stay where they are.
- If a folder, a note path, or the property list changes, and the schema note exists, the note has one more line: "The schema note keeps its text. Edit it so that it names the new layout."
- If nothing changes, there is no note.
- The button label is "Set up wiki" for a vault that is not a wiki. It is "Apply" for a wiki.

### GUI

- Add `Section::Wiki` after Indexing. Its title is "Wiki" and its slug is `wiki`. `Section::ALL` grows to ten. Every test that counts the sections or indexes them changes with it.
- The view follows Indexing. It acts at once, outside the draft. When there is no vault, or while another action runs, it is disabled. It runs the core call off the UI thread with `blocking`.
- When the section opens, it reads the declaration off the UI thread, as the Terminal app row reads its state. It reads again when the vault changes and after every apply. This is my call.
- The layout has five parts, from top to bottom: one line of state, five labeled fields, the effect note, the button, and the result of the last apply.
- The state line reads "This vault is not a wiki." or "This vault is a wiki."
- The result is plain lines such as `Created folder /raw.` and `Kept note /wiki/log.md.`
- If an apply fails, the view shows the error text and changes nothing.
- If the declaration has an error, the view shows its message. The fields start from the defaults, so the user can apply a good declaration.
- For the keyboard, the fields and the button are in the tab order. When Apply is possible, Enter in a field applies. The builder follows the keyboard tests of the other sections.
- The palette gets one GUI-only row, "Set up wiki", with a new `PaletteRun` variant. When no vault is open, it is blocked with the reason `NO_WORKSPACE`.
- The row runs `wiki_init` with the defaults off the UI thread. It shows the result with `flash`. The message depends on the report. If the row created something, it says "Wiki set up: created" and lists what it created. If everything was kept, it says "This vault is already a wiki. Change its folders in Preferences, Wiki." If the declaration file has a mistake, it says "The wiki declaration has a mistake. Fix it in Preferences, Wiki." and nothing changes. The row needs a test for each of the three messages. This is my call.
- The function `wiki_init` never overwrites. So the row needs no state read when the palette opens. This is my call.

### Documents

- Add a short section "In the desktop app" to `docs/content/using-kimun/wiki.md`. Add one line to `desktop-app.md`.
- Write ADR 0069. It records three facts. The function `wiki_configure` replaces the declaration, creates what is missing, and moves nothing. Item 15 of ADR 0068 is now built. New folder roles are an open question.
- If the glossary has a place for GUI screens, add one line for the Wiki section to `CONTEXT.md`, under the Wiki declaration entry.

## Constraints

- Core keeps all filesystem work. The GUI and the logic layer never write a file or make a folder.
- Paths use `VaultPath`. The case rules of vault paths apply to the folder fields.
- The section must work on Windows and macOS. It has no platform code.
- The GUI has no users yet, so wording and layout are free to change.
- All seven CI gates and the agent guidance check pass.
- The GUI is tested headless only. A hand check of the section in a real window stays with the user.

## Tests

- Core tests cover these cases:
  - Configure on a vault with no declaration.
  - Configure on a wiki with a new sources folder. The old folder and its notes stay. The new folder exists. The protection moves.
  - Configure on a vault with a broken file. The file is replaced.
  - An invalid declaration. It is rejected and the old file stays.
  - A note where a folder is wanted. It gives an error and the old file stays.
  - The report lines.
  - A log note outside the wiki folder, such as `/meta/log.md`, with a missing parent folder.
  - The schema template with a custom list of required properties.
- The `wiki_init` tests stay and pass through the shared function.
- Logic tests cover these cases:
  - Each state.
  - The defaults.
  - The log and schema following the wiki folder.
  - The field errors from `validate()`.
  - The effect notes.
  - The button label.
  - The equal-draft case, where Apply repairs the layout.
  - The schema note line in the effect note.
- GUI tests cover these cases:
  - The section list.
  - The states.
  - An apply that creates folders.
  - A failed apply.
  - The disabled states.
  - The keyboard flow.
  - The palette row: blocked, runs, already a wiki, and a broken declaration.
  - A repair apply after someone deleted the wiki folder.

## Left to the builder

- The names of types and functions.
- The element ids.
- The exact wording of labels and messages, inside the simple-english rules.
- The size and spacing of the fields.
- How the property-key rule is reused.
- The order of the work inside the three layers.
