# GitHub Pages Docs Deployment Design

**Date:** 2026-03-30
**Status:** Approved

## Overview

Deploy the Kimun mdbook documentation to GitHub Pages using the GitHub Actions deployment model. A workflow builds the book on every change to `docs/` on `main`, uploads the output as a Pages artifact, and deploys it via `actions/deploy-pages`. No `gh-pages` branch is involved; GitHub Pages is configured to use the "GitHub Actions" source.

## Changes

### 1. `docs/book.toml`

Fill in the GitHub repository metadata (currently placeholders):

- `git-repository-url` → `https://github.com/nico2sh/kimun`
- `edit-url-template` → `https://github.com/nico2sh/kimun/edit/main/docs/src/{path}`

### 2. `.github/workflows/docs.yml` (new file)

Trigger: push to `main`, path filter `docs/**`.

Permissions required:
- `contents: read`
- `pages: write`
- `id-token: write`

Steps:
1. Checkout repository
2. Install `mdbook` using a prebuilt binary (e.g., `taiki-e/install-action@mdbook`) — avoids slow `cargo install` compilation
3. Run `mdbook build` inside the `docs/` directory (output goes to `docs/book/`, the default since `book.toml` does not override `build.build-dir`)
4. Upload `docs/book/` as a Pages artifact (`actions/upload-pages-artifact`)
5. Deploy via the GitHub Pages Actions environment (`actions/deploy-pages`)

### 3. GitHub Pages settings (one-time manual step)

In repo Settings → Pages:
- Source: **GitHub Actions**

## Result

Documentation published at: `https://nico2sh.github.io/kimun/`

Subsequent pushes to `docs/**` on `main` automatically rebuild and redeploy.
