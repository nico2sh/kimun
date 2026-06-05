+++
title = "Themes"
weight = 12
+++

# Themes

Kimün ships with several built-in themes and supports fully custom themes defined as TOML files.

## Built-in Themes

The following themes are included out of the box:

- **Gruvbox Dark** *(default)*
- **Gruvbox Light**
- **Catppuccin Mocha**
- **Catppuccin Latte**
- **Tokyo Night**
- **Tokyo Night Storm**
- **Solarized Dark**
- **Solarized Light**
- **Nord**
- **Dracula**
- **Alucard** — Dracula's official light variant
- **One Dark**
- **One Light**
- **Monokai**
- **Everforest Dark**
- **Everforest Light**
- **Rosé Pine**
- **Rosé Pine Dawn** — Rosé Pine's light variant
- **Kanagawa Wave**
- **Kanagawa Lotus** — Kanagawa's light variant
- **ANSI** — adapts to your terminal's configured 16-color palette (works for both light and dark terminal themes)

Switch with the **live theme picker** — `Ctrl+G v t` (or the CFG drawer's `t`): moving the selection restyles the whole app instantly, Enter persists, Esc reverts. You can also set the `theme` field at the top level of your config file:

```toml
theme = "Nord"
```

### Color depth

Themes adapt automatically to what your terminal supports: truecolor where available, quantized to 256 colors, or mapped onto the 16 ANSI slots on basic terminals (where every theme effectively becomes the **ANSI** theme, following your terminal's palette).

## Creating a Custom Theme

### Theme File Location

Place custom theme files in the `themes/` subdirectory of your Kimün config directory:

- **Linux / macOS:** `~/.config/kimun/themes/`
- **Windows:** `%USERPROFILE%\kimun\themes\`

Each file must have a `.toml` extension. The filename is not significant — the theme's display name comes from the `name` field inside the file.

### Theme File Format

A theme file is a TOML file with a `name` field and up to 27 color roles. **Only `name` and the core roles you want to change are required** — anything omitted derives from a sensible sibling (e.g. `bg_hard`/`bg_soft` derive from `bg`, `focus_border` from `green`, `selection_fg` from `fg_bright`), so a minimal theme can be just a handful of lines.

```toml
name = "My Theme"

# Backgrounds
bg            = "#1e1e2e"   # main/editor background
bg_hard       = "#11111b"   # modals and input fields (harder contrast)
bg_soft       = "#24243a"   # alternating rows, horizontal rules
bg_panel      = "#181825"   # drawer / panel background
selection_bg  = "#313244"   # selected row background

# Text
fg            = "#cdd6f4"   # primary text
fg_bright     = "#f5f7ff"   # titles, headings
fg_secondary  = "#a6adc8"   # filenames, metadata, hints
gray          = "#6c7086"   # placeholders, separators, disabled
selection_fg  = "#f5f7ff"   # text on selected rows

# Chrome
border_dim    = "#45475a"   # unfocused borders
focus_border  = "#a6e3a1"   # focused borders (the green frame)
accent        = "#89b4fa"   # title bars, active markers
cursor        = "#f5e0dc"   # block cursor in text fields

# Accent palette (query highlighting, status, markdown)
red           = "#f38ba8"   # errors, query negation
green         = "#a6e3a1"   # success, quoted query terms
yellow        = "#f9e2af"   # warnings, field keys, keycaps
blue          = "#89b4fa"   # wikilink targets
purple        = "#cba6f7"   # numbers and dates
aqua          = "#94e2d5"   # tags, group labels
orange        = "#fab387"   # operators, strong accents

# Semantic
color_directory    = "#89dceb"  # directory rows in the file list
color_journal_date = "#94e2d5"  # journal date annotations
color_search_match = "#a6e3a1"  # highlighted search matches
color_tag          = "#fab387"  # #hashtag spans in the editor
blockquote_bar     = "#585b70"  # the ▏ bar replacing > markers
code_bg            = "#181825"  # fenced/indented code-block background
```

### Color Formats

Colors can be specified in the following formats:

| Format | Example | Notes |
|---|---|---|
| 6-digit hex | `"#1e1e2e"` | |
| 3-digit hex (shorthand) | `"#abc"` | Expands to `#aabbcc` |
| RGB function | `"rgb(30, 30, 46)"` | |
| ANSI index | `"ansi:4"` | 0-255 |
| Terminal default | `"reset"` | Uses the terminal's default fg or bg |

### Activating a Custom Theme

Once the file is saved, start (or restart) Kimün. Your theme appears in the theme picker (`Ctrl+G v t`) alongside the built-in ones. You can also set it directly in `config.toml`:

```toml
theme = "My Theme"
```

The name must match the `name` field in your `.toml` file exactly.

### Overriding the Default Theme

If you save a file named `default.toml` in the themes directory, it will be loaded as an additional theme option. It does not replace the built-in default — to make it the active theme, set it in `[global]` as shown above.

## Example: Ayu Dark Theme (minimal)

Derivation fills everything not listed:

```toml
name = "Ayu Dark"

bg           = "#0a0e14"
bg_panel     = "#0d1017"
selection_bg = "#273747"
fg           = "#b3b1ad"
fg_secondary = "#6c7380"
gray         = "#4d5566"
border_dim   = "#11151c"
accent       = "#e6b450"

red    = "#d95757"
green  = "#7fd962"
yellow = "#e6b450"
blue   = "#39bae6"
purple = "#d2a6ff"
aqua   = "#95e6cb"
orange = "#ff8f40"
```

Save this as `~/.config/kimun/themes/ayu-dark.toml` and set `theme = "Ayu Dark"` in your config.
