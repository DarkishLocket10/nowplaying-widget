use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
};

use anyhow::{anyhow, Context, Result};
use eframe::egui::epaint::{Mesh, Vertex};
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Pos2, Rect, Rgba, RichText,
    Sense, Stroke, TextureHandle, Vec2,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    layout::{load_layout_from_dir, LayoutSet, LayoutVariant, LoadedLayout},
    theme::{
        load_theme_from_dir, AreaBackground, GradientDirection, GradientSpec, LoadedTheme,
        SliderThumb, Theme,
    },
};

fn to_corner_radius(value: f32) -> CornerRadius {
    CornerRadius::same(value.clamp(0.0, u8::MAX as f32).round() as u8)
}

#[derive(Debug)]
pub struct SkinInfo {
    pub id: String,
    pub display_name: String,
    pub path: PathBuf,
}

pub struct SkinManager {
    root: PathBuf,
    skins: Vec<SkinInfo>,
    current_index: usize,
    current_layout_index: usize,
    theme: Theme,
    layout: LayoutSet,
    warnings: Vec<String>,
    watcher: Option<RecommendedWatcher>,
    changes_rx: Option<Receiver<notify::Result<notify::Event>>>,
    slider_textures: HashMap<PathBuf, TextureHandle>,
    thumbnail_overlay_textures: HashMap<PathBuf, TextureHandle>,
}

impl SkinManager {
    pub fn discover(root: impl AsRef<Path>, default_skin: Option<&str>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut entries = Vec::new();
        if root.exists() {
            for entry in fs::read_dir(&root)
                .with_context(|| format!("Failed to list skins directory: {}", root.display()))?
            {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                match load_theme_from_dir(&path) {
                    Ok(LoadedTheme { theme, .. }) => {
                        entries.push(SkinInfo {
                            id: id.clone(),
                            display_name: theme.display_name.clone(),
                            path,
                        });
                    }
                    Err(err) => {
                        eprintln!("Failed to load skin {id}: {err:?}");
                    }
                }
            }
        }

        if entries.is_empty() {
            return Self::fallback_with_root(root);
        }

        entries.sort_by(|a, b| a.display_name.cmp(&b.display_name));

        let initial_index = default_skin
            .and_then(|name| {
                entries
                    .iter()
                    .position(|s| s.id == name || s.display_name == name)
            })
            .unwrap_or(0);

        let LoadedTheme {
            theme,
            warnings: mut theme_warnings,
        } = load_theme_from_dir(&entries[initial_index].path).with_context(|| {
            format!(
                "Failed to load initial skin: {}",
                entries[initial_index].path.display()
            )
        })?;

        let LoadedLayout {
            layout,
            warnings: mut layout_warnings,
        } = load_layout_from_dir(&entries[initial_index].path).with_context(|| {
            format!(
                "Failed to load layout for initial skin: {}",
                entries[initial_index].path.display()
            )
        })?;

        let mut warnings = Vec::new();
        warnings.append(&mut theme_warnings);
        warnings.append(&mut layout_warnings);

        let layout_index = layout_index_from_set(&layout, Some(&layout.default_variant));

        Ok(Self {
            root,
            skins: entries,
            current_index: initial_index,
            current_layout_index: layout_index,
            theme,
            layout,
            warnings,
            watcher: None,
            changes_rx: None,
            slider_textures: HashMap::new(),
            thumbnail_overlay_textures: HashMap::new(),
        })
    }

    fn fallback_with_root(root: PathBuf) -> Result<Self> {
        let LoadedTheme {
            theme,
            warnings: mut theme_warnings,
        } = load_theme_from_dir(Path::new("."))?;
        let LoadedLayout {
            layout,
            warnings: mut layout_warnings,
        } = load_layout_from_dir(Path::new("."))?;
        let mut warnings = Vec::new();
        warnings.append(&mut theme_warnings);
        warnings.append(&mut layout_warnings);
        Ok(Self {
            root,
            skins: Vec::new(),
            current_index: 0,
            current_layout_index: layout_index_from_set(&layout, Some(&layout.default_variant)),
            theme,
            layout,
            warnings,
            watcher: None,
            changes_rx: None,
            slider_textures: HashMap::new(),
            thumbnail_overlay_textures: HashMap::new(),
        })
    }

