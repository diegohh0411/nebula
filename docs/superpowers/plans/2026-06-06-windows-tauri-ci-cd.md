# Windows Tauri CI/CD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two GitHub Actions workflows that build the Windows Tauri installer — one triggered by `v*` tag pushes that publishes a draft GitHub Release, and one triggered by PRs to `main` that uploads the installers as a workflow artifact.

**Architecture:** Two standalone workflow files under `.github/workflows/`, sharing an identical build preamble (checkout → pnpm → Node 22 → Rust stable → rust-cache → `pnpm install`). They differ only in their publish step: the release workflow passes `tagName` to `tauri-action` which creates a draft GitHub Release; the PR workflow omits `tagName` and uploads the bundle via `actions/upload-artifact`.

**Tech Stack:** GitHub Actions, `tauri-apps/tauri-action@v0`, `pnpm/action-setup@v4`, `actions/setup-node@v4` (Node 22), `dtolnay/rust-toolchain@stable`, `swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, `actions/checkout@v4`.

---

## File Map

| Status | Path | Purpose |
|--------|------|---------|
| Create | `.github/workflows/release-windows.yml` | Tag-push workflow: builds bundle, publishes draft GitHub Release |
| Create | `.github/workflows/pr-build-windows.yml` | PR workflow: builds bundle, uploads 14-day artifact |

---

### Task 1: Scaffold `.github/workflows/` and create `release-windows.yml`

**Files:**
- Create: `.github/workflows/release-windows.yml`

- [ ] **Step 1: Create the directory structure**

```bash
mkdir -p .github/workflows
```

- [ ] **Step 2: Write `release-windows.yml`**

Create `.github/workflows/release-windows.yml` with this exact content:

```yaml
name: Release Windows

on:
  push:
    tags:
      - "v*"

jobs:
  build-windows:
    runs-on: windows-latest
    permissions:
      contents: write          # create the GitHub Release
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22      # keep in sync with pr-build-windows.yml
          cache: pnpm
      - uses: dtolnay/rust-toolchain@stable
      - uses: swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - run: pnpm install --frozen-lockfile
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: "Nebula ${{ github.ref_name }}"
          releaseDraft: true
          prerelease: false
```

- [ ] **Step 3: Verify file structure is correct**

```bash
grep -E "^name:|^on:|^jobs:|runs-on:|tauri-apps/tauri-action|releaseDraft" .github/workflows/release-windows.yml
```

Expected output (order may vary):
```
name: Release Windows
on:
jobs:
    runs-on: windows-latest
      - uses: tauri-apps/tauri-action@v0
          releaseDraft: true
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-windows.yml
git commit -m "feat(TT-32): add release-windows GitHub Actions workflow"
```

---

### Task 2: Create `pr-build-windows.yml`

**Files:**
- Create: `.github/workflows/pr-build-windows.yml`

- [ ] **Step 1: Write `pr-build-windows.yml`**

Create `.github/workflows/pr-build-windows.yml` with this exact content:

```yaml
name: PR Build Windows

on:
  pull_request:
    branches:
      - main

jobs:
  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22      # keep in sync with release-windows.yml
          cache: pnpm
      - uses: dtolnay/rust-toolchain@stable
      - uses: swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - run: pnpm install --frozen-lockfile
      - uses: tauri-apps/tauri-action@v0
        id: tauri
        # no tagName → build only, no release created
      - uses: actions/upload-artifact@v4
        with:
          name: nebula-windows-${{ github.sha }}
          path: |
            src-tauri/target/release/bundle/msi/*.msi
            src-tauri/target/release/bundle/nsis/*.exe
          retention-days: 14
          if-no-files-found: error
```

- [ ] **Step 2: Verify file structure is correct**

```bash
grep -E "^name:|^on:|^jobs:|runs-on:|tauri-apps/tauri-action|upload-artifact|if-no-files-found" .github/workflows/pr-build-windows.yml
```

Expected output (order may vary):
```
name: PR Build Windows
on:
jobs:
    runs-on: windows-latest
      - uses: tauri-apps/tauri-action@v0
      - uses: actions/upload-artifact@v4
          if-no-files-found: error
```

- [ ] **Step 3: Confirm both workflows are present**

```bash
ls -1 .github/workflows/
```

Expected:
```
pr-build-windows.yml
release-windows.yml
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/pr-build-windows.yml
git commit -m "feat(TT-32): add pr-build-windows GitHub Actions workflow"
```

---

### Task 3: Open PR and verify

**Files:** (no changes — operational steps only)

- [ ] **Step 1: Push the branch**

```bash
git push -u origin worktree-tt-32-windows-tauri-ci-cd
```

- [ ] **Step 2: Open a draft PR**

```bash
gh pr create \
  --title "feat(TT-32): Windows Tauri CI/CD — release & PR build GitHub Actions" \
  --body "$(cat <<'EOF'
## Summary
- Adds `.github/workflows/release-windows.yml`: triggers on `v*` tag push, builds Windows installer via `tauri-apps/tauri-action`, publishes a draft GitHub Release with `.msi` + NSIS `.exe` as assets.
- Adds `.github/workflows/pr-build-windows.yml`: triggers on PRs to `main`, builds the same Windows bundle, uploads installers as a 14-day workflow artifact (`nebula-windows-<sha>`).
- Both workflows share an identical preamble: pnpm + Node 22 (active LTS, required by Angular 20) + Rust stable + rust-cache.

## Test plan
- [ ] Open a PR → confirm `PR Build Windows` job starts on `windows-latest`
- [ ] Wait for run to finish → confirm a `nebula-windows-<sha>` artifact containing `.msi` + `.exe` appears in the run's artifact list
- [ ] Push a `v*` tag to the branch (e.g. `v0.1.0-test`) → confirm `Release Windows` job starts
- [ ] Confirm a **draft** GitHub Release named `Nebula v0.1.0-test` is created with installer assets attached

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Update Notion task TT-32 status to "Ready for review" and record PR number**

Get the PR number from the previous step's output, then:

```
notion-update-page(
  id="378e954d-b476-81e5-9865-f100aa2a9c2f",
  properties={
    "Status": {"status": {"name": "Ready for review"}},
    "PR number": {"number": <PR_NUMBER>}
  }
)
```

---

## Verification Checklist (from spec)

- [ ] PR build: `PR Build Windows` workflow runs, `nebula-windows-<sha>` artifact with `.msi` + `.exe` appears
- [ ] Release build: `Release Windows` workflow runs on `v*` tag, **draft** GitHub Release with installer assets created
- [ ] `pnpm install --frozen-lockfile` failure is loud (lockfile drift fails CI)
- [ ] `if-no-files-found: error` is present in artifact upload (failed bundle fails loudly, not silently)
