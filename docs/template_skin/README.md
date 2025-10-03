# Template Skin Reference

This directory provides fully annotated `theme.template.toml` and `layout.template.toml` files that demonstrate every supported option for building a Now Playing Widget skin.

- **theme.template.toml** – Shows every field available in `theme.toml`, with inline comments that describe what each setting controls and example values you can adapt.
- **layout.template.toml** – Covers every layout node type, per-node parameters, and component options, each accompanied by comments and example usage.

## How to Use

1. Copy the template files into a new skin folder under `skins/<your-skin>/`.
2. Rename them to `theme.toml` and `layout.toml`.
3. Replace the example values with your palette, typography, and layout choices.
4. Remove any sections you do not need—the comments indicate which lines are optional.
5. Place any referenced assets (slider thumbs, thumbnail overlays, etc.) in the skin's `assets/` directory.

For deeper background, consult the main documentation in [`docs/theme.md`](../theme.md) and [`docs/layout.md`](../layout.md).