    pub fn fallback() -> Result<Self> {
        Self::fallback_with_root(default_skin_root())
    }

    pub fn skin_list(&self) -> &[SkinInfo] {
        &self.skins
    }

    pub fn current_skin_display_name(&self) -> &str {
        if let Some(info) = self.skins.get(self.current_index) {
            &info.display_name
        } else {
            &self.theme.display_name
        }
    }

    pub fn current_skin_id(&self) -> Option<&str> {
        self.skins
            .get(self.current_index)
            .map(|info| info.id.as_str())
    }

    pub fn current_theme(&self) -> &Theme {
        &self.theme
    }

    pub fn layout_options(&self) -> &[LayoutVariant] {
        self.layout.variants()
    }

    pub fn current_layout_id(&self) -> &str {
        let variants = self.layout.variants();
        if variants.is_empty() {
            "default"
        } else {
            let idx = self
                .current_layout_index
                .min(variants.len().saturating_sub(1));
            variants[idx].id.as_str()
        }
    }

    pub fn current_layout_display_name(&self) -> &str {
        let variants = self.layout.variants();
        if variants.is_empty() {
            "Default"
        } else {
            let idx = self
                .current_layout_index
                .min(variants.len().saturating_sub(1));
            variants[idx].display_name.as_str()
        }
    }

    pub fn current_layout_variant(&self) -> &LayoutVariant {
        let variants = self.layout.variants();
        let idx = self
            .current_layout_index
            .min(variants.len().saturating_sub(1));
        &variants[idx]
    }

