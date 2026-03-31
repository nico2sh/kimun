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

Colors can be specified in three formats:

| Format | Example |
|---|---|
| 6-digit hex | `"#1e1e2e"` |
| 3-digit hex (shorthand) | `"#abc"` → expands to `#aabbcc` |
| RGB function | `"rgb(30, 30, 46)"` |

### Activating a Custom Theme

Once the file is saved, start (or restart) Kimün. Your theme will appear in the theme picker in the Settings screen alongside the built-in themes. You can also set it directly in `kimun_config.toml`:

```toml
[global]
theme = "My Theme"
```

The name must match the `name` field in your `.toml` file exactly.

### Overriding the Default Theme

If you save a file named `default.toml` in the themes directory, it will be loaded as an additional theme option. It does not replace the built-in default — to make it the active theme, set it in `[global]` as shown above.

## Example: Rosé Pine Theme

```toml
name = "Rosé Pine"

bg               = "#191724"
bg_panel         = "#1f1d2e"
bg_selected      = "#403d52"
fg               = "#e0def4"
fg_secondary     = "#908caa"
fg_muted         = "#6e6a86"
fg_selected      = "#e0def4"
border           = "#403d52"
border_focused   = "#c4a7e7"
accent           = "#c4a7e7"
color_directory    = "#9ccfd8"
color_journal_date = "#31748f"
color_search_match = "#f6c177"
```

Save this as `~/.config/kimun/themes/rose-pine.toml` and set `theme = "Rosé Pine"` in your config.
