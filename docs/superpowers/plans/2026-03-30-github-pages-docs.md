# GitHub Pages Docs Deployment Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy the Kimun mdbook documentation to GitHub Pages automatically on every push to `main` that touches `docs/`.

**Architecture:** Update `docs/book.toml` with the real GitHub repo URL, then add a new GitHub Actions workflow that installs mdbook, builds the book, and deploys the output via the GitHub Pages Actions artifact model (`actions/upload-pages-artifact` + `actions/deploy-pages`). No `gh-pages` branch is used — GitHub Pages is configured with source set to "GitHub Actions".

**Tech Stack:** mdbook, GitHub Actions (`actions/checkout@v4`, `taiki-e/install-action@v2`, `actions/upload-pages-artifact`, `actions/deploy-pages`)

---

## Chunk 1: Update book.toml and add docs workflow

**Files:**
- Modify: `docs/book.toml`
- Create: `.github/workflows/docs.yml`

---

### Task 1: Update `docs/book.toml` with real repo URLs

- [ ] **Step 1: Edit `docs/book.toml`**

  Replace the two commented-out lines with the real values:

  ```toml
  [book]
  title = "Kimün"
  authors = []
  language = "en"
  src = "src"

  [output.html]
  git-repository-url = "https://github.com/nico2sh/kimun"
  edit-url-template = "https://github.com/nico2sh/kimun/edit/main/docs/src/{path}"

  [output.html.search]
  enable = true
  limit-results = 20
  use-boolean-and = true
  ```

- [ ] **Step 2: Verify mdbook builds locally**

  Run from repo root:
  ```bash
  cd docs && mdbook build
  ```
  Expected: `book/` directory created with no errors. If `mdbook` is not installed locally, skip this step — it will be validated by the workflow run.

- [ ] **Step 3: Commit**

  ```bash
  git add docs/book.toml
  git commit -m "docs: set GitHub repo URL in book.toml"
  ```

---

### Task 2: Create `.github/workflows/docs.yml`

- [ ] **Step 1: Create the workflow file**

  Create `.github/workflows/docs.yml` with this exact content:

  ```yaml
  # Docs workflow: Build and deploy mdbook documentation to GitHub Pages
  # Triggered on pushes to main that touch docs/
  on:
    push:
      branches: [main]
      paths:
        - "docs/**"
    workflow_dispatch:

  name: docs

  permissions:
    contents: read
    pages: write
    id-token: write

  # Allow only one concurrent deployment
  concurrency:
    group: pages
    cancel-in-progress: false

  jobs:
    deploy:
      name: Build and Deploy Docs
      runs-on: ubuntu-latest
      environment:
        name: github-pages
        url: ${{ steps.deployment.outputs.page_url }}

      steps:
        - name: Checkout repository
          uses: actions/checkout@v4

        - name: Install mdbook
          uses: taiki-e/install-action@v2
          with:
            tool: mdbook

        - name: Build docs
          run: mdbook build docs/

        - name: Upload Pages artifact
          uses: actions/upload-pages-artifact@v3
          with:
            path: docs/book/

        - name: Deploy to GitHub Pages
          id: deployment
          uses: actions/deploy-pages@v4
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/docs.yml
  git commit -m "ci: add GitHub Pages docs deployment workflow"
  ```

- [ ] **Step 3: Push to main**

  ```bash
  git push origin main
  ```

  > **Note:** The workflow will not deploy successfully until GitHub Pages is configured in repo settings (Task 3 below).

---

### Task 3: Enable GitHub Pages in repo settings (manual)

This step must be done by a human with admin access to the repo.

- [ ] **Step 1:** Go to `https://github.com/nico2sh/kimun/settings/pages`
- [ ] **Step 2:** Under **Build and deployment → Source**, select **GitHub Actions**
- [ ] **Step 3:** Save

- [ ] **Step 4: Re-run the workflow (if it already ran and failed)**

  If the workflow ran before Pages was enabled, trigger a re-run:
  ```bash
  gh workflow run docs.yml --repo nico2sh/kimun
  ```
  Or re-run from the GitHub Actions UI.

- [ ] **Step 5: Verify deployment**

  After the workflow completes successfully, visit:
  `https://nico2sh.github.io/kimun/`

  Expected: The Kimün documentation site loads.