    pub fn set_layout(&mut self, id: &str, ctx: &egui::Context) -> bool {
        if let Some(idx) = self
            .layout
            .variants()
            .iter()
            .position(|variant| variant.id == id)
        {
            if idx != self.current_layout_index {
                self.current_layout_index = idx;
                ctx.request_repaint();
            }
            true
        } else {
            false
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn set_skin(&mut self, id_or_name: &str, ctx: &egui::Context) -> Result<()> {
        if let Some((index, info)) = self
            .skins
            .iter()
            .enumerate()
            .find(|(_, skin)| skin.id == id_or_name || skin.display_name == id_or_name)
        {
            let previous_layout = self.current_layout_id().to_string();
            let LoadedTheme {
                theme,
                warnings: mut theme_warnings,
            } = load_theme_from_dir(&info.path)?;
            let LoadedLayout {
                layout,
                warnings: mut layout_warnings,
            } = load_layout_from_dir(&info.path)?;
            let mut warnings = Vec::new();
            warnings.append(&mut theme_warnings);
            warnings.append(&mut layout_warnings);
            self.current_index = index;
            self.theme = theme;
            self.layout = layout;
            self.warnings = warnings;
            self.slider_textures.clear();
            self.thumbnail_overlay_textures.clear();
            self.current_layout_index = layout_index_from_set(&self.layout, Some(&previous_layout));
            ctx.request_repaint();
            Ok(())
        } else {
            Err(anyhow!("Skin '{id_or_name}' not found"))
        }
    }

    pub fn enable_hot_reload(&mut self) -> Result<()> {
        if self.watcher.is_some() {
            return Ok(());
        }
        if !self.root.exists() {
            return Err(anyhow!(
                "Skin directory {} does not exist",
                self.root.display()
            ));
        }

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(&self.root, RecursiveMode::Recursive)?;

        self.changes_rx = Some(rx);
        self.watcher = Some(watcher);
        Ok(())
    }

    pub fn disable_hot_reload(&mut self) {
        self.watcher = None;
        self.changes_rx = None;
    }

    pub fn hot_reload_enabled(&self) -> bool {
        self.watcher.is_some()
    }

    pub fn poll_hot_reload(&mut self, ctx: &egui::Context) -> bool {
        let mut reloaded = false;
        let mut events = Vec::new();
        if let Some(rx) = self.changes_rx.as_ref() {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                Ok(evt) => {
                    let relevant = evt.paths.iter().any(|p| {
                        p.extension()
                            .map(|ext| ext.eq_ignore_ascii_case("toml"))
                            .unwrap_or(false)
                    });
                    if relevant {
                        if let Some(id) = self.current_skin_id().map(|s| s.to_owned()) {
                            if let Err(err) = self.set_skin(&id, ctx) {
                                eprintln!("Failed to reload skin {id}: {err}");
                            } else {
                                reloaded = true;
                            }
                        }
                    }
                }
                Err(err) => eprintln!("Skin watcher error: {err}"),
            }
        }

        reloaded
    }

    pub fn apply_style(&self, ctx: &egui::Context) {
        install_fonts(ctx);
        let mut style = (*ctx.style()).clone();
        let components = &self.theme.components;
        let button = &components.button;

        let corner_radius = to_corner_radius(button.border_radius.max(2.0));
        let padding = Vec2::new(
            (button.border_radius * 0.85).clamp(12.0, 24.0),
            (button.border_radius * 0.55).clamp(10.0, 16.0),
        );
        style.spacing.button_padding = padding;
        style.spacing.interact_size.x = style.spacing.interact_size.x.max(88.0);
        style.spacing.interact_size.y = style.spacing.interact_size.y.max(40.0);
        style.spacing.item_spacing.x = style.spacing.item_spacing.x.max(8.0);
        style.spacing.item_spacing.y = style.spacing.item_spacing.y.max(6.0);

        style.visuals.window_fill = components.root.background_color();
        let window_stroke_width = if components.root.show_border {
            components.root.border_width
        } else {
            0.0
        };
        style.visuals.window_stroke =
            Stroke::new(window_stroke_width, components.root.border_color);
        style.visuals.panel_fill = components.panel.background_color();
        style.visuals.override_text_color = Some(components.panel.foreground);

        let panel_bg_color = components.panel.background_color();
        style.visuals.widgets.noninteractive.bg_fill = panel_bg_color;
        style.visuals.widgets.noninteractive.weak_bg_fill = panel_bg_color;
        let panel_border_width = if components.panel.show_border {
            components.panel.border_width
        } else {
            0.0
        };
        style.visuals.widgets.noninteractive.fg_stroke =
            Stroke::new(panel_border_width, components.panel.border_color);
        style.visuals.widgets.noninteractive.corner_radius = corner_radius;

        let border_stroke = Stroke::new(button.border_width, button.border_color);

        style.visuals.widgets.inactive.bg_fill = button.background;
        style.visuals.widgets.inactive.weak_bg_fill = button.background;
        style.visuals.widgets.inactive.fg_stroke = border_stroke;
        style.visuals.widgets.inactive.corner_radius = corner_radius;
        style.visuals.widgets.inactive.expansion = 3.0;

        style.visuals.widgets.hovered.bg_fill = button.hover_background;
        style.visuals.widgets.hovered.weak_bg_fill = button.hover_background;
        style.visuals.widgets.hovered.fg_stroke = border_stroke;
        style.visuals.widgets.hovered.corner_radius = corner_radius;
        style.visuals.widgets.hovered.expansion = 4.0;

        style.visuals.widgets.active.bg_fill = button.active_background;
        style.visuals.widgets.active.weak_bg_fill = button.active_background;
        style.visuals.widgets.active.fg_stroke = border_stroke;
        style.visuals.widgets.active.corner_radius = corner_radius;
        style.visuals.widgets.active.expansion = 2.0;

        style.visuals.selection.bg_fill = button.background;
        style.visuals.selection.stroke = border_stroke;
        style.visuals.hyperlink_color = button.foreground;

        ctx.set_style(style);
    }

    pub fn skin_button(&self, ui: &mut egui::Ui, label: impl Into<String>) -> egui::Response {
        self.skin_button_scaled(ui, label, 1.0)
    }

    pub fn skin_button_scaled(
        &self,
        ui: &mut egui::Ui,
        label: impl Into<String>,
        scale: f32,
    ) -> egui::Response {
        let label = label.into();
        let clamped_scale = scale.clamp(0.6, 1.0);
        let button = &self.theme.components.button;
        let body_size = self.theme.components.text_body.size;
        let border_stroke = Stroke::new(button.border_width.max(1.0), button.border_color);

        let style = ui.style();
        let base_padding = style.spacing.button_padding;
        let scaled_padding = Vec2::new(
            (base_padding.x * clamped_scale).clamp(base_padding.x * 0.6, base_padding.x),
            (base_padding.y * clamped_scale).clamp(base_padding.y * 0.6, base_padding.y),
        );
        let base_min_width = style.spacing.interact_size.x.max(96.0);
        let base_min_height = style.spacing.interact_size.y.max(40.0);
        let min_width = (base_min_width * clamped_scale).clamp(60.0, base_min_width);
        let min_height = (base_min_height * clamped_scale).clamp(28.0, base_min_height);
        let text_scale = clamped_scale.clamp(0.75, 1.0);
        let rich = RichText::new(label.clone())
            .color(button.foreground)
            .size((body_size + 2.0) * text_scale)
            .strong();

        ui.scope(|scaled_ui| {
            scaled_ui.spacing_mut().button_padding = scaled_padding;
            scaled_ui.add_sized(
                Vec2::new(min_width, min_height),
                egui::Button::new(rich)
                    .fill(button.background)
                    .corner_radius(to_corner_radius(button.border_radius))
                    .stroke(border_stroke)
                    .wrap(),
            )
        })
        .inner
    }

    pub fn skin_text(&self, ui: &mut egui::Ui, text: impl Into<String>, title: bool) {
        let style = if title {
            &self.theme.components.text_title
        } else {
            &self.theme.components.text_body
        };
        ui.label(
            RichText::new(text.into())
                .color(style.color)
                .size(style.size),
        );
    }

    pub fn skin_slider(
        &mut self,
        ui: &mut egui::Ui,
        value: &mut f64,
        range: std::ops::RangeInclusive<f64>,
    ) -> egui::Response {
        let slider = self.theme.components.slider.clone();
        let min = *range.start();
        let max = *range.end();
        let span = (max - min).max(f64::MIN_POSITIVE);
        let fraction = ((*value - min) / span).clamp(0.0, 1.0) as f32;

        let (thumb_half_width, thumb_height) = match &slider.thumb {
            SliderThumb::Circle { radius, .. } => (*radius, radius * 2.0),
            SliderThumb::Image { size, .. } => (size.x / 2.0, size.y),
        };

        let desired_height = thumb_height.max(slider.track_thickness) + 8.0;
        let width = ui.available_width();
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, desired_height), Sense::click_and_drag());

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let available_width = rect.width().max(1.0);
        let thumb_guard = thumb_half_width.min(available_width / 2.0);
        let track_min_x = rect.min.x + thumb_guard;
        let track_max_x = rect.max.x - thumb_guard;
        let track_width = (track_max_x - track_min_x).max(1.0);

