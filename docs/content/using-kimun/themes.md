+++
title = "Themes"
weight = 15
+++

# Themes

Kimün has 69 built-in themes. You can also make your own.

## Pick a theme

In the terminal app, press `Ctrl+B v t`. In the desktop app, press `Ctrl+Shift+B v t`. Type to filter the list. The colors change as you move through the list. Click `Apply` or press `Enter` to keep a theme. Click `Cancel` or press `Esc` to go back.

![The theme picker in the desktop app](/img/gui-theme-picker.png)

You can also set the theme in `config.toml`:

```toml
theme = "Nord"
```

## Built-in themes

The default is Gruvbox Dark. These theme families are included. Most have both a dark and a light theme:

Gruvbox, Catppuccin, Tokyo Night, Solarized, Dracula, One, Everforest, Rosé Pine, Kanagawa, Nord, Monokai, GitHub, Ayu, Material, Night Owl, Nightfox, Oxocarbon, Flexoki, Selenized, Modus, Melange, Tomorrow, PaperColor, Edge, VS Code, Zenburn, Cobalt2, Synthwave '84, Everblush, Sonokai, Vesper, Iceberg, Quiet Light, and Kimün.

The `ANSI` theme uses the colors of your terminal. Use it if you want Kimün to match your terminal.

On a terminal with few colors, every theme looks like the `ANSI` theme.

## Make your own theme

Save a `.toml` file in the `themes/` folder next to your `config.toml`:

- Linux and macOS: `~/.config/kimun/themes/`
- Windows: `%USERPROFILE%\kimun\themes\`

Only `name` is required. Kimün picks the other colors from the ones that you give.

```toml
name = "My Theme"

bg           = "#1e1e2e"   # background
fg           = "#cdd6f4"   # text
fg_bright    = "#f5f7ff"   # headings
accent       = "#89b4fa"   # title bars and active items
selection_bg = "#313244"   # selected row
red          = "#f38ba8"
green        = "#a6e3a1"
yellow       = "#f9e2af"
blue         = "#89b4fa"
purple       = "#cba6f7"
aqua         = "#94e2d5"
orange       = "#fab387"
```

You can write a color as `"#1e1e2e"`, `"#abc"`, `"rgb(30, 30, 46)"`, or `"ansi:4"`.

You can also set these optional keys. If you leave one out, Kimün uses a default:

```toml
focus_border = "#89b4fa"   # focused panel edge, default: accent
border       = "#45475a"   # panel edge at rest, default: border_dim
header       = "#a6e3a1"   # tint of a panel header in the desktop app
selected     = "#cba6f7"   # tint behind a chosen item in the desktop app
heading      = "#f9e2af"   # level 1 and 2 headings, default: accent
link         = "#89dceb"   # links and images, default: accent
code         = "#94e2d5"   # inline code, default: aqua
```

Your theme then appears in the theme picker. To use it, pick it, or set `theme = "My Theme"` in `config.toml`.
