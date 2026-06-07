# Windows Tauri CI/CD — Design

**Date:** 2026-06-06
**Status:** Approved, pending implementation
**Repo:** diegohh0411/nebula

## Goal

Automate building the Windows installer that Tauri produces, with two flows:

1. **Release** — pushing a version tag (`v*`) to the repo builds the Windows
   bundle and publishes it as a (draft) GitHub Release with the installers
   attached as assets.
2. **PR test build** — opening/updating a pull request against `main` builds the
   same Windows bundle and uploads the installers as a workflow artifact for
   internal testing.

## Context

- App stack: **Tauri 2** + **Angular 20**, package manager **pnpm**.
- Frontend builds to `dist/nebula/browser` (`beforeBuildCommand: pnpm build`).
- Tauri `bundle.targets: "all"` → on Windows this produces an **MSI** (WiX) and
  an **NSIS `.exe`** installer.
- No `.github/` directory exists yet — both workflows are net-new.
- Identifier: `com.diegohh0411.nebula`; product name `nebula`.

## Decisions

| Topic | Decision |
|-------|----------|
| Trigger (release) | Push of a tag matching `v*` |
| Trigger (PR build) | `pull_request` targeting `main` |
| Build action | Official `tauri-apps/tauri-action@v0` |
| Release artifact | GitHub Release (draft) with installers as assets |
| PR artifact | `actions/upload-artifact` (14-day retention) |
| Code signing | **Skipped for now** — unsigned installers; add later via secrets |
| Node version | **22** (active LTS) for both workflows |
| Runner | `windows-latest` (ships Rust + MSVC tooling) |

### Rationale highlights

- **Two workflows, not one.** Release and PR-test are separate concerns; two
  files keep each path readable instead of tangling `if:` conditionals.
- **`tauri-action`** is Tauri-team maintained; it builds the bundle *and* (when
  given `tagName`) creates the Release and uploads assets natively, while also
  exposing artifact paths for the PR flow.
- **Node 22, not 20.** Tauri 2 imposes no Node requirement — the constraint is
  Angular 20 (`^20.19 || ^22.12 || ^24`). Node 22 is the active LTS; Node 20 is
  maintenance-only. Both workflows use the same version so a green PR build
  implies a working release build.
- **Draft release** so release notes can be reviewed before publishing.

## Architecture

Two workflow files under `.github/workflows/`, sharing an identical build
preamble (checkout → pnpm → Node 22 → Rust stable → rust-cache → `pnpm install`).
They differ only in trigger and publish step.

```
.github/workflows/
  release-windows.yml     # tag push v*  → tauri-action with tagName → draft Release
  pr-build-windows.yml    # PR to main   → tauri-action (build only) → upload-artifact
```

### `release-windows.yml`

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

### `pr-build-windows.yml`

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

## Error handling

- `if-no-files-found: error` on the PR artifact upload so an empty/failed bundle
  fails loudly rather than producing a green run with no installer.
- `pnpm install --frozen-lockfile` so CI fails if the lockfile is out of sync.
- Fork PRs lack write permissions, but the PR workflow only uploads artifacts
  (no release), so it degrades gracefully.

## Testing / verification

- Open a PR → confirm `PR Build Windows` runs and an `nebula-windows-<sha>`
  artifact with `.msi` + `.exe` appears on the run.
- Push a `v*` tag → confirm `Release Windows` runs and a **draft** GitHub Release
  with installer assets is created.

## Out of scope (future follow-ups)

- Code signing (cert in GitHub secrets, SmartScreen warning removal).
- macOS / Linux build targets.
- Shared Node version source of truth (`.nvmrc` / `engines`) for local + CI.
- Auto-generated release notes / changelog.
```