        if response.dragged() || response.drag_started() || response.clicked() {
            if let Some(pos) = ui.input(|input| input.pointer.interact_pos()) {
                let t = ((pos.x - track_min_x) / track_width).clamp(0.0, 1.0);
                let new_value = min + span * t as f64;
                if (new_value - *value).abs() > f64::EPSILON {
                    *value = new_value;
                    response.mark_changed();
                }
            }
        }

        let painter = ui.painter_at(rect);
        let track_rect = Rect::from_min_max(
            Pos2::new(track_min_x, rect.center().y - slider.track_thickness / 2.0),
            Pos2::new(track_max_x, rect.center().y + slider.track_thickness / 2.0),
        );
        let rounding = to_corner_radius(slider.track_thickness / 2.0);
        painter.rect_filled(track_rect, rounding, slider.track_background);

        if fraction > 0.0 {
            let fill_rect = Rect::from_min_max(
                track_rect.min,
                Pos2::new(track_rect.min.x + track_width * fraction, track_rect.max.y),
            );
            painter.rect_filled(fill_rect, rounding, slider.track_fill);
        }

        let thumb_center = Pos2::new(track_min_x + track_width * fraction, track_rect.center().y);
        match &slider.thumb {
            SliderThumb::Circle { color, radius } => {
                painter.circle_filled(thumb_center, *radius, *color);
            }
            SliderThumb::Image { color, path, size } => {
                if let Some(texture) = self.ensure_texture(ui.ctx(), path, true) {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    let rect = Rect::from_center_size(thumb_center, *size);
                    painter.image(texture.id(), rect, uv, *color);
                } else {
                    painter.circle_filled(thumb_center, size.x.min(size.y) / 2.0, *color);
                }
            }
        }

