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

Switch between them via the Settings screen in the TUI, or by setting the `theme` field in your config file:

```toml
[global]
theme = "Nord"
```

## Creating a Custom Theme

### Theme File Location

Place custom theme files in the `themes/` subdirectory of your Kimün config directory:

- **Linux / macOS:** `~/.config/kimun/themes/`
- **Windows:** `%USERPROFILE%\kimun\themes\`

Each file must have a `.toml` extension. The filename is not significant — the theme's display name comes from the `name` field inside the file.

### Theme File Format

A theme file is a TOML file with a `name` field and 13 color fields:

```toml
name = "My Theme"

# Background colors
bg               = "#1e1e2e"   # Main background (editor area)
bg_panel         = "#181825"   # Sidebar and panel background
bg_selected      = "#313244"   # Selected row background

# Text colors
fg               = "#cdd6f4"   # Primary text
fg_secondary     = "#a6adc8"   # Filenames, metadata, hints
fg_muted         = "#6c7086"   # Placeholders, separators, disabled text
fg_selected      = "#cdd6f4"   # Text on selected rows

# Border colors
border           = "#45475a"   # Unfocused borders
border_focused   = "#89b4fa"   # Focused borders

# Accent
accent           = "#89b4fa"   # Title bars, cursor, markers

# Semantic colors
color_directory    = "#89dceb" # Directory entries in the file list
color_journal_date = "#94e2d5" # Journal date annotations
color_search_match = "#a6e3a1" # Highlighted search match text
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

Once the file is saved, start (or restart) Kimün. Your theme will appear in the theme picker in the Settings screen alongside the built-in themes. You can also set it directly in `config.toml`:

```toml
[global]
theme = "My Theme"
```

The name must match the `name` field in your `.toml` file exactly.

### Overriding the Default Theme

If you save a file named `default.toml` in the themes directory, it will be loaded as an additional theme option. It does not replace the built-in default — to make it the active theme, set it in `[global]` as shown above.

## Example: Ayu Dark Theme

```toml
name = "Ayu Dark"

bg               = "#0a0e14"
bg_panel         = "#0d1017"
bg_selected      = "#273747"
fg               = "#b3b1ad"
fg_secondary     = "#6c7380"
fg_muted         = "#4d5566"
fg_selected      = "#b3b1ad"
border           = "#11151c"
border_focused   = "#e6b450"
accent           = "#e6b450"
color_directory    = "#39bae6"
color_journal_date = "#95e6cb"
color_search_match = "#c2d94c"
```

Save this as `~/.config/kimun/themes/ayu-dark.toml` and set `theme = "Ayu Dark"` in your config.
