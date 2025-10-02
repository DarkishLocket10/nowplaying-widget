# Development Guide

This document captures the workflows and conventions for working on the Now Playing Widget codebase.

## Environment Setup

1. Install the latest [Rust toolchain](https://www.rust-lang.org/tools/install) (stable channel recommended).
2. Install Microsoft Visual C++ Build Tools and the Windows 10/11 SDK. The project relies on the [`windows`](https://crates.io/crates/windows) crate, which requires native headers and libraries.
3. Clone the repository and change into the `app/` crate:

   ```powershell
   git clone https://github.com/<your-org>/nowplaying-widget.git
   cd nowplaying-widget/app
   ```

4. Verify the toolchain works:

   ```powershell
   cargo check
   ```

## Project Layout

- `src/main.rs` – Application entry point, rendering flow, playback polling, and widget UI.
- `src/layout.rs` – Layout engine parsing and representation.
- `src/theme.rs` – Theme loader, validation, and style resolution.
- `src/ui_skin.rs` – Skin manager (discovery, hot reload, egui styling helpers).
- `skins/` – Reference skins with their `theme.toml`, `layout.toml`, and assets.
- `docs/` – Documentation (this guide, skin authoring references).
- `tests/` – Integration scenarios verifying playback polling behaviour.

## Coding Standards

- Follow Rust 2021 idioms; prefer `?` for error propagation and keep functions focused.
- Document public structs and functions with Rustdoc when adding new modules.
- Match existing formatting and naming conventions; run `cargo fmt` before committing.
- Keep imports sorted logically (std, third-party, crate modules).
- Avoid `unwrap()`/`expect()` in non-test code unless failure is unrecoverable and a panic is acceptable.

## Common Tasks

### Build & Run

```powershell
cargo run
```

Use `cargo run --release` when validating performance or generating distributable binaries.

### Formatting & Linting

```powershell
cargo fmt
cargo clippy --all-targets --all-features
```

Address warnings surfaced by Clippy where practical.

### Testing

```powershell
cargo test
```

Integration tests live under `tests/`. Add new tests when fixing bugs or introducing behavior changes to the playback, layout, or theming pipelines.

### Asset & Skin Hot Reload

- Toggle **Hot Reload** in the widget settings drawer to watch skin directories for changes.
- Skins are reloaded when `theme.toml` or `layout.toml` is modified. Image changes require the file timestamp to update (overwrite or delete + re-add).
- Console warnings are logged if a reload fails; the UI will continue using the last valid skin.

### Logging & Diagnostics

- Non-fatal issues (missing assets, parse failures) are pushed into a warnings vector and displayed via the `Skin Warnings` component.
- When debugging playback integration, temporarily enable `env_logger` or print statements around the Windows media session interactions. Remove debugging output before committing.

## Platform Notes

### Windows title bar styling

- On Windows 11 and newer, the widget asks DWM to recolor the native caption, border, and text to match the active skin's root background.
- Title bar colors update automatically when skins change. The tint is derived from `components.root.background` and the caption text color falls back to white/black when a skin doesn't override it explicitly.
- Older Windows builds simply ignore these requests, so the system theme continues to own the title bar appearance.

## Release Checklist

1. Update documentation as needed (README, `docs/*.md`).
2. Run `cargo fmt`, `cargo clippy --all-targets --all-features`, and `cargo test`.
3. Build the release binary: `cargo build --release`.
4. Smoke test on a clean Windows machine with multiple media sources (Spotify, Edge, etc.).
5. Tag the release in Git and attach the binary if distributing externally.

## Contributing Skins

- Place new skin directories under `skins/<id>/`.
- Ensure both `theme.toml` and `layout.toml` compile without warnings (see `Skin Warnings` panel).
- Include a `README.md` inside the skin directory if it has non-obvious assets or licensing.

## Issue Triage

When filing or addressing issues, capture:

- Steps to reproduce (including media application and Windows version).
- Console output (`cargo run` terminal) and any on-screen warnings.
- Skin/layout being used.

Keeping diagnostics detailed speeds up triage and avoids regressions.

---

For end-user documentation and skin authoring details, direct readers to:

- [README](../README.md)
- [Theme & Asset Reference](theme.md)
- [Layout Engine Reference](layout.md)
