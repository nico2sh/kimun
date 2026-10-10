+++
title = "Themes"
weight = 14
+++

# Themes

Kimün ships with 48 built-in themes, and you can write your own as a small TOML file.

To pick one, open the theme picker with `Ctrl+B v t` (or `t` in the CFG drawer). Moving the selection restyles the app as you go; Enter saves the choice and Esc reverts it. You can also set the theme in your config:

```toml
theme = "Nord"
```

## Built-in Themes

| Family | Dark | Light |
|---|---|---|
| Gruvbox | **Gruvbox Dark** *(default)* | Gruvbox Light |
| Catppuccin | Catppuccin Mocha, Catppuccin Macchiato, Catppuccin Frappé | Catppuccin Latte |
| Tokyo Night | Tokyo Night, Tokyo Night Storm, Tokyo Night Moon | Tokyo Night Day |
| Solarized | Solarized Dark | Solarized Light |
| Dracula | Dracula | Alucard *(Dracula's official light variant)* |
| One | One Dark | One Light |
| Everforest | Everforest Dark | Everforest Light |
| Rosé Pine | Rosé Pine, Rosé Pine Moon | Rosé Pine Dawn |
| Kanagawa | Kanagawa Wave, Kanagawa Dragon | Kanagawa Lotus |
| Nord | Nord | — |
| Monokai | Monokai | — |
| GitHub | GitHub Dark, GitHub Dark Dimmed | GitHub Light |
| Ayu | Ayu Dark, Ayu Mirage | Ayu Light |
| Material | Material, Material Palenight, Material Ocean | — |
| Night Owl | Night Owl | Night Owl Light |
| Nightfox | Nightfox, Carbonfox | Dayfox |
| Oxocarbon | Oxocarbon Dark | Oxocarbon Light |
| Others | Zenburn, Cobalt2, Synthwave '84, Everblush | Quiet Light |
| ANSI | *uses your terminal's 16-color palette, light or dark* | |

### Color depth

Themes adapt to what your terminal supports: truecolor where available, quantized to 256 colors otherwise, or mapped onto the 16 ANSI slots on basic terminals. On a 16-color terminal every theme behaves like the ANSI theme and follows your terminal's palette.

## Creating a Custom Theme

### Theme File Location

Place custom theme files in the `themes/` subdirectory of your Kimün config directory:

- **Linux / macOS:** `~/.config/kimun/themes/`
- **Windows:** `%USERPROFILE%\kimun\themes\`

Each file must have a `.toml` extension. The filename doesn't matter; the theme's display name comes from the `name` field inside the file.

### Theme File Format

A theme file is a TOML file with a `name` field and up to 27 color roles. Only `name` and the core roles you want to change are required. Any role you omit is derived from a related one (for example, `bg_hard` and `bg_soft` derive from `bg`, `focus_border` from `green`, and `selection_fg` from `fg_bright`), so a minimal theme can be a few lines long.

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
color_replace_preview = "#3e3450"  # background behind replace-preview text
```

`color_replace_preview` is a background color, so give it a muted shade rather than an accent color. It marks the text the replace preview shows in place of a match (text that is not in the note yet), so it should be clearly distinct from `selection_bg` and `code_bg`. If you omit it, it falls back to `accent`, which makes a very loud highlight.

### Color Formats

Colors can use these formats:

| Format | Example | Notes |
|---|---|---|
| 6-digit hex | `"#1e1e2e"` | |
| 3-digit hex (shorthand) | `"#abc"` | Expands to `#aabbcc` |
| RGB function | `"rgb(30, 30, 46)"` | |
| ANSI index | `"ansi:4"` | 0-255 |
| Terminal default | `"reset"` | Uses the terminal's default fg or bg |

### Activating a Custom Theme

Once the file is saved, start (or restart) Kimün. Your theme appears in the theme picker (`Ctrl+B v t`) alongside the built-in ones. You can also set it directly in `config.toml`:

```toml
theme = "My Theme"
```

The name must match the `name` field in your `.toml` file exactly.

### Overriding the Default Theme

If you save a file named `default.toml` in the themes directory, it will be loaded as an additional theme option. It does not replace the built-in default. To make it the active theme, set the top-level `theme` key to its `name`, as shown above.

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

Theme files are plain TOML, so you can share them by copying the file. For other settings, see [Configuration](@/getting-started/configuration.md).
