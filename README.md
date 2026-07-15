# Nebula

Your photos, organized offline.

Nebula is a desktop photo manager for people who do not want to ship their memories to a cloud. Indexing, face detection, subject clustering, and semantic search all run on your machine. After the first model download, no internet is required.

## The problem

Cloud photo libraries are convenient until they are not. They need bandwidth, they mine your data, and they can disappear or lock you out. For photographers working off the grid, or anyone who would rather keep their library at home, that model does not fit.

Nebula started as a personal project while shooting in Pisgah National Forest, where 2G signal was the best available and uploading thousands of photos was not an option. It is built to make a large local collection searchable and manageable without touching the internet.

## What it does

- Add folders from anywhere on disk. Nebula watches them, rescans on startup, and keeps a SQLite catalog of every image.
- Detect faces with Antelope V2 or InsightFace Buffalo models, then group them into people using semi-supervised clustering. You can name subjects, merge duplicates, and correct assignments.
- Search photos by describing them in plain text, dropping an image into the search bar, or pasting one from the clipboard. SigLIP 2 handles the matching.
- Browse photos by tag or person, with a timeline scrubber, lightbox, and gallery view.
- Run fully offline after the initial model download. DirectML GPU offloading is available for the vision model on Windows.
- Trade speed for accuracy by swapping embedding and face-recognition models in settings.

## Tech stack

- Frontend: Angular 20, Tailwind CSS, Spartan UI, and Lucide icons
- Desktop shell: Tauri 2.0
- Backend: Rust with sqlx, tokio, notify, and ort (ONNX Runtime)
- ML: SigLIP 2 for image and text embeddings; Antelope V2 and Buffalo for face detection and recognition
- Database: SQLite with sqlite-vec for vector search
- Build tooling: pnpm, Vite, and Vitest

## Getting started

You need Node.js, pnpm, and the Rust toolchain installed.

```bash
pnpm install
pnpm tauri dev
```

To build a release binary:

```bash
pnpm tauri build
```

Nebula stores its catalog, cached thumbnails, and downloaded models in the platform app data directory.

## Contributing

Issues and pull requests are welcome. The backend is organized into vertical slices under `src-tauri/src/` (`library`, `people`, `tags`, `search`, `media`, `pipeline`, `vision`, `settings`). Please keep domain queries in the relevant slice and add tests for new behavior.

### Git hooks (lefthook)

This repo uses [lefthook](https://github.com/evilmartians/lefthook) to run checks locally before they hit CI. After cloning, install the hooks once:

```bash
# install the lefthook CLI if you don't have it (pick one)
brew install lefthook          # macOS
cargo install lefthook         # any platform with a Rust toolchain
npm install -g lefthook        # any platform with Node

# then, from the repo root
lefthook install
```

This wires up:

- **`pre-commit`** — runs `cargo fmt --all` against `src-tauri` whenever staged `*.rs` files are found, and re-stages the formatted files. This is the same check the `rust` CI job runs (`cargo fmt --all --check`), so formatting issues are fixed before they ever reach CI.
- **`pre-push`** — runs `cargo clippy --all-targets -- -D warnings` against `src-tauri`, matching the CI clippy gate. It runs on push rather than every commit since clippy is slower than fmt.

You can skip hooks for a single commit/push with `LEFTHOOK=0 git commit ...` if you need to.

## License

Copyright (C) 2026 Diego Hernández Herrera. Released under the GNU General Public License v3. See `LICENSE` for details.