        response
    }

    fn ensure_texture(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        is_slider: bool,
    ) -> Option<TextureHandle> {
        let cache = if is_slider {
            &mut self.slider_textures
        } else {
            &mut self.thumbnail_overlay_textures
        };

        if let Some(handle) = cache.get(path) {
            return Some(handle.clone());
        }

        match load_texture_from_path(ctx, path) {
            Ok(texture) => {
                let cloned = texture.clone();
                cache.insert(path.to_path_buf(), texture);
                Some(cloned)
            }
            Err(err) => {
                eprintln!("Failed to load texture {}: {err}", path.display());
                None
            }
        }
    }

    pub fn thumbnail_overlay_textures(
        &mut self,
        ctx: &egui::Context,
    ) -> Vec<(TextureHandle, egui::Vec2)> {
        self.theme
            .components
            .thumbnail
            .overlays
            .clone()
            .into_iter()
            .filter_map(|overlay| {
                self.ensure_texture(ctx, overlay.path.as_path(), false)
                    .map(|texture| (texture, overlay.offset))
            })
            .collect()
    }
}

pub fn paint_area_background(
    painter: &egui::Painter,
    rect: Rect,
    rounding: CornerRadius,
    background: &AreaBackground,
) {
    match background {
        AreaBackground::Solid(color) => {
            painter.rect_filled(rect, rounding, *color);
        }
        AreaBackground::Gradient(gradient) => {
            paint_gradient_rect(painter, rect, rounding, gradient);
        }
    }
}

fn paint_gradient_rect(
    painter: &egui::Painter,
    rect: Rect,
    rounding: CornerRadius,
    gradient: &GradientSpec,
) {
    if rect.width() <= f32::EPSILON || rect.height() <= f32::EPSILON {
        painter.rect_filled(rect, rounding, gradient.start);
        return;
    }

    if gradient.start == gradient.end {
        painter.rect_filled(rect, rounding, gradient.start);
        return;
    }

    let radii = CornerRadiiF32::from_rect(rounding, rect);
    let mut mesh = Mesh::default();

    match gradient.direction {
        GradientDirection::Vertical => {
            tessellate_vertical_gradient(&mut mesh, rect, &radii, gradient.start, gradient.end)
        }
        GradientDirection::Horizontal => {
            tessellate_horizontal_gradient(&mut mesh, rect, &radii, gradient.start, gradient.end)
        }
    }

    painter.add(egui::Shape::mesh(mesh));
}

fn tessellate_vertical_gradient(
    mesh: &mut Mesh,
    rect: Rect,
    radii: &CornerRadiiF32,
    start: Color32,
    end: Color32,
) {
    let height = rect.height().max(1.0);
    let steps = gradient_steps(height);
    let step_height = height / steps as f32;

    for i in 0..steps {
        let y0 = rect.min.y + step_height * i as f32;
        let y1 = if i == steps - 1 {
            rect.max.y
        } else {
            (y0 + step_height).min(rect.max.y)
        };

        let (left0, right0) = horizontal_span(rect, radii, y0);
        let (left1, right1) = horizontal_span(rect, radii, y1);

        let t0 = ((y0 - rect.min.y) / height).clamp(0.0, 1.0);
        let t1 = ((y1 - rect.min.y) / height).clamp(0.0, 1.0);
        let color0 = lerp_color(start, end, t0);
        let color1 = lerp_color(start, end, t1);

        let v0 = push_vertex(mesh, Pos2::new(left0, y0), color0);
        let v1 = push_vertex(mesh, Pos2::new(right0, y0), color0);
        let v2 = push_vertex(mesh, Pos2::new(left1, y1), color1);
        let v3 = push_vertex(mesh, Pos2::new(right1, y1), color1);

        mesh.add_triangle(v0, v2, v1);
        mesh.add_triangle(v1, v2, v3);
    }
}

