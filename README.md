# Now Playing Widget

<table>
  <tr>
    <td><img src="https://github.com/user-attachments/assets/be3cb518-4228-4a6a-81f3-c126b8469993" alt="Now Playing 1" width="150" height="463"></td>
    <td><img src="https://github.com/user-attachments/assets/ac819988-0dc2-4a8a-8b82-ad0808d11cc6" alt="Now Playing 2" width="150" height="270"></td>
    <td><img src="https://github.com/user-attachments/assets/32bbb3b6-489f-4613-925f-9c78461cb919" alt="New player view" width="500" height="298">
</td>
  </tr>

  <tr>
    <td colspan="2" align="center">
      <img src="https://github.com/user-attachments/assets/93271fa9-9dea-42e5-b758-1a83b4577be6" alt="Player wide 1" width="360" height="208">
    </td>
    <td align="center">
      <img src="https://github.com/user-attachments/assets/2ecca9ea-187c-4700-b4bd-f3ef3a293fdf" alt="Player wide 2" width="360" height="268">
    </td>
  </tr>
</table>

A desktop widget for Windows that surfaces the current playback session using a customizable Rust/egui interface. It mirrors the Windows system media controls, shows album art and metadata, and lets you drive playback with modern, skinable controls.

I was tired of the current media control dashboards, so I just made a super minimal one in Rust, with a theme and layout engine, so users can customize their own skins to their liking.

## Table of Contents

- [Features](#features)
- [Requirements](#requirements)
- [Quick Start](#quick-start)
- [Usage](#usage)
- [Skinning and Layouts](#skinning-and-layouts)
- [Project Structure](#project-structure)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [Additional Resources](#additional-resources)

## Features

- **Windows system media integration** via the Global System Media Transport Controls API.
- **Dynamic album art** with automatic scaling, rounded corners, and optional decorative borders.
- **Responsive layouts and skins** defined entirely in TOML with hot reload support.
- **Modern controls** using glyph-based transport buttons and keyboard/mouse-friendly spacing.
- **Settings drawer** for switching skins, layouts, and toggling hot reload at runtime.
- **Vinyl thumbnail renderer**: swirl-distorted disc with spinning animation toggle it on the fly or let skins opt out entirely.
- **Safety-first error handling** with in-app warnings surfaced when skins or assets are misconfigured.

## Requirements

- Windows 10 or later (uses Windows Runtime media APIs).
- [Rust](https://www.rust-lang.org/tools/install) 1.76 or newer with the `cargo` build tool.
- Microsoft Visual C++ Build Tools (required by the `windows` crate when building from source).

## Quick Start

1. Clone the repository:

   ```powershell
   git clone https://github.com/<your-org>/nowplaying-widget.git
   cd nowplaying-widget/app
   ```

2. Build and run in debug mode:

   ```powershell
   cargo run
   ```

   Use `cargo run --release` for a production-ready binary.

3. The widget will appear with the default skin (Cutesy Pastels). Press the gear icon to open the settings drawer and experiment with alternative skins/layouts.

## Usage

- **Playback controls**: Previous, Play/Pause, and Next buttons map directly to the active media session.
- **Timeline**: Displays current position, duration, and allows seeking when supported by the session.
- **Settings drawer**: Use the left-aligned gear button to toggle. You can switch skins, choose a layout variant, enable hot reload, and flip between vinyl and standard artwork.
- **Artwork display**: Click the album art itself to swap between the spinning vinyl disc and the original square thumbnail.
- **Skin warnings**: When a skin fails to load assets or references missing values, a warning panel appears. Expand it to debug issues quickly.

## Skinning and Layouts

Skin authors can tailor every visual aspect:

- **Theme** (`theme.toml`): Controls colors, typography, button styles, slider behavior, and album art framing (rounded corners and optional border PNGs).
- **Layout** (`layout.toml`): Declares how components are arranged for each variant (rows, columns, and responsive parameters).
- **Assets** (`assets/`): Store fonts, images, slider thumbs, and decorative overlays for per-skin customization.

Bundled reference skins:

- **Cutesy Pastels** | default playful look with soft gradients.
- **Graphite Mono** | dark, minimal theme tuned for desktops.
- **Mobile Glow** | compact layout optimized for narrower windows.
- **Gradient Demo** | showcases the configurable gradient background support.
- **Aurora Vinyl** | neon turntable aesthetic designed to spotlight the vinyl thumbnail renderer.

See the following guides for in-depth skin authoring details:

- [Theme & Asset Reference](docs/theme.md)
- [Layout Engine Reference](docs/layout.md)

## Project Structure

```
app/
├── src/                # Application entry point and rendering logic
├── skins/              # Bundled skins, each with theme/layout/assets
├── assets/fonts/       # Shared font assets (Lato regular/bold)
├── docs/               # Project documentation
├── tests/              # Integration tests
├── Cargo.toml          # Rust crate manifest
└── README.md           # This file
```

## Development

- Format code with `cargo fmt` and lint via `cargo clippy` (optional but recommended).
- Run unit/integration tests with `cargo test`.
- Use `cargo run` while editing skins; enable hot reload from the widget settings drawer to live-reload TOML changes.
- Vinyl rendering is enabled when the active skin allows it; you can switch modes from the UI or pin a default in `config.toml` (see below).
- Refer to [docs/development.md](docs/development.md) for detailed contributor guidelines, coding standards, and release steps.

### Configuration

Drop a `config.toml` in the repository root (alongside `Cargo.toml`) or beside the built binary to customize experimental UI features:

```toml
[ui]
[ui.vinyl_thumbnail]
enabled = true        # preferred startup mode when the skin allows vinyl
swirl_strength = 2.5  # radians of angular distortion at the outer edge
label_ratio = 0.35    # radius of the untouched center label (0.1 to 0.6)
```

The vinyl renderer is **interactive**. It transforms album artwork into a spinning vinyl disc with polar-coordinate swirl, concentric grooves, center label preservation, subtle sheen, and a spindle hole. Click the artwork (or use the settings drawer toggle) to fall back to the untouched thumbnail at any time. The disc rotates in real-time during playback and respects the system's reduced-motion preference on Windows.

Skins can explicitly disable vinyl rendering by setting `disable_vinyl_thumbnail = true` in their `[meta]` section (see `docs/theme.md`).

## Troubleshooting

| Symptom | Resolution |
|---------|------------|
| Widget launches but shows "Unknown" state | Ensure a media session is active (Spotify, Groove, etc.). |
| Album art missing or blank | Verify the media session provides artwork; otherwise the widget displays a placeholder panel. |
| Skin fails to load | Check the on-screen warnings and inspect the referenced file paths in the skin’s `assets` directory. |
| Build errors referencing `windows` crate | Install the latest Windows SDK and C++ build tools, then retry `cargo run`. |

## Additional Resources

- [Egui documentation](https://docs.rs/egui/latest/egui/)
- [Windows Global System Media Transport Controls API](https://learn.microsoft.com/windows/win32/api/mfmediaengine/ne-mfmediaengine-mf_media_engine_event)
- [Rust book](https://doc.rust-lang.org/book/)


