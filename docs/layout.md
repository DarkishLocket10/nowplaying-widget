# Layout Engine Reference

Skins can ship a dedicated `layout.toml` that controls what UI elements are shown and how they are arranged. Layouts hot-reload alongside themes and fall back to a built-in default when omitted. Read this guide alongside the [Theme & Asset Reference](theme.md) when building skins end to end.

## Table of Contents

- [File Structure](#file-structure)
- [Node Types](#node-types)
- [Component IDs](#component-ids)
- [Component Parameters](#component-parameters)
- [Hot Reload & Fallbacks](#hot-reload--fallbacks)
- [Example](#example)

## File Structure

```toml
[meta]
engine = "1"           # layout engine version

[layout]
default = "variant_id" # id of the variant to select initially

[[layout.variants]]
id = "variant_id"
display_name = "Human friendly name"

[layout.variants.structure]
# node definition (see below)
```

Each variant describes a tree of layout nodes. Variants can be switched at runtime from the skin controls panel.

## Node Types

Layouts are built from four node types, declared via `type`:

| Type      | Purpose                                                     | Fields |
|-----------|-------------------------------------------------------------|--------|
| `row`     | Arrange children horizontally.                              | `align` (`start`/`center`/`end`), `spacing` (default `8`), `fill` (bool), `visible` (bool), `children` |
| `column`  | Arrange children vertically.                                | Same fields as `row` |
| `component` | Render a specific UI element.                             | `id` (component identifier), `visible` (bool), `params` (string map) |
| `spacer`  | Insert empty space.                                         | `size` (float, default `8`) |

`fill = true` forces the node to claim the available width before laying out children. `align` controls the cross-axis alignment (`start`, `center`, `end`). Any row/column with all children hidden is discarded automatically.

## Component IDs

`component` nodes accept the following identifiers:

| ID | Effect |
|----|--------|
| `thumbnail` | Album artwork image placeholder (respects `components.thumbnail` styling). |
| `title` | Track title text. |
| `metadata` | Artist, album, and playback state block. |
| `metadata.artist` | Artist line only. |
| `metadata.album` | Album line only. |
| `metadata.state` | Playback state line only. |
| `playback_controls` | Standard previous/play/pause/next row (stop button retired but ID retained for legacy layouts). |
| `button.previous` | Individual Previous button. |
| `button.play` / `button.playpause` / `button.pause` | Play/Pause toggle. |
| `button.next` | Individual Next button. |
| `button.stop` | Legacy stop button (no-op). |
| `timeline` | Seek slider plus timestamps. |
| `skin_warnings` | Render accumulated skin/layout warnings. |
| `skin_error` | Render skin loader errors. |
| `thumbnail_error` | Render artwork loading errors. |
| `error` | Render live playback errors. |

Custom control over visibility is available via `visible = false` on any component node.

## Component Parameters

Optional behavior tweaks are provided via `params` tables:

```toml
[[layout.variants.structure.children]]
type = "component"
id = "playback_controls"
  [layout.variants.structure.children.params]
  centered = "true"
```

Currently supported flags:

| Component | Parameter | Description |
|-----------|-----------|-------------|
| `playback_controls` | `centered` | When `true`, centers the button row within the available width. |
| `timeline` | `centered` | Centers the slider and timestamp readouts. |
| `timeline` | `separator` | Set to `false` to suppress the leading separator line. |
| `metadata` | `show_state` | Set to `false` to omit the playback state line when rendering the full metadata block. |
| `metadata` | `show_state_label` | Controls the `State:` prefix; set to `false` to display only the status text. |
| `metadata.state` | `show_state_label` | Controls the `State:` prefix when using the dedicated state component. |

`show_state` accepts the alias `state`, and `show_state_label` also accepts the shorter alias `state_label` for convenience.

Values are parsed case-insensitively; `true/false`, `yes/no`, `1/0`, `on/off` are recognised.

## Hot Reload & Fallbacks

* Missing or invalid `layout.toml` files trigger a warning and fall back to the embedded default layout.
* Layout files participate in hot reload – saving a `.toml` change under a skin directory refreshes both theme and layout automatically.
* When switching skins, the previous layout selection is preserved if the new skin offers a variant with the same id; otherwise the skin’s `layout.default` (or the first listed variant) is used.

## Example

The `skins/cutesy/layout.toml` skin demonstrates:

* Multiple variants (`cutesy_mobile`, `cutesy_left`, `cutesy_right`, `cutesy_minimal`).
* Centered controls and timelines via `params.centered`.
* Selective omission of metadata and timeline elements in the “Minimal Controls” variant.
* Dedicated warning and error components placed at the bottom of each layout.

Use it as a starting point for bespoke arrangements tailored to your skin aesthetics.