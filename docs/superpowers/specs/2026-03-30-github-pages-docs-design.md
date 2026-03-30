# GitHub Pages Docs Deployment Design

**Date:** 2026-03-30
**Status:** Approved

## Overview

Deploy the Kimun mdbook documentation to GitHub Pages using the `gh-pages` branch strategy. The built HTML is pushed to a dedicated `gh-pages` branch on every change to `docs/` on `main`, keeping generated output separate from source.

## Changes

### 1. `docs/book.toml`

Fill in the GitHub repository metadata (currently placeholders):

- `git-repository-url` → `https://github.com/nico2sh/kimun`
- `edit-url-template` → `https://github.com/nico2sh/kimun/edit/main/docs/src/{path}`

### 2. `.github/workflows/docs.yml` (new file)

Trigger: push to `main`, path filter `docs/**`.

Steps:
1. Checkout repository
2. Install `mdbook` via `cargo install` or a prebuilt action
3. Run `mdbook build` inside the `docs/` directory (output goes to `docs/book/`)
4. Upload `docs/book/` as a Pages artifact (`actions/upload-pages-artifact`)
5. Deploy to `gh-pages` branch (`actions/deploy-pages`)

The workflow requires `pages: write` and `id-token: write` permissions.

### 3. GitHub Pages settings (one-time manual step)

In repo Settings → Pages:
- Source: **GitHub Actions** (not branch — this pairs with `actions/deploy-pages`)

## Result

Documentation published at: `https://nico2sh.github.io/kimun/`

Subsequent pushes to `docs/**` on `main` automatically rebuild and redeploy.