fn tessellate_horizontal_gradient(
    mesh: &mut Mesh,
    rect: Rect,
    radii: &CornerRadiiF32,
    start: Color32,
    end: Color32,
) {
    let width = rect.width().max(1.0);
    let steps = gradient_steps(width);
    let step_width = width / steps as f32;

    for i in 0..steps {
        let x0 = rect.min.x + step_width * i as f32;
        let x1 = if i == steps - 1 {
            rect.max.x
        } else {
            (x0 + step_width).min(rect.max.x)
        };

        let (top0, bottom0) = vertical_span(rect, radii, x0);
        let (top1, bottom1) = vertical_span(rect, radii, x1);

        let t0 = ((x0 - rect.min.x) / width).clamp(0.0, 1.0);
        let t1 = ((x1 - rect.min.x) / width).clamp(0.0, 1.0);
        let color0 = lerp_color(start, end, t0);
        let color1 = lerp_color(start, end, t1);

        let v0 = push_vertex(mesh, Pos2::new(x0, top0), color0);
        let v1 = push_vertex(mesh, Pos2::new(x0, bottom0), color0);
        let v2 = push_vertex(mesh, Pos2::new(x1, top1), color1);
        let v3 = push_vertex(mesh, Pos2::new(x1, bottom1), color1);

        mesh.add_triangle(v0, v1, v2);
        mesh.add_triangle(v1, v3, v2);
    }
}

fn gradient_steps(length: f32) -> usize {
    const MAX_STEPS: usize = 128;
    let approx = length.abs().ceil() as usize;
    approx.clamp(1, MAX_STEPS)
}

fn lerp_color(start: Color32, end: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let a = Rgba::from(start);
    let b = Rgba::from(end);
    Color32::from(a * (1.0 - t) + b * t)
}

fn push_vertex(mesh: &mut Mesh, pos: Pos2, color: Color32) -> u32 {
    let idx = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        pos,
        uv: Pos2::new(0.0, 0.0),
        color,
    });
    idx
}

fn horizontal_span(rect: Rect, radii: &CornerRadiiF32, y: f32) -> (f32, f32) {
    let y = y.clamp(rect.min.y, rect.max.y);
    let mut left = rect.min.x;
    let mut right = rect.max.x;

    if radii.nw > 0.0 && y <= rect.min.y + radii.nw {
        let center = Pos2::new(rect.min.x + radii.nw, rect.min.y + radii.nw);
        let dy = center.y - y;
        if dy.abs() <= radii.nw {
            let dx = (radii.nw * radii.nw - dy * dy).max(0.0).sqrt();
            left = left.max(center.x - dx);
        }
    }

    if radii.sw > 0.0 && y >= rect.max.y - radii.sw {
        let center = Pos2::new(rect.min.x + radii.sw, rect.max.y - radii.sw);
        let dy = y - center.y;
        if dy.abs() <= radii.sw {
            let dx = (radii.sw * radii.sw - dy * dy).max(0.0).sqrt();
            left = left.max(center.x - dx);
        }
    }

    if radii.ne > 0.0 && y <= rect.min.y + radii.ne {
        let center = Pos2::new(rect.max.x - radii.ne, rect.min.y + radii.ne);
        let dy = center.y - y;
        if dy.abs() <= radii.ne {
            let dx = (radii.ne * radii.ne - dy * dy).max(0.0).sqrt();
            right = right.min(center.x + dx);
        }
    }

    if radii.se > 0.0 && y >= rect.max.y - radii.se {
        let center = Pos2::new(rect.max.x - radii.se, rect.max.y - radii.se);
        let dy = y - center.y;
        if dy.abs() <= radii.se {
            let dx = (radii.se * radii.se - dy * dy).max(0.0).sqrt();
            right = right.min(center.x + dx);
        }
    }

    if left > right {
        let mid = (left + right) * 0.5;
        (mid, mid)
    } else {
        (left, right)
    }
}

