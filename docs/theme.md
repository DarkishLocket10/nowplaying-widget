# Theme & Asset Reference

This guide explains how to author `theme.toml` files and skin assets for the Now Playing Widget. Themes control the visual style of every component, while layouts (documented separately in [layout.md](layout.md)) arrange those components on screen.

## Skin Folder Anatomy

Each skin lives under `app/skins/<skin-id>/` with the following structure:

```
skins/<skin-id>/
├── theme.toml      # Visual styling described in this document
├── layout.toml     # Layout variants and component tree (see layout.md)
└── assets/         # Supporting files (images, slider thumbs, borders, fonts)
```

The widget hot-reloads skins whenever `theme.toml` or `layout.toml` changes (enable the toggle in the settings drawer).

## Theme Document Schema

`theme.toml` is divided into a metadata section, color/variable tables, and a `components` block describing UI elements.

### Metadata

```toml
use_gradient = false      # Optional: disable album-art gradients (defaults to true)

[meta]
engine = "1"          # Required. Theme engine version (keep at "1" for now).
name = "graphite"     # Machine-friendly identifier (defaults to folder name).
display_name = "Graphite"  # Shown to users in the settings drawer.
disable_vinyl_thumbnail = false  # Optional: set to true to explicitly disable the vinyl renderer for this skin.
```

### Color and Variable Tables

Colors and variables are string-interpolated throughout the document. You can reference entries with `{colors.some_key}` or `{vars.some_key}`.

```toml
[colors]
background = "#10121a"
accent = "rgba(76, 141, 255, 1)"
text_on_accent = "#081123"

[vars]
radius = "18"
slider_thumb_radius = "10"
```

Supported color formats: `#RRGGBB`, `#RRGGBBAA`, `rgb(r,g,b)`, `rgba(r,g,b,a)`, or the literal `transparent`.

Variables must parse to floating-point numbers and are typically used for border radii, spacing, or font sizes.

> **Dynamic gradients**
>
> The widget can derive background gradients from the current album artwork. This behaviour is enabled by default. Set the top-level `use_gradient = false` flag (outside any `[table]`) to keep the static colors defined in `[components.*]` instead.

### Dynamic Gradients in Detail

When `use_gradient` is left at its default `true`, the widget analyses each incoming album thumbnail and computes a pair of dominant colours using a lightweight K-means clustering pass across the image pixels. The resulting colours are ordered from darker to lighter to preserve contrast, then injected into gradient specs for both the root (window) background and the inner panel.

The gradient direction for each area respects the static fallback you define in `theme.toml`. That means if you specify a vertical gradient in `[components.panel.background]`, the dynamic gradient will also flow vertically. If your static background is a solid colour, the dynamic system defaults to a vertical blend.

#### Control Flow

1. Album artwork is downloaded by the app and decoded to RGBA pixels.
2. Up to ~6,000 opaque pixels are sampled to keep the clustering fast.
3. K-means (k = 3) identifies the most common hues. The two most distinct clusters become the gradient stops.
4. If fewer than two meaningful colours are found, or `use_gradient = false`, the widget falls back to the static colours defined in `components.root` / `components.panel`.
5. Gradients update automatically whenever the artwork changes.

#### Customising Per Area

- **Root area** (`components.root.background`): direction is taken from your theme definition; colour stops are overridden dynamically when gradients are enabled.
- **Panel area** (`components.panel.background`): follows the same rules as the root area.
- Other components (`button`, `slider`, etc.) do not receive dynamic colours, continue to define explicit palette entries for those.

#### Opting Out

- Set `use_gradient = false` once at the top of `theme.toml` to disable dynamic gradients globally for the skin.
- This preserves the exact start/end colours you declare inside each `[components.*.background]` block.

#### Tips for Best Results

- Provide a reasonable static gradient (or solid colour) as a fallback in case artwork is monochrome or fails to load.
- Choose panel foreground colours with sufficient contrast against both your static colours and the kinds of artwork your skin targets.
- Gradients respect the corner radius defined for the area, so rounded panels will retain smooth edges.

### Vinyl Thumbnail Renderer

The vinyl-style thumbnail renderer is **interactive by default** for skins that permit it. It transforms album artwork into a spinning vinyl disc with:

- A polar-coordinate swirl effect applied to the outer ring
- A preserved center label showing the original artwork
- Real-time rotation when playback is active (respects system reduced-motion preferences)
- Configurable parameters in `config.toml` under `[ui.vinyl_thumbnail]`:
	- `enabled` (default `true`): master toggle for the vinyl effect
	- `swirl_strength` (default `2.5`): maximum angular distortion at the outer edge (in radians)
	- `label_ratio` (default `0.35`): radius of the untouched center label as a fraction of the disc

The vinyl renderer outputs a square texture and continues to honour any `overlay_images` declared in `components.thumbnail`.

#### Opting Out of Vinyl

Skins that prefer traditional square album artwork can disable the vinyl effect by adding:

```toml
[meta]
disable_vinyl_thumbnail = true
```

When a skin with `disable_vinyl_thumbnail = true` is selected, the app automatically switches to standard thumbnail rendering and hides the in-app toggle. The bundled **Graphite**, **Cutesy**, and **Gradient Demo** skins opt out this way.

> **Runtime toggle:** When vinyl is allowed, end users can switch between the disc and the original artwork from the settings drawer or by clicking the artwork directly.

> **Tip:** The **Aurora Vinyl** skin is designed specifically to showcase the vinyl renderer with a circular mask and glowing accent ring.

### Components

The `[components.*]` tables specify appearance for individual UI areas. All numeric values accept `{vars.*}` placeholders and fall back to sensible defaults when omitted.

