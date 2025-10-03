use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub const THEME_ENGINE_VERSION: &str = "1";

#[derive(Debug, Clone)]
pub struct LoadedTheme {
    pub theme: Theme,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Theme {
    pub name: String,
    pub display_name: String,
    pub engine_version: String,
    pub asset_root: PathBuf,
    pub colors: HashMap<String, Color32>,
    pub vars: HashMap<String, f32>,
    pub use_gradient: bool,
    pub disable_vinyl_thumbnail: bool,
    pub transparent_background: bool,
    pub components: Components,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Components {
    pub root: AreaStyle,
    pub panel: AreaStyle,
    pub button: ButtonStyle,
    pub button_icon: IconStyle,
    pub slider: SliderStyle,
    pub thumbnail: ThumbnailStyle,
    pub text_title: TextStyle,
    pub text_body: TextStyle,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AreaStyle {
    pub background: AreaBackground,
    pub foreground: Color32,
    pub border_color: Color32,
    pub border_radius: f32,
    pub border_width: f32,
    pub show_border: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AreaBackground {
    Solid(Color32),
    Gradient(GradientSpec),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GradientSpec {
    pub start: Color32,
    pub end: Color32,
    pub direction: GradientDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GradientDirection {
    Vertical,
    Horizontal,
}

impl AreaBackground {
    pub fn primary_color(&self) -> Color32 {
        match self {
            AreaBackground::Solid(color) => *color,
            AreaBackground::Gradient(gradient) => gradient.start,
        }
    }
}

impl AreaStyle {
    pub fn background_color(&self) -> Color32 {
        self.background.primary_color()
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ButtonStyle {
    pub background: Color32,
    pub foreground: Color32,
    pub hover_background: Color32,
    pub active_background: Color32,
    pub border_color: Color32,
    pub border_radius: f32,
    pub border_width: f32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IconStyle {
    pub color: Color32,
    pub size_scale: f32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SliderStyle {
    pub track_fill: Color32,
    pub track_background: Color32,
    pub track_thickness: f32,
    pub thumb: SliderThumb,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SliderThumb {
    Circle {
        color: Color32,
        radius: f32,
    },
    Image {
        color: Color32,
        path: PathBuf,
        size: egui::Vec2,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ThumbnailStyle {
    pub corner_radius: f32,
    pub stroke_color: Color32,
    pub stroke_width: f32,
    pub overlays: Vec<ThumbnailOverlay>,
}

#[derive(Debug, Clone)]
pub struct ThumbnailOverlay {
    pub path: PathBuf,
    pub offset: egui::Vec2,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TextStyle {
    pub color: Color32,
    pub size: f32,
}

pub fn load_theme_from_dir(skin_dir: &Path) -> Result<LoadedTheme> {
    let mut warnings = Vec::new();
    let mut base = builtin_theme_document();

    let theme_path = skin_dir.join("theme.toml");
    if theme_path.exists() {
        let data = fs::read_to_string(&theme_path)
            .with_context(|| format!("Failed to read theme file: {}", theme_path.display()))?;
        match toml::from_str::<ThemeDocument>(&data) {
            Ok(doc) => {
                if let Some(engine) = doc.meta.engine.as_deref() {
                    if engine != THEME_ENGINE_VERSION {
                        warnings.push(format!(
                            "Skin engine version {engine} does not match {THEME_ENGINE_VERSION}; using defaults"
                        ));
                    } else {
                        merge_documents(&mut base, doc);
                    }
                } else {
                    warnings.push("meta.engine missing; assuming version 1".to_string());
                    merge_documents(&mut base, doc);
                }
            }
            Err(err) => {
                warnings.push(format!("Failed to parse theme: {err}"));
            }
        }
    } else {
        warnings.push(format!(
            "Skin folder {} missing theme.toml; falling back to defaults",
            skin_dir.display()
        ));
    }

    let theme = resolve_document(base, skin_dir, &mut warnings)?;
    Ok(LoadedTheme { theme, warnings })
}

fn resolve_document(
    doc: ThemeDocument,
    skin_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<Theme> {
    let mut context = ValueContext::new(&doc.colors, &doc.vars);
    for _ in 0..5 {
        let mut changed = false;
        for (k, v) in doc.colors.iter() {
            let resolved = resolve_tokens_with_opts(v, &context, warnings, false);
            if context
                .colors
                .get(k)
                .map(|current| current != &resolved)
                .unwrap_or(true)
            {
                context.colors.insert(k.clone(), resolved);
                changed = true;
            }
        }
        for (k, v) in doc.vars.iter() {
            let resolved = resolve_tokens_with_opts(v, &context, warnings, false);
            if context
                .vars
                .get(k)
                .map(|current| current != &resolved)
                .unwrap_or(true)
            {
                context.vars.insert(k.clone(), resolved);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let colors = context
        .colors
        .iter()
        .map(|(k, v)| {
            let color = match parse_color(v) {
                Ok(c) => c,
                Err(err) => {
                    warnings.push(format!("{k}: {err}; using fallback #FFFFFF"));
                    Color32::WHITE
                }
            };
            (k.clone(), color)
        })
        .collect::<HashMap<_, _>>();

    let vars = context
        .vars
        .iter()
        .filter_map(|(k, v)| match parse_number(v) {
            Some(num) => Some((k.clone(), num)),
            None => {
                warnings.push(format!("Variable {k} could not be parsed as number: {v}"));
                None
            }
        })
        .collect::<HashMap<_, _>>();

    let get_color =
        |key: &str, fallback: Color32| -> Color32 { colors.get(key).copied().unwrap_or(fallback) };

    let radius_default = *vars.get("radius").unwrap_or(&8.0);
    let thumb_radius_default = *vars.get("slider_thumb_radius").unwrap_or(&8.0);

    let root = resolve_area(
        &doc.components.root,
        &context,
        &colors,
        radius_default,
        warnings,
    )
    .unwrap_or_else(|_| AreaStyle {
        background: AreaBackground::Solid(Color32::from_rgb(18, 18, 18)),
        foreground: Color32::WHITE,
        border_color: Color32::TRANSPARENT,
        border_radius: radius_default,
        border_width: 0.0,
        show_border: false,
    });

    let panel = resolve_area(
        &doc.components.panel,
        &context,
        &colors,
        radius_default,
        warnings,
    )
    .unwrap_or_else(|_| AreaStyle {
        background: AreaBackground::Solid(get_color("panel", Color32::from_rgb(32, 32, 32))),
        foreground: get_color("text_primary", Color32::WHITE),
        border_color: Color32::TRANSPARENT,
        border_radius: radius_default,
        border_width: 0.0,
        show_border: false,
    });

    let button = resolve_button(
        &doc.components.button,
        &context,
        &colors,
        radius_default,
        warnings,
    )
    .unwrap_or_else(|_| ButtonStyle {
        background: get_color("accent", Color32::from_rgb(0, 120, 212)),
        foreground: get_color("text_on_accent", Color32::WHITE),
        hover_background: get_color("accent_hover", Color32::from_rgb(15, 108, 189)),
        active_background: get_color("accent_active", Color32::from_rgb(17, 94, 163)),
        border_color: Color32::TRANSPARENT,
        border_radius: radius_default,
        border_width: 0.0,
    });

    let button_icon = resolve_icon(&doc.components.button.icon, &context, &colors, warnings)
        .unwrap_or_else(|_| IconStyle {
            color: get_color("text_on_accent", Color32::WHITE),
            size_scale: 1.0,
        });

    let slider = resolve_slider(
        &doc.components.slider,
        &context,
        &colors,
        thumb_radius_default,
        skin_dir,
        warnings,
    )
    .unwrap_or_else(|_| SliderStyle {
        track_fill: get_color("accent", Color32::from_rgb(0, 120, 212)),
        track_background: get_color("slider_track_bg", Color32::from_rgb(64, 64, 64)),
        track_thickness: 4.0,
        thumb: SliderThumb::Circle {
            color: get_color("accent", Color32::from_rgb(0, 120, 212)),
            radius: thumb_radius_default,
        },
    });

    let thumbnail = resolve_thumbnail(
        &doc.components.thumbnail,
        &context,
        &colors,
        radius_default,
        skin_dir,
        warnings,
    )
    .unwrap_or_else(|_| ThumbnailStyle {
        corner_radius: radius_default,
        stroke_color: Color32::TRANSPARENT,
        stroke_width: 0.0,
        overlays: Vec::new(),
    });

    let text_title = resolve_text(
        &doc.components.text.title,
        &context,
        &colors,
        20.0,
        warnings,
    )
    .unwrap_or_else(|_| TextStyle {
        color: get_color("text_primary", Color32::WHITE),
        size: 20.0,
    });

    let text_body = resolve_text(&doc.components.text.body, &context, &colors, 16.0, warnings)
        .unwrap_or_else(|_| TextStyle {
            color: get_color("text_secondary", Color32::from_rgb(200, 200, 200)),
            size: 16.0,
        });

    let name = doc.meta.name.clone().unwrap_or_else(|| {
        skin_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    let display_name = doc
        .meta
        .display_name
        .clone()
        .unwrap_or_else(|| name.clone());
    let use_gradient = doc.use_gradient.unwrap_or(true);
    let disable_vinyl = doc.meta.disable_vinyl_thumbnail.unwrap_or(false);
    let transparent_bg = doc.transparent_background.or(doc.meta.transparent_background).unwrap_or(false);

    Ok(Theme {
        name,
        display_name,
        engine_version: doc
            .meta
            .engine
            .clone()
            .unwrap_or_else(|| THEME_ENGINE_VERSION.to_string()),
        asset_root: skin_dir.join("assets"),
        colors,
        vars,
        use_gradient,
        disable_vinyl_thumbnail: disable_vinyl,
        transparent_background: transparent_bg,
        components: Components {
            root,
            panel,
            button,
            button_icon,
            slider,
            thumbnail,
            text_title,
            text_body,
        },
    })
}

fn resolve_area(
    cfg: &AreaConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    radius_default: f32,
    warnings: &mut Vec<String>,
) -> Result<AreaStyle> {
    let background = resolve_area_background(&cfg.background, ctx, colors, warnings)
        .unwrap_or_else(|| AreaBackground::Solid(Color32::from_rgb(32, 32, 32)));
    let foreground =
        resolve_color_field(&cfg.foreground, ctx, colors, warnings).unwrap_or(Color32::WHITE);
    let border_color = resolve_color_field(&cfg.border_color, ctx, colors, warnings)
        .unwrap_or(Color32::TRANSPARENT);
    let border_radius =
        resolve_number_field(&cfg.border_radius, ctx, warnings).unwrap_or(radius_default);
    let border_width = resolve_number_field(&cfg.border_width, ctx, warnings).unwrap_or(0.0);
    let show_border = cfg
        .show_border
        .unwrap_or(border_width > f32::EPSILON && border_color != Color32::TRANSPARENT);
    Ok(AreaStyle {
        background,
        foreground,
        border_color,
        border_radius,
        border_width,
        show_border,
    })
}

fn resolve_area_background(
    value: &Option<BackgroundFieldConfig>,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Option<AreaBackground> {
    match value.as_ref()? {
        BackgroundFieldConfig::Simple(simple) => Some(AreaBackground::Solid(resolve_color_string(
            simple, ctx, colors, warnings,
        ))),
        BackgroundFieldConfig::Table(table) => {
            resolve_background_table(table, ctx, colors, warnings)
        }
    }
}

fn resolve_background_table(
    table: &BackgroundTableConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Option<AreaBackground> {
    let kind = table.kind.as_ref().map(|k| k.trim().to_ascii_lowercase());

    match kind.as_deref() {
        Some("solid") => resolve_solid_background(table, ctx, colors, warnings),
        Some("gradient") => resolve_gradient_background(table, ctx, colors, warnings),
        Some(other) => {
            warnings.push(format!("Unknown background type '{other}'"));
            None
        }
        None => {
            if table.start.is_some() || table.end.is_some() {
                resolve_gradient_background(table, ctx, colors, warnings)
            } else if table.color.is_some() {
                resolve_solid_background(table, ctx, colors, warnings)
            } else {
                warnings
                    .push("Background table requires either 'color' or 'start'/'end'".to_string());
                None
            }
        }
    }
}

fn resolve_solid_background(
    table: &BackgroundTableConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Option<AreaBackground> {
    match table.color.as_ref() {
        Some(color_value) => Some(AreaBackground::Solid(resolve_color_string(
            color_value,
            ctx,
            colors,
            warnings,
        ))),
        None => {
            warnings.push("Solid background requires 'color' value".to_string());
            None
        }
    }
}

fn resolve_gradient_background(
    table: &BackgroundTableConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Option<AreaBackground> {
    let Some(start_value) = table.start.as_ref() else {
        warnings.push("Gradient background missing 'start' color".to_string());
        return None;
    };
    let Some(end_value) = table.end.as_ref() else {
        warnings.push("Gradient background missing 'end' color".to_string());
        return None;
    };

    let start_color = resolve_color_string(start_value, ctx, colors, warnings);
    let end_color = resolve_color_string(end_value, ctx, colors, warnings);
    let direction = match table.direction {
        GradientDirectionConfig::Horizontal => GradientDirection::Horizontal,
        GradientDirectionConfig::Vertical => GradientDirection::Vertical,
    };

    if start_color == end_color {
        return Some(AreaBackground::Solid(start_color));
    }

    Some(AreaBackground::Gradient(GradientSpec {
        start: start_color,
        end: end_color,
        direction,
    }))
}

fn resolve_button(
    cfg: &ButtonConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    radius_default: f32,
    warnings: &mut Vec<String>,
) -> Result<ButtonStyle> {
    Ok(ButtonStyle {
        background: resolve_color_field(&cfg.background, ctx, colors, warnings)
            .unwrap_or(Color32::from_rgb(0, 120, 212)),
        foreground: resolve_color_field(&cfg.foreground, ctx, colors, warnings)
            .unwrap_or(Color32::WHITE),
        hover_background: resolve_color_field(&cfg.hover_background, ctx, colors, warnings)
            .unwrap_or(Color32::from_rgb(15, 108, 189)),
        active_background: resolve_color_field(&cfg.active_background, ctx, colors, warnings)
            .unwrap_or(Color32::from_rgb(17, 94, 163)),
        border_color: resolve_color_field(&cfg.border_color, ctx, colors, warnings)
            .unwrap_or(Color32::TRANSPARENT),
        border_radius: resolve_number_field(&cfg.border_radius, ctx, warnings)
            .unwrap_or(radius_default),
        border_width: resolve_number_field(&cfg.border_width, ctx, warnings).unwrap_or(0.0),
    })
}

fn resolve_icon(
    cfg: &IconConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Result<IconStyle> {
    Ok(IconStyle {
        color: resolve_color_field(&cfg.color, ctx, colors, warnings).unwrap_or(Color32::WHITE),
        size_scale: resolve_number_field(&cfg.size_scale, ctx, warnings).unwrap_or(1.0),
    })
}

fn resolve_slider(
    cfg: &SliderConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    thumb_radius_default: f32,
    skin_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<SliderStyle> {
    let track_fill = resolve_color_field(&cfg.track_fill, ctx, colors, warnings)
        .unwrap_or(Color32::from_rgb(0, 120, 212));
    let track_background = resolve_color_field(&cfg.track_background, ctx, colors, warnings)
        .unwrap_or(Color32::from_rgb(64, 64, 64));
    let track_thickness = resolve_number_field(&cfg.track_thickness, ctx, warnings).unwrap_or(4.0);

    let thumb_shape = cfg
        .thumb_shape
        .as_ref()
        .map(|s| resolve_tokens(s, ctx, warnings))
        .unwrap_or_else(|| "circle".to_string())
        .to_lowercase();

    let thumb_color =
        resolve_color_field(&cfg.thumb_color, ctx, colors, warnings).unwrap_or(track_fill);

    let thumb = if thumb_shape == "image" {
        let image_name = cfg
            .thumb_image
            .as_ref()
            .map(|s| resolve_tokens(s, ctx, warnings))
            .unwrap_or_default();
        if image_name.is_empty() {
            warnings.push("Slider thumb image requested but no image provided".to_string());
            SliderThumb::Circle {
                color: thumb_color,
                radius: thumb_radius_default,
            }
        } else {
            let assets_dir = skin_dir.join("assets");
            let mut path = assets_dir.join(&image_name);
            if !path.exists() {
                warnings.push(format!(
                    "Slider thumb image {} not found; reverting to circle thumb",
                    path.display()
                ));
                SliderThumb::Circle {
                    color: thumb_color,
                    radius: thumb_radius_default,
                }
            } else {
                path = canonicalize_asset_path(path);
                let size = resolve_number_field(&cfg.thumb_size, ctx, warnings).unwrap_or(24.0);
                SliderThumb::Image {
                    color: thumb_color,
                    path,
                    size: egui::vec2(size, size),
                }
            }
        }
    } else {
        let radius =
            resolve_number_field(&cfg.thumb_radius, ctx, warnings).unwrap_or(thumb_radius_default);
        SliderThumb::Circle {
            color: thumb_color,
            radius,
        }
    };

    Ok(SliderStyle {
        track_fill,
        track_background,
        track_thickness,
        thumb,
    })
}

fn resolve_thumbnail(
    cfg: &ThumbnailConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    radius_default: f32,
    skin_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<ThumbnailStyle> {
    let corner_radius =
        resolve_number_field(&cfg.corner_radius, ctx, warnings).unwrap_or(radius_default);

    let stroke_color = resolve_color_field(&cfg.stroke_color, ctx, colors, warnings)
        .unwrap_or(Color32::TRANSPARENT);
    let stroke_width = resolve_number_field(&cfg.stroke_width, ctx, warnings)
        .unwrap_or(0.0)
        .max(0.0);

    let mut overlays = Vec::new();

    if let Some(images) = cfg.overlay_images.as_ref() {
        for entry in images {
            if let Some(overlay) = build_thumbnail_overlay(entry, skin_dir, ctx, warnings) {
                overlays.push(overlay);
            }
        }
    }

    if let Some(single) = cfg.border_image.as_ref() {
        if let Some(border_overlay) =
            build_overlay_from_components(single, egui::Vec2::ZERO, skin_dir, ctx, warnings)
        {
            overlays.push(border_overlay);
        }
    }

    Ok(ThumbnailStyle {
        corner_radius,
        stroke_color,
        stroke_width,
        overlays,
    })
}

fn canonicalize_asset_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn build_thumbnail_overlay(
    entry: &OverlayImageEntry,
    skin_dir: &Path,
    ctx: &ValueContext,
    warnings: &mut Vec<String>,
) -> Option<ThumbnailOverlay> {
    match entry {
        OverlayImageEntry::Path(raw) => {
            build_overlay_from_components(raw, egui::Vec2::ZERO, skin_dir, ctx, warnings)
        }
        OverlayImageEntry::Detailed {
            path,
            offset_x,
            offset_y,
        } => {
            let offset = egui::vec2(
                resolve_overlay_offset(offset_x, "offset_x", ctx, warnings),
                resolve_overlay_offset(offset_y, "offset_y", ctx, warnings),
            );
            build_overlay_from_components(path, offset, skin_dir, ctx, warnings)
        }
    }
}

fn build_overlay_from_components(
    raw_path: &str,
    offset: egui::Vec2,
    skin_dir: &Path,
    ctx: &ValueContext,
    warnings: &mut Vec<String>,
) -> Option<ThumbnailOverlay> {
    let resolved = resolve_tokens(raw_path, ctx, warnings);
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut path = skin_dir.join("assets").join(trimmed);
    if path.exists() {
        path = canonicalize_asset_path(path);
        Some(ThumbnailOverlay { path, offset })
    } else {
        warnings.push(format!(
            "Thumbnail overlay image {} not found; skipping",
            path.display()
        ));
        None
    }
}

fn resolve_overlay_offset(
    value: &Option<String>,
    axis: &str,
    ctx: &ValueContext,
    warnings: &mut Vec<String>,
) -> f32 {
    value
        .as_ref()
        .and_then(|raw| {
            let resolved = resolve_tokens(raw, ctx, warnings);
            parse_number(&resolved).or_else(|| {
                warnings.push(format!(
                    "Could not parse thumbnail overlay {axis}: {resolved}; using 0"
                ));
                None
            })
        })
        .unwrap_or(0.0)
}

fn resolve_text(
    cfg: &TextConfig,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    default_size: f32,
    warnings: &mut Vec<String>,
) -> Result<TextStyle> {
    Ok(TextStyle {
        color: resolve_color_field(&cfg.color, ctx, colors, warnings).unwrap_or(Color32::WHITE),
        size: resolve_number_field(&cfg.size, ctx, warnings).unwrap_or(default_size),
    })
}

fn resolve_color_field(
    value: &Option<String>,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Option<Color32> {
    value
        .as_ref()
        .map(|v| resolve_color_string(v, ctx, colors, warnings))
}

fn resolve_color_string(
    value: &str,
    ctx: &ValueContext,
    colors: &HashMap<String, Color32>,
    warnings: &mut Vec<String>,
) -> Color32 {
    let resolved = resolve_tokens(value, ctx, warnings);
    if let Some(existing) = colors.get(&resolved) {
        *existing
    } else {
        match parse_color(&resolved) {
            Ok(color) => color,
            Err(err) => {
                warnings.push(format!("{resolved}: {err}; using transparent"));
                Color32::TRANSPARENT
            }
        }
    }
}

fn resolve_number_field(
    value: &Option<String>,
    ctx: &ValueContext,
    warnings: &mut Vec<String>,
) -> Option<f32> {
    value.as_ref().and_then(|v| {
        let resolved = resolve_tokens(v, ctx, warnings);
        parse_number(&resolved).or_else(|| {
            warnings.push(format!("Could not parse number value: {resolved}"));
            None
        })
    })
}

fn parse_color(value: &str) -> Result<Color32> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("transparent") {
        return Ok(Color32::TRANSPARENT);
    }
    if let Some(hex) = v.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(rest) = v.strip_prefix("rgba(") {
        return parse_rgba(rest.trim_end_matches(')'));
    }
    if let Some(rest) = v.strip_prefix("rgb(") {
        let (r, g, b, a) = parse_rgb_components(rest.trim_end_matches(')'))?;
        return Ok(Color32::from_rgba_premultiplied(r, g, b, a));
    }
    Err(anyhow!("Unsupported color format: {v}"))
}

fn parse_hex_color(hex: &str) -> Result<Color32> {
    let value = hex.trim();
    let bytes = match value.len() {
        6 => u32::from_str_radix(value, 16).ok(),
        8 => u32::from_str_radix(value, 16).ok(),
        _ => None,
    }
    .ok_or_else(|| anyhow!("Invalid hex color: #{value}"))?;

    Ok(match value.len() {
        6 => {
            let r = ((bytes >> 16) & 0xFF) as u8;
            let g = ((bytes >> 8) & 0xFF) as u8;
            let b = (bytes & 0xFF) as u8;
            Color32::from_rgb(r, g, b)
        }
        8 => {
            let a = (bytes & 0xFF) as u8;
            let b = ((bytes >> 8) & 0xFF) as u8;
            let g = ((bytes >> 16) & 0xFF) as u8;
            let r = ((bytes >> 24) & 0xFF) as u8;
            Color32::from_rgba_premultiplied(r, g, b, a)
        }
        _ => unreachable!(),
    })
}

fn parse_rgba(input: &str) -> Result<Color32> {
    let (r, g, b, a) = parse_rgba_components(input)?;
    Ok(Color32::from_rgba_premultiplied(r, g, b, a))
}

fn parse_rgba_components(input: &str) -> Result<(u8, u8, u8, u8)> {
    let parts: Vec<_> = input.split(',').map(|p| p.trim()).collect();
    if parts.len() != 4 {
        return Err(anyhow!("rgba expects 4 components"));
    }
    let (r, g, b, _) = parse_rgb_components(&parts[0..3].join(","))?;
    let a = parse_alpha(parts[3])?;
    Ok((r, g, b, a))
}

fn parse_rgb_components(input: &str) -> Result<(u8, u8, u8, u8)> {
    let parts: Vec<_> = input.split(',').map(|p| p.trim()).collect();
    if parts.len() != 3 {
        return Err(anyhow!("rgb expects 3 components"));
    }
    let r = parse_component(parts[0])?;
    let g = parse_component(parts[1])?;
    let b = parse_component(parts[2])?;
    Ok((r, g, b, 255))
}

fn parse_component(src: &str) -> Result<u8> {
    let value: f32 = src
        .parse()
        .map_err(|_| anyhow!("Invalid color channel: {src}"))?;
    if !(0.0..=255.0).contains(&value) {
        return Err(anyhow!("Color channel out of range: {src}"));
    }
    Ok(value.round() as u8)
}

fn parse_alpha(src: &str) -> Result<u8> {
    if src.contains('.') {
        let value: f32 = src.parse().map_err(|_| anyhow!("Invalid alpha: {src}"))?;
        if !(0.0..=1.0).contains(&value) {
            return Err(anyhow!("Alpha out of range: {src}"));
        }
        Ok((value * 255.0).round() as u8)
    } else {
        parse_component(src)
    }
}

fn parse_number(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok()
}

fn resolve_tokens(value: &str, ctx: &ValueContext, warnings: &mut Vec<String>) -> String {
    resolve_tokens_with_opts(value, ctx, warnings, true)
}

fn resolve_tokens_with_opts(
    value: &str,
    ctx: &ValueContext,
    warnings: &mut Vec<String>,
    warn_missing: bool,
) -> String {
    let mut out = value.to_string();
    for _ in 0..5 {
        let mut changed = false;
        let mut cursor = 0;
        let chars: Vec<char> = out.chars().collect();
        let mut result = String::with_capacity(out.len());
        while cursor < chars.len() {
            if chars[cursor] == '{' {
                if let Some(end) = chars[cursor + 1..].iter().position(|&c| c == '}') {
                    let token: String = chars[cursor + 1..cursor + 1 + end].iter().collect();
                    let replacement = ctx.lookup(&token);
                    match replacement {
                        Some(val) => {
                            result.push_str(val);
                            cursor += end + 2;
                            changed = true;
                            continue;
                        }
                        None => {
                            if warn_missing {
                                warnings.push(format!("Unknown token {{{token}}}"));
                            }
                        }
                    }
                }
            }
            result.push(chars[cursor]);
            cursor += 1;
        }
        out = result;
        if !changed {
            break;
        }
    }
    out
}

#[derive(Default, Clone)]
struct ValueContext {
    colors: HashMap<String, String>,
    vars: HashMap<String, String>,
}

impl ValueContext {
    fn new(colors: &HashMap<String, String>, vars: &HashMap<String, String>) -> Self {
        ValueContext {
            colors: colors.clone(),
            vars: vars.clone(),
        }
    }

    fn lookup(&self, token: &str) -> Option<&String> {
        if let Some(rem) = token.strip_prefix("colors.") {
            return self.colors.get(rem);
        }
        if let Some(rem) = token.strip_prefix("vars.") {
            return self.vars.get(rem);
        }
        None
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ThemeDocument {
    meta: MetaSection,
    colors: HashMap<String, String>,
    vars: HashMap<String, String>,
    use_gradient: Option<bool>,
    transparent_background: Option<bool>,
    components: ComponentsConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct MetaSection {
    engine: Option<String>,
    name: Option<String>,
    display_name: Option<String>,
    disable_vinyl_thumbnail: Option<bool>,
    transparent_background: Option<bool>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ComponentsConfig {
    root: AreaConfig,
    panel: AreaConfig,
    button: ButtonConfig,
    slider: SliderConfig,
    thumbnail: ThumbnailConfig,
    text: TextComponents,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct AreaConfig {
    background: Option<BackgroundFieldConfig>,
    foreground: Option<String>,
    border_color: Option<String>,
    border_radius: Option<String>,
    border_width: Option<String>,
    show_border: Option<bool>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum BackgroundFieldConfig {
    Simple(String),
    Table(BackgroundTableConfig),
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct BackgroundTableConfig {
    #[serde(rename = "type")]
    kind: Option<String>,
    color: Option<String>,
    start: Option<String>,
    end: Option<String>,
    #[serde(default)]
    direction: GradientDirectionConfig,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GradientDirectionConfig {
    Vertical,
    Horizontal,
}

impl Default for BackgroundTableConfig {
    fn default() -> Self {
        BackgroundTableConfig {
            kind: None,
            color: None,
            start: None,
            end: None,
            direction: GradientDirectionConfig::default(),
        }
    }
}

impl Default for GradientDirectionConfig {
    fn default() -> Self {
        GradientDirectionConfig::Vertical
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ButtonConfig {
    background: Option<String>,
    foreground: Option<String>,
    hover_background: Option<String>,
    active_background: Option<String>,
    border_color: Option<String>,
    border_radius: Option<String>,
    border_width: Option<String>,
    icon: IconConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct IconConfig {
    color: Option<String>,
    size_scale: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct SliderConfig {
    track_fill: Option<String>,
    track_background: Option<String>,
    track_thickness: Option<String>,
    thumb_shape: Option<String>,
    thumb_color: Option<String>,
    thumb_radius: Option<String>,
    thumb_size: Option<String>,
    thumb_image: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ThumbnailConfig {
    corner_radius: Option<String>,
    border_image: Option<String>,
    stroke_color: Option<String>,
    stroke_width: Option<String>,
    overlay_images: Option<Vec<OverlayImageEntry>>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum OverlayImageEntry {
    Path(String),
    Detailed {
        path: String,
        #[serde(default)]
        offset_x: Option<String>,
        #[serde(default)]
        offset_y: Option<String>,
    },
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct TextComponents {
    title: TextConfig,
    body: TextConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct TextConfig {
    color: Option<String>,
    size: Option<String>,
}

impl Default for ThemeDocument {
    fn default() -> Self {
        ThemeDocument {
            meta: MetaSection::default(),
            colors: HashMap::new(),
            vars: HashMap::new(),
            use_gradient: None,
            transparent_background: None,
            components: ComponentsConfig::default(),
        }
    }
}

impl Default for MetaSection {
    fn default() -> Self {
        MetaSection {
            engine: Some(THEME_ENGINE_VERSION.to_string()),
            name: None,
            display_name: None,
            disable_vinyl_thumbnail: None,
            transparent_background: None,
        }
    }
}

impl Default for ComponentsConfig {
    fn default() -> Self {
        ComponentsConfig {
            root: AreaConfig::default(),
            panel: AreaConfig::default(),
            button: ButtonConfig::default(),
            slider: SliderConfig::default(),
            thumbnail: ThumbnailConfig::default(),
            text: TextComponents::default(),
        }
    }
}

impl Default for AreaConfig {
    fn default() -> Self {
        AreaConfig {
            background: None,
            foreground: None,
            border_color: None,
            border_radius: None,
            border_width: None,
            show_border: None,
        }
    }
}

impl Default for ButtonConfig {
    fn default() -> Self {
        ButtonConfig {
            background: None,
            foreground: None,
            hover_background: None,
            active_background: None,
            border_color: None,
            border_radius: None,
            border_width: None,
            icon: IconConfig::default(),
        }
    }
}

impl Default for IconConfig {
    fn default() -> Self {
        IconConfig {
            color: None,
            size_scale: None,
        }
    }
}

impl Default for SliderConfig {
    fn default() -> Self {
        SliderConfig {
            track_fill: None,
            track_background: None,
            track_thickness: None,
            thumb_shape: None,
            thumb_color: None,
            thumb_radius: None,
            thumb_size: None,
            thumb_image: None,
        }
    }
}

impl Default for ThumbnailConfig {
    fn default() -> Self {
        ThumbnailConfig {
            corner_radius: None,
            border_image: None,
            stroke_color: None,
            stroke_width: None,
            overlay_images: None,
        }
    }
}

impl Default for TextComponents {
    fn default() -> Self {
        TextComponents {
            title: TextConfig::default(),
            body: TextConfig::default(),
        }
    }
}

impl Default for TextConfig {
    fn default() -> Self {
        TextConfig {
            color: None,
            size: None,
        }
    }
}

fn merge_documents(base: &mut ThemeDocument, overlay: ThemeDocument) {
    if overlay.meta.engine.is_some() {
        base.meta.engine = overlay.meta.engine;
    }
    if overlay.meta.name.is_some() {
        base.meta.name = overlay.meta.name;
    }
    if overlay.meta.display_name.is_some() {
        base.meta.display_name = overlay.meta.display_name;
    }
    if overlay.use_gradient.is_some() {
        base.use_gradient = overlay.use_gradient;
    }

    base.colors.extend(overlay.colors);
    base.vars.extend(overlay.vars);

    merge_area(&mut base.components.root, overlay.components.root);
    merge_area(&mut base.components.panel, overlay.components.panel);
    merge_button(&mut base.components.button, overlay.components.button);
    merge_slider(&mut base.components.slider, overlay.components.slider);
    merge_thumbnail(&mut base.components.thumbnail, overlay.components.thumbnail);
    merge_text(
        &mut base.components.text.title,
        overlay.components.text.title,
    );
    merge_text(&mut base.components.text.body, overlay.components.text.body);
}

fn merge_area(base: &mut AreaConfig, overlay: AreaConfig) {
    if overlay.background.is_some() {
        base.background = overlay.background;
    }
    if overlay.foreground.is_some() {
        base.foreground = overlay.foreground;
    }
    if overlay.border_color.is_some() {
        base.border_color = overlay.border_color;
    }
    if overlay.border_radius.is_some() {
        base.border_radius = overlay.border_radius;
    }
    if overlay.border_width.is_some() {
        base.border_width = overlay.border_width;
    }
    if overlay.show_border.is_some() {
        base.show_border = overlay.show_border;
    }
}

fn merge_button(base: &mut ButtonConfig, overlay: ButtonConfig) {
    if overlay.background.is_some() {
        base.background = overlay.background;
    }
    if overlay.foreground.is_some() {
        base.foreground = overlay.foreground;
    }
    if overlay.hover_background.is_some() {
        base.hover_background = overlay.hover_background;
    }
    if overlay.active_background.is_some() {
        base.active_background = overlay.active_background;
    }
    if overlay.border_color.is_some() {
        base.border_color = overlay.border_color;
    }
    if overlay.border_radius.is_some() {
        base.border_radius = overlay.border_radius;
    }
    if overlay.border_width.is_some() {
        base.border_width = overlay.border_width;
    }
    merge_icon(&mut base.icon, overlay.icon);
}

fn merge_icon(base: &mut IconConfig, overlay: IconConfig) {
    if overlay.color.is_some() {
        base.color = overlay.color;
    }
    if overlay.size_scale.is_some() {
        base.size_scale = overlay.size_scale;
    }
}

fn merge_slider(base: &mut SliderConfig, overlay: SliderConfig) {
    if overlay.track_fill.is_some() {
        base.track_fill = overlay.track_fill;
    }
    if overlay.track_background.is_some() {
        base.track_background = overlay.track_background;
    }
    if overlay.track_thickness.is_some() {
        base.track_thickness = overlay.track_thickness;
    }
    if overlay.thumb_shape.is_some() {
        base.thumb_shape = overlay.thumb_shape;
    }
    if overlay.thumb_color.is_some() {
        base.thumb_color = overlay.thumb_color;
    }
    if overlay.thumb_radius.is_some() {
        base.thumb_radius = overlay.thumb_radius;
    }
    if overlay.thumb_size.is_some() {
        base.thumb_size = overlay.thumb_size;
    }
    if overlay.thumb_image.is_some() {
        base.thumb_image = overlay.thumb_image;
    }
}

fn merge_thumbnail(base: &mut ThumbnailConfig, overlay: ThumbnailConfig) {
    if overlay.corner_radius.is_some() {
        base.corner_radius = overlay.corner_radius;
    }
    if overlay.border_image.is_some() {
        base.border_image = overlay.border_image;
    }
    if overlay.stroke_color.is_some() {
        base.stroke_color = overlay.stroke_color;
    }
    if overlay.stroke_width.is_some() {
        base.stroke_width = overlay.stroke_width;
    }
    if overlay.overlay_images.is_some() {
        base.overlay_images = overlay.overlay_images;
    }
}

fn merge_text(base: &mut TextConfig, overlay: TextConfig) {
    if overlay.color.is_some() {
        base.color = overlay.color;
    }
    if overlay.size.is_some() {
        base.size = overlay.size;
    }
}

fn builtin_theme_document() -> ThemeDocument {
    toml::from_str(DEFAULT_THEME_TOML).expect("Embedded default theme must parse")
}

const DEFAULT_THEME_TOML: &str = r##"
[meta]
engine = "1"
name = "builtin-windows"
display_name = "Windows 11"

[colors]
background = "#15161b"
panel = "#1d1f26"
accent = "#4c8dff"
accent_hover = "#5e9dff"
accent_active = "#336cff"
text_primary = "#f7f9fc"
text_secondary = "#9ea7b8"
text_on_accent = "#081123"
slider_track_bg = "#2a2c35"
outline = "rgba(76, 141, 255, 0.45)"

[vars]
radius = "18"
slider_thumb_radius = "10"


[components.root]
background = "{colors.background}"
foreground = "{colors.text_primary}"
border_color = "transparent"
border_radius = "{vars.radius}"
border_width = "0"

[components.panel]
background = "{colors.panel}"
foreground = "{colors.text_primary}"
border_color = "transparent"
border_radius = "{vars.radius}"
border_width = "0"

[components.button]
background = "{colors.accent}"
foreground = "{colors.text_on_accent}"
hover_background = "{colors.accent_hover}"
active_background = "{colors.accent_active}"
border_color = "{colors.outline}"
border_radius = "26"
border_width = "1"

[components.button.icon]
color = "{colors.text_on_accent}"
size_scale = "1"

[components.slider]
track_fill = "{colors.accent}"
track_background = "{colors.slider_track_bg}"
track_thickness = "4"
thumb_shape = "circle"
thumb_color = "{colors.accent}"
thumb_radius = "{vars.slider_thumb_radius}"

[components.thumbnail]
corner_radius = "{vars.radius}"
stroke_color = "transparent"
stroke_width = "0"

[components.text.title]
color = "{colors.text_primary}"
size = "20"

[components.text.body]
color = "{colors.text_secondary}"
size = "16"
"##;