fn vertical_span(rect: Rect, radii: &CornerRadiiF32, x: f32) -> (f32, f32) {
    let x = x.clamp(rect.min.x, rect.max.x);
    let mut top = rect.min.y;
    let mut bottom = rect.max.y;

    if radii.nw > 0.0 && x <= rect.min.x + radii.nw {
        let center = Pos2::new(rect.min.x + radii.nw, rect.min.y + radii.nw);
        let dx = center.x - x;
        if dx.abs() <= radii.nw {
            let dy = (radii.nw * radii.nw - dx * dx).max(0.0).sqrt();
            top = top.max(center.y - dy);
        }
    }

    if radii.ne > 0.0 && x >= rect.max.x - radii.ne {
        let center = Pos2::new(rect.max.x - radii.ne, rect.min.y + radii.ne);
        let dx = x - center.x;
        if dx.abs() <= radii.ne {
            let dy = (radii.ne * radii.ne - dx * dx).max(0.0).sqrt();
            top = top.max(center.y - dy);
        }
    }

    if radii.sw > 0.0 && x <= rect.min.x + radii.sw {
        let center = Pos2::new(rect.min.x + radii.sw, rect.max.y - radii.sw);
        let dx = center.x - x;
        if dx.abs() <= radii.sw {
            let dy = (radii.sw * radii.sw - dx * dx).max(0.0).sqrt();
            bottom = bottom.min(center.y + dy);
        }
    }

    if radii.se > 0.0 && x >= rect.max.x - radii.se {
        let center = Pos2::new(rect.max.x - radii.se, rect.max.y - radii.se);
        let dx = x - center.x;
        if dx.abs() <= radii.se {
            let dy = (radii.se * radii.se - dx * dx).max(0.0).sqrt();
            bottom = bottom.min(center.y + dy);
        }
    }

    if top > bottom {
        let mid = (top + bottom) * 0.5;
        (mid, mid)
    } else {
        (top, bottom)
    }
}

struct CornerRadiiF32 {
    nw: f32,
    ne: f32,
    sw: f32,
    se: f32,
}

impl CornerRadiiF32 {
    fn from_rect(rounding: CornerRadius, rect: Rect) -> Self {
        let max_width = (rect.width() * 0.5).max(0.0);
        let max_height = (rect.height() * 0.5).max(0.0);
        let limit = max_width.min(max_height);
        let clamp = |value: u8| -> f32 { (value as f32).min(limit).max(0.0) };
        Self {
            nw: clamp(rounding.nw),
            ne: clamp(rounding.ne),
            sw: clamp(rounding.sw),
            se: clamp(rounding.se),
        }
    }
}
fn install_fonts(ctx: &egui::Context) {
    const LATO_REGULAR: &[u8] = include_bytes!("../assets/fonts/Lato-Regular.ttf");
    const LATO_BOLD: &[u8] = include_bytes!("../assets/fonts/Lato-Bold.ttf");

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "lato-regular".into(),
        Arc::new(FontData::from_static(LATO_REGULAR)),
    );
    fonts.font_data.insert(
        "lato-bold".into(),
        Arc::new(FontData::from_static(LATO_BOLD)),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "lato-regular".into());
    fonts
        .families
        .entry(FontFamily::Name("body".into()))
        .or_default()
        .insert(0, "lato-regular".into());
    fonts
        .families
        .entry(FontFamily::Name("strong".into()))
        .or_default()
        .insert(0, "lato-bold".into());
    fonts
        .families
        .entry(FontFamily::Name("bold".into()))
        .or_default()
        .insert(0, "lato-bold".into());

    ctx.set_fonts(fonts);
}

fn load_texture_from_path(ctx: &egui::Context, path: &Path) -> Result<TextureHandle> {
    let data = fs::read(path)
        .with_context(|| format!("Unable to open texture image: {}", path.display()))?;
    let reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .with_context(|| format!("Unable to determine image format: {}", path.display()))?;
    let image = reader
        .decode()
        .with_context(|| format!("Failed to decode texture image: {}", path.display()))?;
    let image = image.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Ok(ctx.load_texture(
        format!("skin-thumb-{}", path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

pub fn default_skin_root() -> PathBuf {
    PathBuf::from("skins")
}

fn layout_index_from_set(layout: &LayoutSet, preferred: Option<&str>) -> usize {
    let variants = layout.variants();
    if variants.is_empty() {
        return 0;
    }

    if let Some(id) = preferred {
        if let Some(idx) = variants.iter().position(|variant| variant.id == id) {
            return idx;
        }
    }

    if let Some(idx) = variants
        .iter()
        .position(|variant| variant.id == layout.default_variant)
    {
        return idx;
    }

    0
}