| Component | Purpose |
|-----------|---------|
| `components.root` | Window background and border styling. |
| `components.panel` | Inner panel behind text and controls. |
| `components.button` | Transport buttons (backgrounds, borders). |
| `components.button.icon` | Glyph color and scaling for transport icons. |
| `components.slider` | Timeline track and thumb appearance. |
| `components.thumbnail` | Album artwork framing (corners + optional border image). |
| `components.text.title` | Primary typography (track title). |
| `components.text.body` | Secondary typography (artist, album, auxiliary labels). |

#### Area Components (`root` and `panel`)

```toml
[components.panel]
background = "{colors.panel}"
foreground = "{colors.text_primary}"
border_color = "{colors.outline}"
border_radius = "{vars.panel_radius}"
border_width = "1.5"
show_border = true
```

- `background`: Accepts either a direct color string or a table describing a gradient. For example:

	```toml
	[components.panel.background]
	kind = "gradient"          # optional when `start`/`end` are present
	start = "{colors.panel}"
	end = "{colors.shadow}"
	direction = "vertical"     # "vertical" (default) or "horizontal"
	```
- `foreground`: Default text/icon color rendered above the area.
- `border_color` / `border_width`: Outline styling (set width to `0` for no border).
- `show_border`: Optional boolean toggle (defaults to `true`). Set to `false` to hide the outline even if a width/color are provided.
- `border_radius`: Corner radius in logical pixels.

#### Button Styling

```toml
[components.button]
background = "{colors.accent}"
foreground = "{colors.text_on_accent}"
hover_background = "{colors.accent_hover}"
active_background = "{colors.accent_active}"
border_color = "transparent"
border_radius = "26"
border_width = "1"

[components.button.icon]
color = "{colors.text_on_accent}"
size_scale = "3.2"
```

- Use contrasting `hover_background`/`active_background` to provide tactile feedback.
- `size_scale` enlarges or shrinks the glyph relative to the button padding (default `1.0`).

#### Slider Configuration

```toml
[components.slider]
track_fill = "{colors.accent}"
track_background = "{colors.slider_track_bg}"
track_thickness = "4"
thumb_shape = "image"        # `circle` or `image`
thumb_color = "{colors.text_on_accent}"
thumb_radius = "10"          # Used when `thumb_shape = "circle"`
thumb_size = "32"            # Used when `thumb_shape = "image"`
thumb_image = "thumb.png"    # Relative to `assets/`
```

- `thumb_shape`: choose between a simple circle or a custom PNG.
- When using `thumb_image`, place the asset in the skin’s `assets/` directory. The widget will emit warnings if the file is missing.

#### Thumbnail Styling

border_image = "thumbnail-border.png"  # Optional overlay PNG
```toml
[components.thumbnail]
corner_radius = "{vars.radius}"
stroke_color = "{colors.outline}"
stroke_width = "6"
overlay_images = ["thumbnail-border.png", "sparkles.png"]
```

- `corner_radius`: Applied to album artwork and the fallback placeholder.
- `stroke_color` / `stroke_width`: Configure a programmatically rendered rounded stroke that frames the artwork. Set width to `0` (default) to disable.
- `overlay_images`: Optional list of PNG/JPEG overlays drawn in order above the artwork. Each entry may be a bare string (`"sparkles.png"`) or an inline table with offsets (`{ path = "sparkles.png", offset_x = "12", offset_y = "-8" }`). Paths are resolved relative to the skin’s `assets/` directory, overlays are clipped to the same rounded corners as the underlying thumbnail, and large images automatically scale down (maintaining aspect ratio) so they fit inside the frame. Offsets are integer (or numeric) amounts in logical pixels applied after scaling, letting you nudge individual overlays horizontally or vertically.

		```toml
		[components.thumbnail]
		corner_radius = "{vars.radius}"
		overlay_images = [
			{ path = "sparkles.png", offset_x = "12", offset_y = "-8" },
			"thumbnail-border.png"
		]
		```
- `border_image`: Legacy single-overlay field. Still supported for compatibility; its value is appended to `overlay_images` if both are present.

#### Typography

```toml
[components.text.title]
color = "{colors.text_primary}"
size = "24"

[components.text.body]
color = "{colors.text_secondary}"
size = "16"
```

Font sizes map directly to egui point sizes. The application currently ships with Lato Regular/Bold embedded; custom skins can still set sizes and colors, but font family changes require code changes.

### Token Resolution & Warnings

The theme loader resolves tokens recursively (up to five passes). When a token or file cannot be resolved, the widget collects a warning and falls back to defaults. Warnings are shown via the `Skin Warnings` component at runtime.

## Asset Handling

- **Slider thumbs & borders**: Stored under `skins/<skin-id>/assets/`. Images are loaded lazily and cached per skin.
- **Fonts**: Shared fonts live under `app/assets/fonts/`. If you need a skin-specific font, include it under the skin’s assets and extend the code to load it (see `SkinManager::apply_style`).
- **Artwork overrides**: The application only displays album art from the active media session; skins cannot override the actual image, but can frame it using `components.thumbnail`.

## Creating a New Skin

1. Duplicate an existing skin directory (e.g., `graphite` or `cutesy`).
2. Update `theme.toml` colors, variables, and component settings following the sections above.
3. Adjust `layout.toml` or create new layout variants for different window sizes.
4. Place any custom images (slider thumbs, border frames) under the `assets/` folder and reference them from `theme.toml`.
5. Launch the widget with hot reload enabled to iterate quickly.

## Validation Checklist

- Run `cargo run` with the widget visible and confirm there are no warnings in the Skin Warnings panel.
- Resize the window to test how your layout responds to compact and wide modes.
- Verify that required assets exist. Missing files trigger a warning with the expected path.

For component arrangement and layout parameters, continue to [layout.md](layout.md).
