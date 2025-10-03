mod config;
mod layout;
mod theme;
mod ui_skin;
mod vinyl;

use crate::{
    config::Config,
    layout::{ComponentNode, ContainerNode, LayoutAlign, LayoutComponent, LayoutNode},
    theme::{AreaBackground, GradientDirection, GradientSpec},
    vinyl::{render_vinyl, VinylSpin, VinylThumbnailOptions},
};
use eframe::egui::{
    self, Align2, ColorImage, CornerRadius, FontId, LayerId, PointerButton, ResizeDirection,
    TextureHandle, TextureOptions, UiBuilder, ViewportCommand, WindowLevel, ViewportBuilder,
};
use futures::executor::block_on;
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::future::IntoFuture;
use std::{
    cmp::Reverse,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};
use ui_skin::{default_skin_root, paint_area_background, SkinManager};
use windows::{
    core::Result as WinResult,
    Foundation::TimeSpan,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession,
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionMediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    },
    Storage::Streams::{
        DataReader, IRandomAccessStreamReference, IRandomAccessStreamWithContentType,
        InputStreamOptions,
    },
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

#[cfg(target_os = "windows")]
use windows::UI::ViewManagement::UISettings;

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
        DWMWCP_ROUND,
    },
};

const TICKS_PER_SECOND: f64 = 10_000_000.0;

const PLAYBACK_CONTROLS_MAX_WIDTH: f32 = 420.0;
const PLAYBACK_CONTROL_SPACING_X: f32 = 12.0;
const TIMELINE_PADDING_RATIO: f32 = 0.06;
const TIMELINE_PADDING_MIN: f32 = 12.0;
const TIMELINE_PADDING_MAX: f32 = 32.0;
const TIMELINE_MIN_CONTENT_WIDTH: f32 = 160.0;
const TIMELINE_MAX_CONTENT_WIDTH: f32 = 720.0;
const TIMELINE_LABEL_GAP: f32 = 16.0;
const DWM_COLOR_UNSET: u32 = 0xFFFFFFFF;

#[cfg(target_os = "windows")]
#[derive(Default)]
struct WindowsTitlebarState {
    last_caption: Option<u32>,
    last_text: Option<u32>,
    last_border: Option<u32>,
    last_dark_mode: Option<bool>,
}

#[cfg(target_os = "windows")]
fn color32_to_colorref(color: egui::Color32) -> u32 {
    let [r, g, b, _] = color.to_array();
    (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r)
}

#[cfg(target_os = "windows")]
fn is_dark_color(color: egui::Color32) -> bool {
    let [r, g, b, _] = color.to_array();
    let luminance = 0.2126 * (r as f32) + 0.7152 * (g as f32) + 0.0722 * (b as f32);
    luminance < 128.0
}

#[cfg(target_os = "windows")]
fn animations_enabled_from_system() -> bool {
    if let Ok(settings) = UISettings::new() {
        if let Ok(enabled) = settings.AnimationsEnabled() {
            return enabled;
        }
    }
    true
}

#[cfg(not(target_os = "windows"))]
fn animations_enabled_from_system() -> bool {
    true
}

#[derive(Debug, Copy, Clone)]
struct StripMetrics {
    total_width: f32,
    content_width: f32,
    margin: f32,
}

impl StripMetrics {
    fn from_content(total_width: f32, content_width: f32) -> Self {
        let total = total_width.max(1.0);
        let content = content_width.clamp(1.0, total);
        let margin = ((total - content) / 2.0).max(0.0);
        Self {
            total_width: total,
            content_width: content,
            margin,
        }
    }

    #[allow(dead_code)]
    fn center_with_max(total_width: f32, max_content_width: f32) -> Self {
        let target = max_content_width.max(1.0).min(total_width.max(1.0));
        Self::from_content(total_width, target)
    }

    fn padded(total_width: f32, padding: f32) -> Self {
        let total = total_width.max(1.0);
        let margin = padding.clamp(0.0, total / 2.0);
        let content = (total - 2.0 * margin).max(1.0);
        Self {
            total_width: total,
            content_width: content,
            margin,
        }
    }

    fn content_width(&self) -> f32 {
        self.content_width
    }

    fn show<R>(&self, ui: &mut egui::Ui, builder: impl FnOnce(&mut egui::Ui) -> R) -> R {
        ui.allocate_ui_with_layout(
            egui::vec2(self.total_width, 0.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |row| {
                if self.margin > 0.0 {
                    row.add_space(self.margin);
                }
                let result = row
                    .allocate_ui_with_layout(
                        egui::vec2(self.content_width, 0.0),
                        egui::Layout::top_down(egui::Align::Center),
                        builder,
                    )
                    .inner;
                if self.margin > 0.0 {
                    row.add_space(self.margin);
                }
                result
            },
        )
        .inner
    }

    #[allow(dead_code)]
    fn show_with_layout<R>(
        &self,
        ui: &mut egui::Ui,
        layout: egui::Layout,
        builder: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        self.show(ui, |inner| inner.with_layout(layout, builder).inner)
    }

    /// Show the strip and anchor its content left/center/right within the total width.
    ///
    /// `align` controls which side the content is anchored to. This replaces manual
    /// left/right spacer calculations and makes anchoring explicit and robust.
    fn show_anchored<R>(
        &self,
        ui: &mut egui::Ui,
        align: egui::Align,
        builder: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        ui.allocate_ui_with_layout(
            egui::vec2(self.total_width, 0.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |row| {
                let extra = (self.total_width - self.content_width).max(0.0);
                let (left_space, right_space) = match align {
                    egui::Align::Min => (0.0, extra),
                    egui::Align::Center => (extra / 2.0, extra / 2.0),
                    egui::Align::Max => (extra, 0.0),
                };

                if left_space > 0.0 {
                    row.add_space(left_space);
                }

                let result = row
                    .allocate_ui_with_layout(
                        egui::vec2(self.content_width, 0.0),
                        egui::Layout::top_down(egui::Align::Center),
                        builder,
                    )
                    .inner;

                if right_space > 0.0 {
                    row.add_space(right_space);
                }

                result
            },
        )
        .inner
    }
}

fn timeline_strip_metrics(total_width: f32, centered: bool) -> StripMetrics {
    let total = total_width.max(1.0);
    let mut padding =
        (total * TIMELINE_PADDING_RATIO).clamp(TIMELINE_PADDING_MIN, TIMELINE_PADDING_MAX);
    let max_padding = (total - TIMELINE_MIN_CONTENT_WIDTH).max(0.0) / 2.0;
    padding = padding.min(max_padding);
    let content_candidate = (total - 2.0 * padding).max(1.0);

    if centered {
        let limited = content_candidate
            .min(TIMELINE_MAX_CONTENT_WIDTH.min(total))
            .max(TIMELINE_MIN_CONTENT_WIDTH.min(total));
        StripMetrics::from_content(total, limited)
    } else {
        StripMetrics::padded(total, padding)
    }
}

type SnapshotResult = std::result::Result<(NowPlaying, Option<Timeline>), String>;

#[derive(Clone, Default)]
struct NowPlaying {
    title: String,
    artist: String,
    album: String,
    state: PlayState,
}

impl PartialEq for NowPlaying {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title && self.artist == other.artist && self.album == other.album
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayState {
    Closed,
    Opened,
    Changing,
    Stopped,
    Playing,
    Paused,
    Unknown,
}

impl Default for PlayState {
    fn default() -> Self {
        PlayState::Unknown
    }
}

#[derive(Clone, Debug)]
struct Timeline {
    start_secs: f64,
    end_secs: f64,
    position_secs: f64,
    can_seek: bool,
}

impl Timeline {
    fn duration_secs(&self) -> f64 {
        (self.end_secs - self.start_secs).max(0.0)
    }
}

struct ThumbnailMessage {
    request_id: u64,
    track: NowPlaying,
    hash: Option<u64>,
    base_image: Option<ColorImage>,
    vinyl_image: Option<ColorImage>,
    error: Option<String>,
}

#[derive(Clone)]
enum PendingThumbnail {
    Update {
        track: NowPlaying,
        hash: u64,
        base_image: ColorImage,
        vinyl_image: Option<ColorImage>,
    },
    Clear {
        track: Option<NowPlaying>,
    },
}

enum SnapshotCommand {
    Fetch,
    Shutdown,
}

#[derive(Clone, Copy)]
enum PlaybackButtonKind {
    Previous,
    PlayPause,
    Next,
}

#[derive(Clone, Copy)]
enum ThumbnailOverlayAction {
    Previous,
    Play,
    Pause,
    Next,
}

#[derive(Clone, Copy)]
struct ThumbnailOverlayGeometry {
    rect: egui::Rect,
    icon_slot: f32,
    icon_spacing: f32,
    height: f32,
}

fn time_span_to_secs(span: TimeSpan) -> f64 {
    span.Duration as f64 / TICKS_PER_SECOND
}

fn secs_to_ticks(seconds: f64) -> i64 {
    if !seconds.is_finite() {
        return if seconds.is_sign_positive() {
            i64::MAX
        } else {
            i64::MIN
        };
    }

    let ticks_f = seconds * TICKS_PER_SECOND;
    if !ticks_f.is_finite() {
        return if ticks_f.is_sign_positive() {
            i64::MAX
        } else {
            i64::MIN
        };
    }

    if ticks_f >= i64::MAX as f64 {
        return i64::MAX;
    }
    if ticks_f <= i64::MIN as f64 {
        return i64::MIN;
    }

    ticks_f.round() as i64
}

fn format_timestamp(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0).floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

fn playstate_to_str(state: PlayState) -> &'static str {
    match state {
        PlayState::Closed => "Closed",
        PlayState::Opened => "Opened",
        PlayState::Changing => "Changing",
        PlayState::Stopped => "Stopped",
        PlayState::Playing => "Playing",
        PlayState::Paused => "Paused",
        PlayState::Unknown => "Unknown",
    }
}

fn hash_bytes(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

fn decode_thumbnail_image(bytes: &[u8]) -> std::result::Result<ColorImage, String> {
    let image =
        image::load_from_memory(bytes).map_err(|e| format!("Failed to decode thumbnail: {e}"))?;
    let image = image.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    Ok(ColorImage::from_rgba_unmultiplied(size, &pixels))
}

#[derive(Clone, Copy)]
struct Cluster {
    centroid: [f32; 3],
    count: usize,
}

fn sample_pixels(image: &ColorImage, max_samples: usize) -> Vec<[f32; 3]> {
    if max_samples == 0 {
        return Vec::new();
    }

    let total = image.pixels.len();
    if total == 0 {
        return Vec::new();
    }

    let step = (total / max_samples).max(1);
    let mut samples = Vec::with_capacity(max_samples.min(total));

    for pixel in image.pixels.iter().step_by(step) {
        if pixel.a() < 16 {
            continue;
        }
        samples.push([pixel.r() as f32, pixel.g() as f32, pixel.b() as f32]);
        if samples.len() >= max_samples {
            break;
        }
    }

    samples
}

fn squared_distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    dr * dr + dg * dg + db * db
}

fn kmeans_clusters(samples: &[[f32; 3]], k: usize, max_iter: usize) -> Vec<Cluster> {
    if samples.is_empty() || k == 0 {
        return Vec::new();
    }

    let mut centroids = Vec::with_capacity(k);
    for i in 0..k {
        let idx = (i * samples.len()) / k;
        let idx = idx.min(samples.len() - 1);
        centroids.push(samples[idx]);
    }

    let mut assignments = vec![0usize; samples.len()];

    for iter in 0..max_iter {
        let mut sums = vec![[0f32; 3]; k];
        let mut counts = vec![0usize; k];

        for (sample_idx, sample) in samples.iter().enumerate() {
            let mut best = 0usize;
            let mut best_dist = f32::MAX;
            for (centroid_idx, centroid) in centroids.iter().enumerate() {
                let dist = squared_distance(sample, centroid);
                if dist < best_dist {
                    best_dist = dist;
                    best = centroid_idx;
                }
            }

            assignments[sample_idx] = best;
            for channel in 0..3 {
                sums[best][channel] += sample[channel];
            }
            counts[best] += 1;
        }

        let mut changed = false;
        for i in 0..k {
            if counts[i] == 0 {
                centroids[i] = samples[(i + iter) % samples.len()];
                changed = true;
                continue;
            }
            let new_centroid = [
                sums[i][0] / counts[i] as f32,
                sums[i][1] / counts[i] as f32,
                sums[i][2] / counts[i] as f32,
            ];
            if squared_distance(&centroids[i], &new_centroid) > 1e-2 {
                changed = true;
            }
            centroids[i] = new_centroid;
        }

        if !changed {
            break;
        }
    }

    let mut counts = vec![0usize; k];
    for &assignment in &assignments {
        counts[assignment] += 1;
    }

    centroids
        .into_iter()
        .enumerate()
        .map(|(idx, centroid)| Cluster {
            centroid,
            count: counts[idx],
        })
        .collect()
}

fn color_from_centroid(centroid: [f32; 3]) -> egui::Color32 {
    let r = centroid[0].clamp(0.0, 255.0).round() as u8;
    let g = centroid[1].clamp(0.0, 255.0).round() as u8;
    let b = centroid[2].clamp(0.0, 255.0).round() as u8;
    egui::Color32::from_rgb(r, g, b)
}

fn color_distance_sq(a: egui::Color32, b: egui::Color32) -> f32 {
    let dr = a.r() as f32 - b.r() as f32;
    let dg = a.g() as f32 - b.g() as f32;
    let db = a.b() as f32 - b.b() as f32;
    dr * dr + dg * dg + db * db
}

fn luminance(color: egui::Color32) -> f32 {
    0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32
}

fn order_by_luminance(a: egui::Color32, b: egui::Color32) -> (egui::Color32, egui::Color32) {
    if luminance(a) <= luminance(b) {
        (a, b)
    } else {
        (b, a)
    }
}

fn dominant_gradient_colors(image: &ColorImage) -> Option<[egui::Color32; 2]> {
    const MAX_SAMPLES: usize = 6_000;
    const K: usize = 3;
    const MAX_ITER: usize = 10;
    const DISTINCT_THRESHOLD: f32 = 400.0;

    let samples = sample_pixels(image, MAX_SAMPLES);
    if samples.len() < 2 {
        return None;
    }

    let k = K.min(samples.len()).max(1);
    let mut clusters = kmeans_clusters(&samples, k, MAX_ITER);
    if clusters.is_empty() {
        return None;
    }

    clusters.sort_by_key(|cluster| Reverse(cluster.count));

    let mut unique = Vec::new();
    for cluster in clusters {
        if cluster.count == 0 {
            continue;
        }
        let color = color_from_centroid(cluster.centroid);
        if unique
            .iter()
            .all(|&(existing, _)| color_distance_sq(existing, color) > DISTINCT_THRESHOLD)
        {
            unique.push((color, cluster.count));
        }
    }

    if unique.len() < 2 {
        return None;
    }

    let primary = unique[0].0;
    let secondary = unique[1].0;
    let (start, end) = order_by_luminance(primary, secondary);
    Some([start, end])
}

fn gradient_direction_from_background(background: &AreaBackground) -> GradientDirection {
    match background {
        AreaBackground::Gradient(spec) => spec.direction,
        AreaBackground::Solid(_) => GradientDirection::Vertical,
    }
}

fn dynamic_gradient_from_image(
    image: &ColorImage,
    direction: GradientDirection,
) -> Option<GradientSpec> {
    dominant_gradient_colors(image).map(|[start, end]| GradientSpec {
        start,
        end,
        direction,
    })
}

fn load_thumbnail_bytes(
    props: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> WinResult<Option<Vec<u8>>> {
    let reference: IRandomAccessStreamReference = match props.Thumbnail() {
        Ok(reference) => reference,
        Err(_) => return Ok(None),
    };

    let stream: IRandomAccessStreamWithContentType =
        block_on_operation(reference.OpenReadAsync()?)?;
    let input_stream = stream.GetInputStreamAt(0)?;
    let reader = DataReader::CreateDataReader(&input_stream)?;
    reader.SetInputStreamOptions(InputStreamOptions::Partial)?;

    let mut buffer = Vec::new();
    const CHUNK: u32 = 64 * 1024;

    loop {
        let loaded = block_on_operation(reader.LoadAsync(CHUNK)?)?;
        if loaded == 0 {
            break;
        }
        let mut chunk = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut chunk)?;
        buffer.extend_from_slice(&chunk);
        if loaded < CHUNK {
            break;
        }
    }

    Ok(Some(buffer))
}

fn block_on_operation<O, T>(operation: O) -> WinResult<T>
where
    O: IntoFuture<Output = WinResult<T>>,
{
    block_on(operation.into_future())
}

fn current_session() -> WinResult<GlobalSystemMediaTransportControlsSession> {
    let manager =
        block_on_operation(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?)?;
    manager.GetCurrentSession()
}

fn fetch_session_snapshot() -> WinResult<(NowPlaying, Option<Timeline>)> {
    let session = current_session()?;

    let props = block_on_operation(session.TryGetMediaPropertiesAsync()?)?;
    let playback_info = session.GetPlaybackInfo()?;
    let status = playback_info.PlaybackStatus()?;

    let state = match status {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed => PlayState::Closed,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Opened => PlayState::Opened,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Changing => PlayState::Changing,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped => PlayState::Stopped,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => PlayState::Playing,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => PlayState::Paused,
        _ => PlayState::Unknown,
    };

    let now = NowPlaying {
        title: props.Title()?.to_string_lossy(),
        artist: props.Artist()?.to_string_lossy(),
        album: props.AlbumTitle()?.to_string_lossy(),
        state,
    };

    let timeline_props = session.GetTimelineProperties()?;
    let mut start_secs = time_span_to_secs(timeline_props.StartTime()?);
    let mut end_secs = time_span_to_secs(timeline_props.EndTime()?);
    let mut position_secs = time_span_to_secs(timeline_props.Position()?);

    if end_secs < start_secs {
        std::mem::swap(&mut start_secs, &mut end_secs);
    }
    if !position_secs.is_finite() {
        position_secs = start_secs;
    }
    position_secs = position_secs.clamp(start_secs, end_secs.max(start_secs));

    let can_seek = (end_secs - start_secs).abs() > f64::EPSILON;

    let timeline = Timeline {
        start_secs,
        end_secs,
        position_secs,
        can_seek,
    };

    let timeline = if timeline.duration_secs() <= f64::EPSILON && !can_seek {
        None
    } else {
        Some(timeline)
    };

    Ok((now, timeline))
}

fn fetch_thumbnail_bytes() -> WinResult<Option<Vec<u8>>> {
    let session = current_session()?;
    let props = block_on_operation(session.TryGetMediaPropertiesAsync()?)?;
    load_thumbnail_bytes(&props)
}

struct App {
    now: NowPlaying,
    last_pull: Instant,
    err: Option<String>,
    timeline: Option<Timeline>,
    last_position_update: Instant,
    last_position_secs: f64,
    is_user_seeking: bool,
    pending_seek_target: Option<f64>,
    pending_seek_deadline: Option<Instant>,
    thumbnail_texture: Option<TextureHandle>,
    thumbnail_base_texture: Option<TextureHandle>,
    thumbnail_base_image: Option<ColorImage>,
    thumbnail_vinyl_image: Option<ColorImage>,
    thumbnail_hash: Option<u64>,
    pending_thumbnail: Option<PendingThumbnail>,
    thumbnail_rx: Option<mpsc::Receiver<ThumbnailMessage>>,
    thumbnail_err: Option<String>,
    thumbnail_inflight_request: Option<u64>,
    thumbnail_inflight_track: Option<NowPlaying>,
    next_thumbnail_request_id: u64,
    current_thumbnail_track: Option<NowPlaying>,
    snapshot_rx: Option<mpsc::Receiver<SnapshotResult>>,
    snapshot_request_tx: Option<mpsc::Sender<SnapshotCommand>>,
    snapshot_inflight: bool,
    last_snapshot_request: Option<Instant>,
    skin_manager: SkinManager,
    dynamic_root_gradient: Option<GradientSpec>,
    dynamic_panel_gradient: Option<GradientSpec>,
    skin_warnings: Vec<String>,
    skin_error: Option<String>,
    watch_skins: bool,
    settings_panel_open: bool,
    always_on_top: bool,
    last_window_level: Option<WindowLevel>,
    window_decorations_hidden: bool,
    last_window_decorations: Option<bool>,
    show_pin_button: bool,
    viewport_size: egui::Vec2,
    thumbnail_overlay_alpha: f32,
    config: Config,
    animations_enabled: bool,
    vinyl_spin: VinylSpin,
    vinyl_last_frame: Option<Instant>,
    vinyl_pending_refresh: bool,
    #[cfg(target_os = "windows")]
    titlebar_state: WindowsTitlebarState,
}

impl Default for App {
    fn default() -> Self {
        let mut config = Config::load().unwrap_or_default();
        let animations_enabled = animations_enabled_from_system();
        let vinyl_spin = VinylSpin::new();

        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let (request_tx, request_rx) = mpsc::channel();

        thread::spawn(move || {
            let com_initialized = unsafe {
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                if hr.is_ok() {
                    true
                } else if hr == RPC_E_CHANGED_MODE {
                    false
                } else {
                    let _ = snapshot_tx.send(Err(format!("COM init failed: {hr:?}")));
                    return;
                }
            };

            while let Ok(command) = request_rx.recv() {
                match command {
                    SnapshotCommand::Fetch => {
                        let res = fetch_session_snapshot().map_err(|e| format!("{e:?}"));
                        let _ = snapshot_tx.send(res);
                    }
                    SnapshotCommand::Shutdown => break,
                }
            }

            if com_initialized {
                unsafe {
                    CoUninitialize();
                }
            }
        });

        let skin_root = default_skin_root();
        let (skin_manager, skin_error) = match SkinManager::discover(&skin_root, None) {
            Ok(manager) => (manager, None),
            Err(err) => {
                let fallback = SkinManager::fallback().expect("default skin must load");
                (fallback, Some(format!("{err:?}")))
            }
        };
        let skin_warnings = skin_manager.warnings().to_vec();

        let mut vinyl_pending_refresh = false;
        let skin_disables_vinyl = skin_manager.current_theme().disable_vinyl_thumbnail;
        let vinyl_should_be_enabled = !skin_disables_vinyl;
        if config.ui.vinyl_thumbnail.enabled != vinyl_should_be_enabled {
            config.ui.vinyl_thumbnail.enabled = vinyl_should_be_enabled;
            vinyl_pending_refresh = true;
        }

        let mut app = Self {
            now: NowPlaying {
                state: PlayState::Unknown,
                ..Default::default()
            },
            last_pull: Instant::now() - Duration::from_secs(1),
            err: None,
            timeline: None,
            last_position_update: Instant::now(),
            last_position_secs: 0.0,
            is_user_seeking: false,
            pending_seek_target: None,
            pending_seek_deadline: None,
            thumbnail_texture: None,
            thumbnail_base_texture: None,
            thumbnail_base_image: None,
            thumbnail_vinyl_image: None,
            thumbnail_hash: None,
            pending_thumbnail: None,
            thumbnail_rx: None,
            thumbnail_err: None,
            thumbnail_inflight_request: None,
            thumbnail_inflight_track: None,
            next_thumbnail_request_id: 1,
            current_thumbnail_track: None,
            snapshot_rx: Some(snapshot_rx),
            snapshot_request_tx: Some(request_tx),
            snapshot_inflight: false,
            last_snapshot_request: None,
            skin_manager,
            dynamic_root_gradient: None,
            dynamic_panel_gradient: None,
            skin_warnings,
            skin_error,
            watch_skins: false,
            settings_panel_open: false,
            always_on_top: false,
            last_window_level: None,
            window_decorations_hidden: false,
            last_window_decorations: None,
            show_pin_button: true,
            viewport_size: egui::vec2(800.0, 600.0),
            thumbnail_overlay_alpha: 0.0,
            config,
            animations_enabled,
            vinyl_spin,
            vinyl_last_frame: None,
            vinyl_pending_refresh,
            #[cfg(target_os = "windows")]
            titlebar_state: WindowsTitlebarState::default(),
        };

        if let Some(tx) = app.snapshot_request_tx.as_ref() {
            if tx.send(SnapshotCommand::Fetch).is_ok() {
                app.snapshot_inflight = true;
                app.last_snapshot_request = Some(Instant::now());
            } else {
                app.snapshot_request_tx = None;
            }
        }

        app
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.skin_manager.apply_style(ctx);
        self.update_window_decorations(ctx, frame);
        #[cfg(target_os = "windows")]
        if !self.window_decorations_hidden {
            self.update_windows_titlebar(ctx, frame);
        }
        self.update_window_level(ctx);
        self.maintain_skin_watcher(ctx);

        let mut snapshots = Vec::new();
        if let Some(rx) = self.snapshot_rx.as_mut() {
            loop {
                match rx.try_recv() {
                    Ok(res) => snapshots.push(res),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.snapshot_rx = None;
                        self.snapshot_request_tx = None;
                        self.snapshot_inflight = false;
                        self.last_snapshot_request = None;
                        break;
                    }
                }
            }
        }

        for res in snapshots {
            self.snapshot_inflight = false;
            self.last_snapshot_request = None;
            match res {
                Ok((now, timeline)) => self.apply_snapshot(now, timeline),
                Err(e) => {
                    self.err = Some(e);
                    self.timeline = None;
                    self.last_pull = Instant::now();
                }
            }
        }

        self.maybe_refresh_vinyl_thumbnail();
        self.process_pending_thumbnail(ctx);

        if let Some(timeline) = &mut self.timeline {
            let is_playing = self.now.state == PlayState::Playing;
            if is_playing && self.pending_seek_target.is_none() {
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_position_update).as_secs_f64();
                let new_pos = (self.last_position_secs + elapsed)
                    .clamp(timeline.start_secs, timeline.end_secs);
                timeline.position_secs = new_pos;
                self.last_position_secs = new_pos;
                self.last_position_update = now;
            } else {
                self.last_position_update = Instant::now();
                self.last_position_secs = timeline.position_secs;
            }
        }

        let theme = self.skin_manager.current_theme();
        let theme_components = &theme.components;
        let use_dynamic_gradient = theme.use_gradient;
        let root_background = if use_dynamic_gradient {
            self.dynamic_root_gradient
                .as_ref()
                .map(|spec| AreaBackground::Gradient(spec.clone()))
                .unwrap_or_else(|| theme_components.root.background.clone())
        } else {
            theme_components.root.background.clone()
        };
        let panel_background = if use_dynamic_gradient {
            self.dynamic_panel_gradient
                .as_ref()
                .map(|spec| AreaBackground::Gradient(spec.clone()))
                .unwrap_or_else(|| theme_components.panel.background.clone())
        } else {
            theme_components.panel.background.clone()
        };

        let root_rect = ctx.screen_rect();
        self.viewport_size = root_rect.size();
        
        let transparent_bg = theme.transparent_background;
        
        if !transparent_bg {
            let root_painter = ctx.layer_painter(LayerId::background());
            paint_area_background(
                &root_painter,
                root_rect,
                CornerRadius::same(0),
                &root_background,
            );
        }

        let mut panel_frame = egui::Frame::central_panel(&ctx.style());
        panel_frame.fill = egui::Color32::TRANSPARENT;

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                let panel_rect = ui.max_rect();
                
                if !transparent_bg {
                    let panel_painter = ui.painter();
                    paint_area_background(
                        &panel_painter,
                        panel_rect,
                        CornerRadius::same(0),
                        &panel_background,
                    );
                }

                ui.spacing_mut().item_spacing.y = 12.0;

                self.render_skin_controls(ui, ctx);
                //ui.separator();
                self.render_now_playing(ui);
            });

        self.handle_borderless_window_interactions(ctx, root_rect);

        self.maybe_request_snapshot();
        ctx.request_repaint_after(self.desired_repaint_interval());
    }
}

impl App {
    fn align_from_layout(layout: &egui::Layout) -> egui::Align {
        use egui::Direction;

        match layout.main_dir() {
            Direction::LeftToRight | Direction::RightToLeft => layout.main_align,
            Direction::TopDown | Direction::BottomUp => layout.cross_align,
        }
    }

    fn desired_repaint_interval(&self) -> Duration {
        if self.animations_enabled && self.now.state == PlayState::Playing {
            Duration::from_millis(16)
        } else if matches!(self.now.state, PlayState::Changing | PlayState::Opened) {
            Duration::from_millis(120)
        } else if self.now.state == PlayState::Paused {
            Duration::from_millis(250)
        } else {
            Duration::from_millis(200)
        }
    }

    fn snapshot_poll_interval(&self) -> Duration {
        // Poll more aggressively while playback is active or changing, but
        // back off in idle states to avoid unnecessary COM traffic.
        match self.now.state {
            PlayState::Playing => Duration::from_millis(800),
            PlayState::Changing => Duration::from_millis(500),
            PlayState::Opened => Duration::from_secs(2),
            PlayState::Paused => Duration::from_secs(3),
            PlayState::Stopped => Duration::from_secs(4),
            PlayState::Closed | PlayState::Unknown => Duration::from_secs(5),
        }
    }

    fn maybe_request_snapshot(&mut self) {
        let now = Instant::now();

        if self.snapshot_inflight {
            if let Some(sent_at) = self.last_snapshot_request {
                if now.duration_since(sent_at) > Duration::from_secs(5) {
                    self.snapshot_inflight = false;
                    self.last_snapshot_request = None;
                }
            } else {
                self.snapshot_inflight = false;
            }
        }

        if self.snapshot_inflight {
            return;
        }

        if now.duration_since(self.last_pull) < self.snapshot_poll_interval() {
            return;
        }

        if let Some(tx) = self.snapshot_request_tx.as_ref() {
            match tx.send(SnapshotCommand::Fetch) {
                Ok(()) => {
                    self.snapshot_inflight = true;
                    self.last_snapshot_request = Some(now);
                }
                Err(_) => {
                    self.snapshot_request_tx = None;
                }
            }
        }
    }

    fn update_window_level(&mut self, ctx: &egui::Context) {
        let desired = if self.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };

        if self.last_window_level != Some(desired) {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(desired));
            self.last_window_level = Some(desired);
        }
    }

    fn update_window_decorations(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let desired = !self.window_decorations_hidden;
        if self.last_window_decorations != Some(desired) {
            ctx.send_viewport_cmd(ViewportCommand::Decorations(desired));
            self.last_window_decorations = Some(desired);
            #[cfg(target_os = "windows")]
            {
                if desired {
                    self.titlebar_state = WindowsTitlebarState::default();
                }
                self.apply_windows_corner_preference(frame);
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn update_windows_titlebar(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Ok(window_handle) = frame.window_handle() else {
            return;
        };

        let hwnd = match window_handle.as_raw() {
            RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut std::ffi::c_void),
            _ => return,
        };

        let style = ctx.style();
        let visuals = &style.visuals;
        let caption_color = visuals.window_fill;
        let caption_ref = color32_to_colorref(caption_color);
        let window_stroke = visuals.window_stroke;
        let has_window_border = window_stroke.width > f32::EPSILON;

        let dark_caption = is_dark_color(caption_color);
        let text_color = visuals.override_text_color.unwrap_or_else(|| {
            if dark_caption {
                egui::Color32::WHITE
            } else {
                egui::Color32::BLACK
            }
        });
        let text_ref = color32_to_colorref(text_color);
        let border_ref = if has_window_border {
            color32_to_colorref(window_stroke.color)
        } else {
            DWM_COLOR_UNSET
        };

        if self.titlebar_state.last_caption != Some(caption_ref) {
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_CAPTION_COLOR,
                    &caption_ref as *const u32 as *const _,
                    std::mem::size_of::<u32>() as u32,
                );
            }
            self.titlebar_state.last_caption = Some(caption_ref);
        }

        if self.titlebar_state.last_text != Some(text_ref) {
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_TEXT_COLOR,
                    &text_ref as *const u32 as *const _,
                    std::mem::size_of::<u32>() as u32,
                );
            }
            self.titlebar_state.last_text = Some(text_ref);
        }

        if self.titlebar_state.last_border != Some(border_ref) {
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &border_ref as *const u32 as *const _,
                    std::mem::size_of::<u32>() as u32,
                );
            }
            self.titlebar_state.last_border = Some(border_ref);
        }

        if self.titlebar_state.last_dark_mode != Some(dark_caption) {
            let dark_flag: i32 = dark_caption as i32;
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark_flag as *const i32 as *const _,
                    std::mem::size_of::<i32>() as u32,
                );
            }
            self.titlebar_state.last_dark_mode = Some(dark_caption);
        }
    }

    fn handle_borderless_window_interactions(
        &mut self,
        ctx: &egui::Context,
        root_rect: egui::Rect,
    ) {
        if !self.window_decorations_hidden {
            return;
        }

        let (pointer_pos, primary_pressed, primary_down) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.primary_down(),
            )
        });

        let Some(pos) = pointer_pos else {
            return;
        };

        let edge = 6.0;
        let drag_height = 36.0;

        if !primary_down {
            // Allow resizing when hovering near the border even if the pointer is just outside.
            if !root_rect.expand(edge).contains(pos) {
                return;
            }
        } else if !root_rect.expand(edge).contains(pos) {
            return;
        }

        let near_left = pos.x <= root_rect.left() + edge;
        let near_right = pos.x >= root_rect.right() - edge;
        let near_top = pos.y <= root_rect.top() + edge;
        let near_bottom = pos.y >= root_rect.bottom() - edge;

        let resize_dir = if near_left && near_top {
            Some(ResizeDirection::NorthWest)
        } else if near_right && near_top {
            Some(ResizeDirection::NorthEast)
        } else if near_left && near_bottom {
            Some(ResizeDirection::SouthWest)
        } else if near_right && near_bottom {
            Some(ResizeDirection::SouthEast)
        } else if near_left {
            Some(ResizeDirection::West)
        } else if near_right {
            Some(ResizeDirection::East)
        } else if near_top {
            Some(ResizeDirection::North)
        } else if near_bottom {
            Some(ResizeDirection::South)
        } else {
            None
        };

        if let Some(direction) = resize_dir {
            let cursor = match direction {
                ResizeDirection::North => egui::CursorIcon::ResizeNorth,
                ResizeDirection::South => egui::CursorIcon::ResizeSouth,
                ResizeDirection::East => egui::CursorIcon::ResizeEast,
                ResizeDirection::West => egui::CursorIcon::ResizeWest,
                ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
                ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
                ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
                ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
            };
            ctx.set_cursor_icon(cursor);
            if primary_pressed && !ctx.is_using_pointer() {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
            }
            return;
        }

        // Drag zone across the top excluding the overlay controls.
        let icon_size = ctx
            .style()
            .text_styles
            .get(&egui::TextStyle::Body)
            .map(|style| style.size)
            .unwrap_or(14.0);
        let icon_extent = icon_size + 8.0;
        let icon_spacing = 6.0;
        let icon_count = 1 + usize::from(self.show_pin_button);
        let overlay_width = if icon_count > 0 {
            icon_count as f32 * icon_extent + (icon_count.saturating_sub(1) as f32) * icon_spacing
        } else {
            0.0
        };
        let overlay_rect = egui::Rect::from_min_size(
            egui::pos2(root_rect.left() + 8.0, root_rect.top() + 8.0),
            egui::vec2(overlay_width, icon_extent),
        );

        let in_drag_strip = pos.y <= root_rect.top() + drag_height
            && !overlay_rect.contains(pos)
            && root_rect.contains(pos);

        if in_drag_strip {
            ctx.set_cursor_icon(egui::CursorIcon::Move);
            if primary_pressed && !ctx.is_using_pointer() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
        }
    }

    fn thumbnail_overlay_geometry(
        &self,
        rect: egui::Rect,
        icon_count: usize,
    ) -> Option<ThumbnailOverlayGeometry> {
        if icon_count == 0 {
            return None;
        }

        let icon_count_f = icon_count as f32;
        let available_width = (rect.width() - 20.0).max(60.0);
        let icon_slot = (available_width / icon_count_f).clamp(18.0, 44.0);
        let icon_spacing = (icon_slot * 0.2).clamp(4.0, 12.0);
        let overlay_width = icon_slot * icon_count_f + icon_spacing * (icon_count_f - 1.0);
        let overlay_height = icon_slot + 6.0;

        let mut center_y = rect.max.y - overlay_height * 0.5 - 8.0;
        let min_y = rect.min.y + overlay_height * 0.5 + 6.0;
        if center_y < min_y {
            center_y = rect.center().y;
        }

        let mut overlay_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, center_y),
            egui::vec2(overlay_width, overlay_height),
        );

        if overlay_rect.max.y > rect.max.y - 4.0 {
            let shift = overlay_rect.max.y - (rect.max.y - 4.0);
            overlay_rect = overlay_rect.translate(egui::vec2(0.0, -shift));
        }
        if overlay_rect.min.y < rect.min.y + 4.0 {
            let shift = (rect.min.y + 4.0) - overlay_rect.min.y;
            overlay_rect = overlay_rect.translate(egui::vec2(0.0, shift));
        }

        Some(ThumbnailOverlayGeometry {
            rect: overlay_rect,
            icon_slot,
            icon_spacing,
            height: overlay_height,
        })
    }

    fn adjust_thumbnail_overlay_alpha(&mut self, target: f32, ctx: &egui::Context) -> f32 {
        let target = target.clamp(0.0, 1.0);
        let new_alpha = egui::lerp(self.thumbnail_overlay_alpha..=target, 0.2);
        if (new_alpha - target).abs() > 0.01 {
            ctx.request_repaint();
        }
        self.thumbnail_overlay_alpha = new_alpha;
        new_alpha
    }

    fn draw_thumbnail_overlay(
        &mut self,
        ui: &mut egui::Ui,
        geometry: ThumbnailOverlayGeometry,
        alpha: f32,
    ) {
        let visuals = ui.visuals().clone();
        
        // Show play or pause based on current state
        let play_pause_action = if self.now.state == PlayState::Playing {
            ThumbnailOverlayAction::Pause
        } else {
            ThumbnailOverlayAction::Play
        };
        let play_pause_icon = if self.now.state == PlayState::Playing {
            "⏸"
        } else {
            "⏵"
        };
        
        let icons = [
            (ThumbnailOverlayAction::Previous, "⏮"),
            (play_pause_action, play_pause_icon),
            (ThumbnailOverlayAction::Next, "⏭"),
        ];

        let background_alpha = (alpha * 110.0).round() as u8;
        if background_alpha > 0 {
            let bg_color = egui::Color32::from_rgba_unmultiplied(15, 23, 42, background_alpha);
            let rounding = CornerRadius::same((geometry.height / 2.0).round() as u8);
            ui.painter_at(geometry.rect)
                .rect_filled(geometry.rect, rounding, bg_color);
        }

        let overlay_id = ui.id().with("thumbnail.overlay");
        let mut overlay_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(geometry.rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt(overlay_id),
        );
        overlay_ui.spacing_mut().item_spacing.x = geometry.icon_spacing;
        overlay_ui.set_min_height(geometry.height);

        for (action, symbol) in icons {
            let (icon_rect, icon_response) = overlay_ui.allocate_exact_size(
                egui::vec2(geometry.icon_slot, geometry.height),
                egui::Sense::click(),
            );

            let mut icon_color = visuals.widgets.inactive.fg_stroke.color;

            if icon_response.hovered() {
                overlay_ui
                    .ctx()
                    .set_cursor_icon(egui::CursorIcon::PointingHand);
                icon_color = visuals.hyperlink_color;
            }

            let icon_color = icon_color.gamma_multiply(alpha);
            overlay_ui.painter().text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                symbol,
                FontId::proportional(geometry.icon_slot * 0.65),
                icon_color,
            );

            if icon_response.clicked() {
                self.handle_thumbnail_overlay_action(action);
            }
        }
    }

    fn handle_thumbnail_overlay_action(&mut self, action: ThumbnailOverlayAction) {
        match action {
            ThumbnailOverlayAction::Previous => {
                self.playback_command("Previous", |session| {
                    block_on_operation(session.TrySkipPreviousAsync()?)
                });
            }
            ThumbnailOverlayAction::Next => {
                self.playback_command("Next", |session| {
                    block_on_operation(session.TrySkipNextAsync()?)
                });
            }
            ThumbnailOverlayAction::Play => {
                self.playback_command("Play", |session| {
                    block_on_operation(session.TryPlayAsync()?)
                });
            }
            ThumbnailOverlayAction::Pause => {
                self.playback_command("Pause", |session| {
                    block_on_operation(session.TryPauseAsync()?)
                });
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn apply_windows_corner_preference(&self, frame: &eframe::Frame) {
        let Ok(window_handle) = frame.window_handle() else {
            return;
        };
        let hwnd = match window_handle.as_raw() {
            RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut std::ffi::c_void),
            _ => return,
        };

        let preference = if self.window_decorations_hidden {
            DWMWCP_ROUND
        } else {
            DWMWCP_DEFAULT
        };

        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const _,
                std::mem::size_of_val(&preference) as u32,
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn apply_windows_corner_preference(&self, _frame: &eframe::Frame) {}

    #[allow(dead_code)]
    fn is_mobile_stack_layout(&self) -> bool {
        let variant = self.skin_manager.current_layout_variant();
        let id_lower = variant.id.to_ascii_lowercase();
        let name_lower = variant.display_name.to_ascii_lowercase();
        if id_lower.contains("mobile") || name_lower.contains("mobile") {
            return true;
        }

        fn looks_like_mobile_column(node: &LayoutNode) -> bool {
            match node {
                LayoutNode::Column(container) => {
                    let mut found_thumbnail = false;
                    let mut found_playback = false;
                    let mut found_timeline = false;
                    let mut component_count = 0;

                    for child in &container.children {
                        if let LayoutNode::Component(component) = child {
                            component_count += 1;
                            match component.component {
                                LayoutComponent::Thumbnail => found_thumbnail = true,
                                LayoutComponent::PlaybackControlsGroup => found_playback = true,
                                LayoutComponent::Timeline => found_timeline = true,
                                _ => {}
                            }
                        }
                    }

                    container.fill
                        && matches!(container.align, LayoutAlign::Center)
                        && component_count >= 3
                        && found_thumbnail
                        && found_playback
                        && found_timeline
                }
                _ => false,
            }
        }

        looks_like_mobile_column(&variant.root)
    }

    fn maintain_skin_watcher(&mut self, ctx: &egui::Context) {
        if self.watch_skins {
            if !self.skin_manager.hot_reload_enabled() {
                match self.skin_manager.enable_hot_reload() {
                    Ok(()) => {
                        self.skin_error = None;
                    }
                    Err(err) => {
                        self.skin_error = Some(err.to_string());
                        self.watch_skins = false;
                    }
                }
            }
        } else if self.skin_manager.hot_reload_enabled() {
            self.skin_manager.disable_hot_reload();
        }

        if self.skin_manager.hot_reload_enabled() && self.skin_manager.poll_hot_reload(ctx) {
            self.skin_warnings = self.skin_manager.warnings().to_vec();
        }
    }

    fn reload_skins(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let selected = self.skin_manager.current_skin_id().map(|s| s.to_string());
        let root = default_skin_root();
        let mut manager =
            SkinManager::discover(&root, selected.as_deref()).map_err(|err| format!("{err:?}"))?;
        if self.watch_skins {
            if let Err(err) = manager.enable_hot_reload() {
                self.watch_skins = false;
                return Err(err.to_string());
            }
        }
        manager.apply_style(ctx);
        self.skin_warnings = manager.warnings().to_vec();
        self.skin_manager = manager;
        self.clear_dynamic_gradients();
        Ok(())
    }

    fn render_skin_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let skins: Vec<(String, String)> = self
            .skin_manager
            .skin_list()
            .iter()
            .map(|info| (info.id.clone(), info.display_name.clone()))
            .collect();
        let current_skin_display = self.skin_manager.current_skin_display_name().to_string();
        let current_skin_id = self.skin_manager.current_skin_id().map(|id| id.to_string());
        let layout_options = self.skin_manager.layout_options().to_vec();
        let current_layout_display = self.skin_manager.current_layout_display_name().to_string();
        let current_layout_id = self.skin_manager.current_layout_id().to_string();

        let mut requested_skin: Option<String> = None;
        let mut requested_layout: Option<String> = None;

        const SETTINGS_PANEL_MAX_WIDTH: f32 = 360.0;
        const SETTINGS_PANEL_ITEM_SPACING: f32 = 18.0;
        const SETTINGS_CONTROL_SPACING: f32 = 12.0;
        const SETTINGS_SECTION_GAP: f32 = 24.0;
        const SETTINGS_HEADER_GAP: f32 = 8.0;
        const SETTINGS_PANEL_PADDING_X: i8 = 20;
        const SETTINGS_PANEL_PADDING_Y: i8 = 18;
        const SETTINGS_PANEL_CORNER_RADIUS: u8 = 14;

        fn settings_section<R>(
            ui: &mut egui::Ui,
            visuals: &egui::Visuals,
            title: &str,
            header_gap: f32,
            control_spacing: f32,
            content_width: f32,
            build: impl FnOnce(&mut egui::Ui) -> R,
        ) -> R {
            ui.label(
                egui::RichText::new(title)
                    .size(13.0)
                    .color(visuals.strong_text_color()),
            );
            ui.add_space(header_gap);
            ui.vertical(|section| {
                section.set_min_width(content_width);
                section.set_max_width(content_width);
                section.spacing_mut().item_spacing = egui::vec2(0.0, control_spacing);
                build(section)
            })
            .inner
        }

        fn settings_separator(ui: &mut egui::Ui, gap: f32) {
            ui.add_space(gap * 0.5);
            ui.separator();
            ui.add_space(gap * 0.5);
        }

        egui::Area::new(egui::Id::new("overlay-controls"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 8.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .movable(false)
            .show(ui.ctx(), |overlay| {
                overlay.spacing_mut().item_spacing.x = 6.0;
                overlay.horizontal(|row| {
                    row.spacing_mut().item_spacing.x = 6.0;

                    let overlay_icon_button =
                        |ui: &mut egui::Ui, icon: &str, tooltip: &str, active: bool| {
                            let icon_size = ui
                                .style()
                                .text_styles
                                .get(&egui::TextStyle::Body)
                                .map(|style| style.size)
                                .unwrap_or(14.0);
                            let desired_size = egui::Vec2::splat(icon_size + 8.0);
                            let (rect, response) =
                                ui.allocate_exact_size(desired_size, egui::Sense::click());

                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            let visuals = ui.visuals();
                            let fg_color = if active {
                                visuals.widgets.active.fg_stroke.color
                            } else {
                                visuals.widgets.inactive.fg_stroke.color
                            };

                            ui.painter_at(rect).text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icon,
                                egui::FontId::proportional(icon_size),
                                fg_color,
                            );

                            response.on_hover_text(tooltip)
                        };

                    if self.show_pin_button {
                        let pin_icon = if self.always_on_top { "📌" } else { "📍" };
                        let pin_tooltip = if self.always_on_top {
                            "Unpin window"
                        } else {
                            "Pin window (stay on top)"
                        };
                        if overlay_icon_button(row, pin_icon, pin_tooltip, self.always_on_top)
                            .clicked()
                        {
                            self.always_on_top = !self.always_on_top;
                        }
                    }

                    let gear_tooltip = if self.settings_panel_open {
                        "Hide settings"
                    } else {
                        "Show settings"
                    };
                    if overlay_icon_button(row, "⚙", gear_tooltip, self.settings_panel_open)
                        .clicked()
                    {
                        self.settings_panel_open = !self.settings_panel_open;
                    }
                });
            });

        if self.settings_panel_open {
            let visuals = ctx.style().visuals.clone();
            
            let mut window_frame = egui::Frame::window(&ctx.style());
            window_frame.inner_margin = egui::Margin {
                left: SETTINGS_PANEL_PADDING_X,
                right: SETTINGS_PANEL_PADDING_X,
                top: SETTINGS_PANEL_PADDING_Y,
                bottom: SETTINGS_PANEL_PADDING_Y,
            };
            window_frame.corner_radius = CornerRadius::same(SETTINGS_PANEL_CORNER_RADIUS);
            window_frame.shadow = egui::Shadow {
                offset: [0, 6],
                blur: 28,
                spread: 4,
                color: if visuals.dark_mode {
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120)
                } else {
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 72)
                },
            };
            window_frame.fill = if visuals.dark_mode {
                egui::Color32::from_rgba_unmultiplied(28, 28, 32, 240)
            } else {
                egui::Color32::from_rgba_unmultiplied(244, 246, 249, 245)
            };

            egui::Window::new("Settings")
                .id(egui::Id::new("settings-window"))
                .collapsible(false)
                .resizable(false)
                .title_bar(false)
                .frame(window_frame)
                .fixed_size([SETTINGS_PANEL_MAX_WIDTH, 0.0])
                .show(ctx, |panel| {
                    let content_width = SETTINGS_PANEL_MAX_WIDTH - 2.0 * f32::from(SETTINGS_PANEL_PADDING_X);
                    panel.set_min_width(SETTINGS_PANEL_MAX_WIDTH);
                    panel.set_max_width(SETTINGS_PANEL_MAX_WIDTH);
                    panel.spacing_mut().item_spacing = egui::vec2(0.0, SETTINGS_PANEL_ITEM_SPACING);

                        panel.horizontal(|row| {
                            row.spacing_mut().item_spacing.x = 12.0;
                            row.label(egui::RichText::new("Settings").heading());

                            row.allocate_ui_with_layout(
                                egui::vec2(row.available_width(), 0.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |actions| {
                                    let close_icon = egui::RichText::new("×").size(18.0);
                                    let close = actions
                                        .add(
                                            egui::Label::new(close_icon)
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Close settings");
                                    if close.hovered() {
                                        actions.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if close.clicked() {
                                        self.settings_panel_open = false;
                                    }
                                },
                            );
                        });

                        panel.separator();

                        egui::ScrollArea::vertical()
                            .max_height(420.0)
                            .show(panel, |scroll| {
                                scroll.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                scroll.set_min_width(content_width);
                                scroll.set_max_width(content_width);

                                settings_section(
                                    scroll,
                                    &visuals,
                                    "Window",
                                    SETTINGS_HEADER_GAP,
                                    SETTINGS_CONTROL_SPACING,
                                    content_width,
                                    |section| {
                                        let toggle_label = if self.window_decorations_hidden {
                                            "Show window title bar"
                                        } else {
                                            "Hide window title bar"
                                        };
                                        if self
                                            .skin_manager
                                            .skin_button(section, toggle_label)
                                            .clicked()
                                        {
                                            self.window_decorations_hidden =
                                                !self.window_decorations_hidden;
                                        }

                                        let pin_toggle_label = if self.always_on_top {
                                            "Disable stay-on-top"
                                        } else {
                                            "Pin window (stay on top)"
                                        };
                                        if self
                                            .skin_manager
                                            .skin_button(section, pin_toggle_label)
                                            .on_hover_text(
                                                "Keep the widget above other application windows.",
                                            )
                                            .clicked()
                                        {
                                            self.always_on_top = !self.always_on_top;
                                        }

                                        let mut show_pin_button = self.show_pin_button;
                                        if section
                                            .checkbox(
                                                &mut show_pin_button,
                                                "Show pin button in overlay",
                                            )
                                            .on_hover_text(
                                                "Disable to hide the pin toggle from the top overlay.",
                                            )
                                            .changed()
                                        {
                                            self.show_pin_button = show_pin_button;
                                        }

                                        section.label(
                                            if self.window_decorations_hidden {
                                                "Title bar hidden. Use the app body to drag the window."
                                            } else {
                                                "Hiding the title bar removes the OS chrome."
                                            },
                                        );
                                    },
                                );

                                settings_separator(scroll, SETTINGS_SECTION_GAP);

                                settings_section(
                                    scroll,
                                    &visuals,
                                    "Appearance",
                                    SETTINGS_HEADER_GAP,
                                    SETTINGS_CONTROL_SPACING,
                                    content_width,
                                    |section| {
                                        let combo_width = content_width;
                                        egui::ComboBox::from_id_salt("skin-select")
                                            .width(combo_width)
                                            .selected_text(current_skin_display.clone())
                                            .show_ui(section, |combo| {
                                                if skins.is_empty() {
                                                    combo.label("Embedded default");
                                                } else {
                                                    for (id, name) in &skins {
                                                        let selected = current_skin_id
                                                            .as_deref()
                                                            .map(|current| current == id.as_str())
                                                            .unwrap_or(false);
                                                        if combo
                                                            .selectable_label(selected, name)
                                                            .clicked()
                                                            && !selected
                                                        {
                                                            requested_skin = Some(id.clone());
                                                        }
                                                    }
                                                }
                                            });

                                        if layout_options.len() > 1 {
                                            egui::ComboBox::from_id_salt("layout-select")
                                                .width(combo_width)
                                                .selected_text(current_layout_display.clone())
                                                .show_ui(section, |combo| {
                                                    for option in &layout_options {
                                                        let selected = option.id == current_layout_id;
                                                        if combo
                                                            .selectable_label(
                                                                selected,
                                                                &option.display_name,
                                                            )
                                                            .clicked()
                                                            && !selected
                                                        {
                                                            requested_layout = Some(option.id.clone());
                                                        }
                                                    }
                                                });
                                        } else if let Some(option) = layout_options.first() {
                                            section.label(
                                                format!("Layout: {}", option.display_name),
                                            );
                                        }
                                    },
                                );

                                settings_separator(scroll, SETTINGS_SECTION_GAP);

                                settings_section(
                                    scroll,
                                    &visuals,
                                    "Artwork",
                                    SETTINGS_HEADER_GAP,
                                    SETTINGS_CONTROL_SPACING,
                                    content_width,
                                    |section| {
                                        let theme_disables_vinyl = self
                                            .skin_manager
                                            .current_theme()
                                            .disable_vinyl_thumbnail;
                                        if theme_disables_vinyl {
                                            section.label(
                                                "This skin always shows the original album art.",
                                            );
                                        } else {
                                            let mut vinyl_enabled =
                                                self.config.ui.vinyl_thumbnail.enabled;
                                            if section
                                                .checkbox(&mut vinyl_enabled, "Show spinning vinyl disc")
                                                .on_hover_text(
                                                    "Toggle between the animated vinyl and the original thumbnail.",
                                                )
                                                .changed()
                                            {
                                                self.set_vinyl_enabled(ctx, vinyl_enabled);
                                            }
                                            section.label(
                                                "Tip: You can also click the artwork to switch views.",
                                            );
                                        }
                                    },
                                );

                                settings_separator(scroll, SETTINGS_SECTION_GAP);

                                settings_section(
                                    scroll,
                                    &visuals,
                                    "Skins",
                                    SETTINGS_HEADER_GAP,
                                    SETTINGS_CONTROL_SPACING,
                                    content_width,
                                    |section| {
                                        section.horizontal_wrapped(|row| {
                                            row.spacing_mut().item_spacing =
                                                egui::vec2(12.0, SETTINGS_CONTROL_SPACING);
                                            let toggle_label = if self.watch_skins {
                                                "Disable hot reload"
                                            } else {
                                                "Enable hot reload"
                                            };
                                            if self.skin_manager.skin_button(row, toggle_label).clicked() {
                                                self.watch_skins = !self.watch_skins;
                                            }

                                            if self
                                                .skin_manager
                                                .skin_button(row, "Reload skins")
                                                .on_hover_text("Re-scan the skin directory")
                                                .clicked()
                                            {
                                                match self.reload_skins(ctx) {
                                                    Ok(()) => self.skin_error = None,
                                                    Err(err) => self.skin_error = Some(err),
                                                }
                                            }
                                        });
                                    },
                                );
                            });
                });
        }

        if let Some(id) = requested_skin {
            match self.skin_manager.set_skin(&id, ctx) {
                Ok(()) => {
                    self.skin_warnings = self.skin_manager.warnings().to_vec();
                    self.skin_error = None;
                    self.clear_dynamic_gradients();
                    let skin_disables_vinyl =
                        self.skin_manager.current_theme().disable_vinyl_thumbnail;
                    let vinyl_should_be_enabled = !skin_disables_vinyl;
                    if self.config.ui.vinyl_thumbnail.enabled != vinyl_should_be_enabled {
                        self.set_vinyl_enabled(ctx, vinyl_should_be_enabled);
                        self.force_thumbnail_refresh();
                    }
                }
                Err(err) => {
                    self.skin_error = Some(err.to_string());
                }
            }
        }

        if let Some(layout_id) = requested_layout {
            self.skin_manager.set_layout(&layout_id, ctx);
        }
    }

    fn render_now_playing(&mut self, ui: &mut egui::Ui) {
        let layout_root = self.skin_manager.current_layout_variant().root.clone();
        self.render_layout_node(ui, &layout_root);
    }

    fn render_layout_node(&mut self, ui: &mut egui::Ui, node: &LayoutNode) {
        match node {
            LayoutNode::Row(container) => self.render_container(ui, container, true),
            LayoutNode::Column(container) => self.render_container(ui, container, false),
            LayoutNode::Component(component) => self.render_component_node(ui, component),
            LayoutNode::Spacer(spacer) => {
                if spacer.size > f32::EPSILON {
                    ui.add_space(spacer.size);
                }
            }
        }
    }

    fn render_container(&mut self, ui: &mut egui::Ui, container: &ContainerNode, is_row: bool) {
        if container.children.is_empty() {
            return;
        }

        let align = match container.align {
            LayoutAlign::Start => egui::Align::Min,
            LayoutAlign::Center => egui::Align::Center,
            LayoutAlign::End => egui::Align::Max,
        };

        let layout = if is_row {
            egui::Layout::left_to_right(align)
        } else {
            egui::Layout::top_down(align)
        };

        if container.fill {
            let width = ui.available_width();
            ui.allocate_ui_with_layout(egui::Vec2::new(width, 0.0), layout, |child_ui| {
                self.render_container_children(child_ui, &container.children, container.spacing);
            });
        } else {
            ui.with_layout(layout, |child_ui| {
                self.render_container_children(child_ui, &container.children, container.spacing);
            });
        }
    }

    fn render_container_children(
        &mut self,
        ui: &mut egui::Ui,
        children: &[LayoutNode],
        spacing: f32,
    ) {
        let mut first = true;
        for child in children {
            if !first {
                ui.add_space(spacing);
            }
            first = false;
            self.render_layout_node(ui, child);
        }
    }

    fn render_component_node(&mut self, ui: &mut egui::Ui, component: &ComponentNode) {
        if !component.visible {
            return;
        }

        match component.component {
            LayoutComponent::Thumbnail => self.paint_thumbnail(ui),
            LayoutComponent::Title => {
                self.skin_manager.skin_text(ui, &self.now.title, true);
            }
            LayoutComponent::MetadataGroup => self.render_metadata_group(ui, component),
            LayoutComponent::MetadataArtist => self.render_metadata_artist(ui),
            LayoutComponent::MetadataAlbum => self.render_metadata_album(ui),
            LayoutComponent::MetadataState => {
                if Self::component_param_bool(component, "show_state")
                    .or_else(|| Self::component_param_bool(component, "state"))
                    .unwrap_or(true)
                {
                    let show_label = Self::component_param_bool(component, "show_state_label")
                        .or_else(|| Self::component_param_bool(component, "state_label"))
                        .unwrap_or(true);
                    self.render_metadata_state(ui, show_label);
                }
            }
            LayoutComponent::PlaybackControlsGroup => {
                let centered = Self::component_param_bool(component, "centered").unwrap_or(false);
                self.render_playback_controls_group(ui, centered);
            }
            LayoutComponent::PlaybackButtonPrevious => {
                self.render_playback_button(ui, PlaybackButtonKind::Previous, 1.0);
            }
            LayoutComponent::PlaybackButtonPlayPause => {
                self.render_playback_button(ui, PlaybackButtonKind::PlayPause, 1.0);
            }
            LayoutComponent::PlaybackButtonNext => {
                self.render_playback_button(ui, PlaybackButtonKind::Next, 1.0);
            }
            LayoutComponent::PlaybackButtonStop => {
                // Stop button retired; keep layout compatibility with no output.
            }
            LayoutComponent::Timeline => {
                let centered = Self::component_param_bool(component, "centered").unwrap_or(false);
                let show_separator =
                    Self::component_param_bool(component, "separator").unwrap_or(true);
                self.render_timeline_component(ui, centered, show_separator);
            }
            LayoutComponent::SkinWarnings => self.render_skin_warnings(ui),
            LayoutComponent::SkinError => self.render_skin_error(ui),
            LayoutComponent::NowPlayingError => self.render_now_playing_error(ui),
            LayoutComponent::ThumbnailError => self.render_thumbnail_error(ui),
        }
    }

    fn component_param_bool(component: &ComponentNode, key: &str) -> Option<bool> {
        component.params.get(key).and_then(|value| {
            match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }
        })
    }

    fn paint_thumbnail(&mut self, ui: &mut egui::Ui) {
        let (thumbnail_style, panel_style, theme_disables_vinyl) = {
            let theme = self.skin_manager.current_theme();
            (
                theme.components.thumbnail.clone(),
                theme.components.panel.clone(),
                theme.disable_vinyl_thumbnail,
            )
        };
        let panel_fg = panel_style.foreground;
        let corner_radius = thumbnail_style.corner_radius.max(0.0);
        let rounding = CornerRadius::same(corner_radius.clamp(0.0, u8::MAX as f32).round() as u8);
        let overlay_textures = self.skin_manager.thumbnail_overlay_textures(ui.ctx());
        let stroke_width = thumbnail_style.stroke_width.max(0.0);
        let stroke_color = thumbnail_style.stroke_color;

        let vinyl_active = self.config.ui.vinyl_thumbnail.enabled && !theme_disables_vinyl;
        let primary_texture = if vinyl_active {
            self.thumbnail_texture.as_ref()
        } else {
            self.thumbnail_base_texture
                .as_ref()
                .or(self.thumbnail_texture.as_ref())
        };

        let sense = if theme_disables_vinyl {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        };

        let viewport_min_side = self.viewport_size.x.min(self.viewport_size.y);

        if let Some(texture) = primary_texture {
            let mut size = texture.size_vec2();
            if size.x > 0.0 && size.y > 0.0 {
                let width_limit = ui.available_width().max(140.0);
                let view_limit = (viewport_min_side * 0.58).max(140.0);
                let max_side = width_limit.min(view_limit).min(220.0);
                let scale = (max_side / size.x).min(max_side / size.y).min(1.0);
                size *= scale;
            } else {
                let width_limit = ui.available_width().max(140.0);
                let view_limit = (viewport_min_side * 0.58).max(140.0);
                let max_side = width_limit.min(view_limit).min(220.0);
                size = egui::vec2(max_side, max_side);
            }

            let (rect, sense_response) = ui.allocate_exact_size(size, sense);

            if stroke_width > 0.0 && stroke_color.a() > 0 {
                let border_rect = rect.expand(stroke_width);
                let border_rounding = CornerRadius::same(
                    (corner_radius + stroke_width)
                        .clamp(0.0, u8::MAX as f32)
                        .round() as u8,
                );
                ui.painter_at(border_rect)
                    .rect_filled(border_rect, border_rounding, stroke_color);
            }

            let mut response = sense_response;
            if vinyl_active {
                let now = Instant::now();
                let dt = self
                    .vinyl_last_frame
                    .map(|last| (now - last).as_secs_f32())
                    .unwrap_or(0.0)
                    .min(0.25);
                self.vinyl_last_frame = Some(now);

                let should_spin = self.animations_enabled && self.now.state == PlayState::Playing;
                self.vinyl_spin.advance(dt, should_spin);
                if should_spin {
                    ui.ctx().request_repaint();
                }

                self.paint_vinyl_disc(ui, rect, size, texture, self.vinyl_spin.angle());
            } else {
                self.vinyl_last_frame = None;
                let image_widget = egui::Image::new((texture.id(), size))
                    .fit_to_exact_size(size)
                    .corner_radius(rounding);
                let image_response = ui.put(rect, image_widget);
                response = response.union(image_response);
            }

            if !theme_disables_vinyl {
                let tooltip = if vinyl_active {
                    "Click to show the original album artwork"
                } else {
                    "Click to switch to the spinning vinyl"
                };
                if response.clicked() {
                    self.set_vinyl_enabled(ui.ctx(), !vinyl_active);
                }
                response = response.on_hover_text(tooltip);
            } else {
                response =
                    response.on_hover_text("Current skin disables the spinning vinyl overlay.");
            }

            let overlay_enabled =
                size.x <= 200.0 || size.y <= 200.0 || ui.available_width() < 360.0;
            let overlay_geometry = if overlay_enabled {
                self.thumbnail_overlay_geometry(rect, 3)
            } else {
                None
            };

            let overlay_hovered = overlay_geometry
                .as_ref()
                .and_then(|geom| ui.ctx().pointer_latest_pos().map(|pos| geom.rect.contains(pos)))
                .unwrap_or(false);

            let alpha = self.adjust_thumbnail_overlay_alpha(
                if overlay_enabled && (response.hovered() || overlay_hovered) {
                    1.0
                } else {
                    0.0
                },
                ui.ctx(),
            );

            if alpha > 0.01 {
                if let Some(geometry) = overlay_geometry {
                    self.draw_thumbnail_overlay(ui, geometry, alpha);
                }
            }

            for (overlay, offset) in &overlay_textures {
                let tex_size = overlay.size_vec2();
                if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
                    continue;
                }

                let scale = (size.x / tex_size.x)
                    .min(size.y / tex_size.y)
                    .min(1.0)
                    .max(0.0);
                let overlay_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);
                if overlay_size.x <= 0.0 || overlay_size.y <= 0.0 {
                    continue;
                }
                let center = response.rect.center() + *offset;
                let overlay_rect = egui::Rect::from_center_size(center, overlay_size);
                let overlay_widget = egui::Image::new((overlay.id(), overlay_size))
                    .fit_to_exact_size(overlay_size)
                    .corner_radius(rounding);
                ui.put(overlay_rect, overlay_widget);
            }
        } else {
            let width_limit = ui.available_width().max(96.0);
            let view_limit = (viewport_min_side * 0.55).max(96.0);
            let max_side = width_limit.min(view_limit).min(220.0);
            let size = egui::vec2(max_side, max_side);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());

            if stroke_width > 0.0 && stroke_color.a() > 0 {
                let border_rect = rect.expand(stroke_width);
                let border_rounding = CornerRadius::same(
                    (corner_radius + stroke_width)
                        .clamp(0.0, u8::MAX as f32)
                        .round() as u8,
                );
                ui.painter_at(border_rect)
                    .rect_filled(border_rect, border_rounding, stroke_color);
            }

            let painter = ui.painter_at(rect);
            paint_area_background(&painter, rect, rounding, &panel_style.background);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No artwork",
                egui::TextStyle::Body.resolve(ui.style()),
                panel_fg,
            );

            for (overlay, offset) in &overlay_textures {
                let tex_size = overlay.size_vec2();
                if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
                    continue;
                }

                let scale = (size.x / tex_size.x)
                    .min(size.y / tex_size.y)
                    .min(1.0)
                    .max(0.0);
                let overlay_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);
                if overlay_size.x <= 0.0 || overlay_size.y <= 0.0 {
                    continue;
                }
                let center = rect.center() + *offset;
                let overlay_rect = egui::Rect::from_center_size(center, overlay_size);
                let overlay_widget = egui::Image::new((overlay.id(), overlay_size))
                    .fit_to_exact_size(overlay_size)
                    .corner_radius(rounding);
                ui.put(overlay_rect, overlay_widget);
            }

            self.adjust_thumbnail_overlay_alpha(0.0, ui.ctx());
        }
    }

    fn set_vinyl_enabled(&mut self, ctx: &egui::Context, enabled: bool) {
        let theme_disables_vinyl = self.skin_manager.current_theme().disable_vinyl_thumbnail;
        let final_enabled = enabled && !theme_disables_vinyl;

        if self.config.ui.vinyl_thumbnail.enabled == final_enabled {
            return;
        }

        self.config.ui.vinyl_thumbnail.enabled = final_enabled;

        if final_enabled {
            if let Some(vinyl_image) = self.thumbnail_vinyl_image.clone() {
                let texture = ctx.load_texture(
                    "now_playing.thumbnail",
                    vinyl_image.clone(),
                    TextureOptions::LINEAR,
                );
                self.thumbnail_texture = Some(texture);
                self.vinyl_spin.reset();
                self.vinyl_last_frame = None;
                self.vinyl_pending_refresh = false;
            } else if let Some(base_image) = self.thumbnail_base_image.clone() {
                let options = VinylThumbnailOptions::from_config(
                    &self.config.ui.vinyl_thumbnail,
                    base_image.size[0],
                    base_image.size[1],
                );
                let vinyl_image = render_vinyl(&base_image, &options);
                let texture = ctx.load_texture(
                    "now_playing.thumbnail",
                    vinyl_image.clone(),
                    TextureOptions::LINEAR,
                );
                self.thumbnail_vinyl_image = Some(vinyl_image);
                self.thumbnail_texture = Some(texture);
                self.vinyl_spin.reset();
                self.vinyl_last_frame = None;
                self.vinyl_pending_refresh = false;
            } else if let Some(track) = self.current_thumbnail_track.clone() {
                self.thumbnail_inflight_track = None;
                self.thumbnail_inflight_request = None;
                self.request_thumbnail_for(track);
                self.vinyl_pending_refresh = true;
            } else {
                self.vinyl_pending_refresh = true;
            }
        } else {
            self.vinyl_spin.reset();
            self.vinyl_last_frame = None;
            self.vinyl_pending_refresh = false;
            if let Some(base_texture) = self.thumbnail_base_texture.clone() {
                self.thumbnail_texture = Some(base_texture);
            }
        }

        ctx.request_repaint();
    }

    fn paint_vinyl_disc(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        size: egui::Vec2,
        texture: &TextureHandle,
        angle: f32,
    ) {
        let half = size * 0.5;
        let center = rect.center();
        let cos_r = angle.cos();
        let sin_r = angle.sin();

        let offsets = [
            egui::Vec2::new(-half.x, -half.y),
            egui::Vec2::new(half.x, -half.y),
            egui::Vec2::new(half.x, half.y),
            egui::Vec2::new(-half.x, half.y),
        ];
        let uvs = [
            egui::Pos2::new(0.0, 0.0),
            egui::Pos2::new(1.0, 0.0),
            egui::Pos2::new(1.0, 1.0),
            egui::Pos2::new(0.0, 1.0),
        ];

        let mut mesh = egui::Mesh::with_texture(texture.id());
        for (offset, uv) in offsets.into_iter().zip(uvs) {
            let rotated = egui::Vec2::new(
                offset.x * cos_r - offset.y * sin_r,
                offset.x * sin_r + offset.y * cos_r,
            );
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::Pos2::new(center.x + rotated.x, center.y + rotated.y),
                uv,
                color: egui::Color32::WHITE,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        ui.painter_at(rect).add(egui::Shape::mesh(mesh));
    }

    fn render_metadata_group(&mut self, ui: &mut egui::Ui, component: &ComponentNode) {
        self.render_metadata_artist(ui);
        self.render_metadata_album(ui);
        if Self::component_param_bool(component, "show_state")
            .or_else(|| Self::component_param_bool(component, "state"))
            .unwrap_or(true)
        {
            let show_label = Self::component_param_bool(component, "show_state_label")
                .or_else(|| Self::component_param_bool(component, "state_label"))
                .unwrap_or(true);
            self.render_metadata_state(ui, show_label);
        }
    }

    fn render_metadata_artist(&mut self, ui: &mut egui::Ui) {
        if !self.now.artist.is_empty() {
            self.skin_manager
                .skin_text(ui, format!("Artist: {}", self.now.artist), false);
        }
    }

    fn render_metadata_album(&mut self, ui: &mut egui::Ui) {
        if !self.now.album.is_empty() {
            self.skin_manager
                .skin_text(ui, format!("Album: {}", self.now.album), false);
        }
    }

    fn render_metadata_state(&mut self, ui: &mut egui::Ui, show_label: bool) {
        let state_text = playstate_to_str(self.now.state);
        let content = if show_label {
            format!("State: {state_text}")
        } else {
            state_text.to_string()
        };
        self.skin_manager.skin_text(ui, content, false);
    }

    fn render_playback_controls_group(&mut self, ui: &mut egui::Ui, centered: bool) {
        let base_height = ui.style().spacing.interact_size.y.max(40.0);
        let available_width = ui.available_width().max(1.0);
        let effective_width = available_width.min(PLAYBACK_CONTROLS_MAX_WIDTH);

        let style = ui.style();
        let base_button_width = style.spacing.interact_size.x.max(96.0);
        let base_row_width = 3.0 * base_button_width + 2.0 * PLAYBACK_CONTROL_SPACING_X;
        let scale = if base_row_width <= f32::EPSILON {
            1.0
        } else {
            (effective_width / base_row_width).clamp(0.6, 1.0)
        };

        let button_width = (base_button_width * scale).clamp(60.0, base_button_width);
        let button_height = (base_height * scale).clamp(28.0, base_height);
        let spacing = (PLAYBACK_CONTROL_SPACING_X * scale).clamp(6.0, PLAYBACK_CONTROL_SPACING_X);
        let row_width = 3.0 * button_width + 2.0 * spacing;

        let metrics = StripMetrics::from_content(available_width, row_width);
        let align = if centered {
            egui::Align::Center
        } else {
            Self::align_from_layout(ui.layout())
        };

        metrics.show_anchored(ui, align, |inner| {
            inner.allocate_ui_with_layout(
                egui::vec2(row_width, button_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |row| {
                    self.render_playback_buttons_row(
                        row,
                        scale,
                        egui::vec2(button_width, button_height),
                        spacing,
                    );
                },
            );
        });
    }

    fn render_playback_buttons_row(
        &mut self,
        row: &mut egui::Ui,
        scale: f32,
        button_size: egui::Vec2,
        button_spacing: f32,
    ) {
        let scale = scale.clamp(0.6, 1.0);
        row.set_height(button_size.y);
        let spacing_cfg = row.spacing_mut();
        spacing_cfg.item_spacing.x = button_spacing;
        spacing_cfg.item_spacing.y = 0.0;

        for kind in [
            PlaybackButtonKind::Previous,
            PlaybackButtonKind::PlayPause,
            PlaybackButtonKind::Next,
        ] {
            row.allocate_ui_with_layout(
                button_size,
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |cell| {
                    self.render_playback_button(cell, kind, scale);
                },
            );
        }
    }

    fn render_playback_button(&mut self, ui: &mut egui::Ui, kind: PlaybackButtonKind, scale: f32) {
        let scale = scale.clamp(0.6, 1.0);
        match kind {
            PlaybackButtonKind::Previous => {
                let response = self
                    .skin_manager
                    .skin_button_scaled(ui, "⏮", scale)
                    .on_hover_text("Previous track");
                if response.clicked() {
                    self.playback_command("Previous", |session| {
                        block_on_operation(session.TrySkipPreviousAsync()?)
                    });
                }
            }
            PlaybackButtonKind::PlayPause => {
                let is_playing = self.now.state == PlayState::Playing;
                let glyph = if is_playing { "⏸" } else { "▶" };
                let hint = if is_playing { "Pause" } else { "Play" };
                let response = self
                    .skin_manager
                    .skin_button_scaled(ui, glyph, scale)
                    .on_hover_text(hint);
                if response.clicked() {
                    if is_playing {
                        self.playback_command("Pause", |session| {
                            block_on_operation(session.TryPauseAsync()?)
                        });
                    } else {
                        self.playback_command("Play", |session| {
                            block_on_operation(session.TryPlayAsync()?)
                        });
                    }
                }
            }
            PlaybackButtonKind::Next => {
                let response = self
                    .skin_manager
                    .skin_button_scaled(ui, "⏭", scale)
                    .on_hover_text("Next track");
                if response.clicked() {
                    self.playback_command("Next", |session| {
                        block_on_operation(session.TrySkipNextAsync()?)
                    });
                }
            }
        }
    }

    fn render_timeline_component(
        &mut self,
        ui: &mut egui::Ui,
        centered: bool,
        show_separator: bool,
    ) {
        if show_separator {
            //ui.separator();
        }

        let Some(timeline) = &mut self.timeline else {
            self.skin_manager
                .skin_text(ui, "Timeline unavailable for this session.", false);
            return;
        };

        let duration = timeline.duration_secs();
        let mut relative = if duration > 0.0 {
            (timeline.position_secs - timeline.start_secs).clamp(0.0, duration)
        } else {
            0.0
        };
        let previous_position = timeline.position_secs;

        let metrics = timeline_strip_metrics(ui.available_width(), centered);

        if duration > f64::EPSILON {
            let mut slider_value = relative;
            let response = {
                let skin = &mut self.skin_manager;
                metrics.show_anchored(ui, egui::Align::Center, |inner| {
                    inner.set_width(metrics.content_width());
                    Self::render_seek_slider_with_skin(
                        skin,
                        inner,
                        timeline.can_seek,
                        &mut slider_value,
                        duration,
                    )
                })
            };

            relative = slider_value;

            let start_label = format_timestamp(relative);
            let end_label = format_timestamp(duration);
            {
                let skin = &mut self.skin_manager;
                Self::render_timeline_labels_with_skin(
                    skin,
                    ui,
                    &metrics,
                    &start_label,
                    &end_label,
                );
            }

            if timeline.can_seek && response.changed() {
                let new_pos = timeline.start_secs + relative;
                timeline.position_secs = new_pos;
                self.is_user_seeking = true;
                self.pending_seek_target = None;
                self.pending_seek_deadline = None;
                self.last_position_secs = timeline.position_secs;
                self.last_position_update = Instant::now();
            }

            let commit_seek = timeline.can_seek
                && (response.drag_stopped() || (response.clicked() && !response.dragged()));

            if commit_seek {
                let target_secs = timeline.start_secs + relative;
                if (target_secs - previous_position).abs() > 0.001 {
                    self.pending_seek_target = Some(target_secs);
                    self.pending_seek_deadline = Some(Instant::now() + Duration::from_secs(4));
                    self.is_user_seeking = true;
                    self.last_position_secs = target_secs;
                    self.last_position_update = Instant::now();
                    self.playback_command("Seek", move |session| {
                        block_on_operation(
                            session.TryChangePlaybackPositionAsync(secs_to_ticks(target_secs))?,
                        )
                    });
                } else {
                    self.is_user_seeking = false;
                    self.pending_seek_target = None;
                    self.pending_seek_deadline = None;
                }
            }
        } else {
            let fraction = if timeline.end_secs > timeline.start_secs {
                ((timeline.position_secs - timeline.start_secs)
                    / (timeline.end_secs - timeline.start_secs))
                    .clamp(0.0, 1.0)
            } else {
                0.0
            } as f32;

            metrics.show_anchored(ui, egui::Align::Center, |inner| {
                inner.set_width(metrics.content_width());
                inner.add(egui::ProgressBar::new(fraction).desired_width(f32::INFINITY));
            });

            let start_label = format_timestamp(relative);
            {
                let skin = &mut self.skin_manager;
                Self::render_timeline_labels_with_skin(skin, ui, &metrics, &start_label, "Live");
            }
        }
    }

    fn render_seek_slider_with_skin(
        skin: &mut SkinManager,
        ui: &mut egui::Ui,
        can_seek: bool,
        value: &mut f64,
        duration: f64,
    ) -> egui::Response {
        if can_seek {
            skin.skin_slider(ui, value, 0.0..=duration)
        } else {
            ui.add_enabled_ui(false, |disabled| {
                skin.skin_slider(disabled, value, 0.0..=duration)
            })
            .inner
        }
    }

    fn render_timeline_labels_with_skin(
        skin: &mut SkinManager,
        ui: &mut egui::Ui,
        metrics: &StripMetrics,
        start_label: &str,
        end_label: &str,
    ) {
        metrics.show_anchored(ui, egui::Align::Center, |inner| {
            inner.set_width(metrics.content_width());
            inner.spacing_mut().item_spacing.x = TIMELINE_LABEL_GAP;
            inner.columns(2, |columns| {
                columns[0].with_layout(egui::Layout::left_to_right(egui::Align::Center), |col| {
                    skin.skin_text(col, start_label, false);
                });
                columns[1].with_layout(egui::Layout::right_to_left(egui::Align::Center), |col| {
                    skin.skin_text(col, end_label, false);
                });
            });
        });
    }

    fn render_skin_warnings(&mut self, ui: &mut egui::Ui) {
        for warn in &self.skin_warnings {
            ui.colored_label(
                egui::Color32::from_rgb(240, 200, 80),
                format!("Skin warning: {warn}"),
            );
        }
    }

    fn render_skin_error(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.skin_error {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                format!("Skin error: {err}"),
            );
        }
    }

    fn render_now_playing_error(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.err {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                format!("Error: {err}"),
            );
        }
    }

    fn render_thumbnail_error(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.thumbnail_err {
            ui.colored_label(
                egui::Color32::from_rgb(240, 200, 80),
                format!("Thumbnail error: {err}"),
            );
        }
    }

    fn apply_snapshot(&mut self, now: NowPlaying, timeline: Option<Timeline>) {
        let now_instant = Instant::now();
        let track_changed = self.now != now;
        if track_changed {
            self.pending_thumbnail = Some(PendingThumbnail::Clear { track: None });
            self.current_thumbnail_track = None;
            self.thumbnail_hash = None;
        }

        if track_changed
            || (self.thumbnail_texture.is_none()
                && self.thumbnail_inflight_request.is_none()
                && self.current_thumbnail_track.as_ref() != Some(&now))
        {
            self.request_thumbnail_for(now.clone());
        }

        if let Some(target) = self.pending_seek_target {
            if let Some(mut tl) = timeline.clone() {
                if (tl.position_secs - target).abs() <= 0.5 {
                    self.pending_seek_target = None;
                    self.pending_seek_deadline = None;
                    self.is_user_seeking = false;
                } else {
                    tl.position_secs = target;
                }
                self.last_position_secs = tl.position_secs;
                self.last_position_update = now_instant;
                self.timeline = Some(tl);
            } else {
                self.last_position_secs = target;
                self.last_position_update = now_instant;
            }

            if let Some(deadline) = self.pending_seek_deadline {
                if now_instant >= deadline {
                    self.pending_seek_target = None;
                    self.pending_seek_deadline = None;
                    self.is_user_seeking = false;
                }
            }
        } else if let Some(mut tl) = timeline.clone() {
            let predicted = self.last_position_secs
                + now_instant
                    .duration_since(self.last_position_update)
                    .as_secs_f64();
            if now.state == PlayState::Playing {
                let predicted_clamped = predicted.clamp(tl.start_secs, tl.end_secs);
                if self.timeline.is_some() {
                    let discrepancy = (predicted_clamped - tl.position_secs).abs();
                    let threshold = (tl.duration_secs() * 0.01).clamp(0.2, 7.0);
                    if discrepancy <= threshold || tl.duration_secs() <= f64::EPSILON {
                        tl.position_secs = predicted_clamped;
                    }
                } else {
                    tl.position_secs = predicted_clamped;
                }
            }
            self.last_position_secs = tl.position_secs;
            self.last_position_update = now_instant;
            self.timeline = Some(tl);
        } else {
            self.last_position_update = now_instant;
            self.timeline = None;
        }

        self.now = now;
        self.err = None;
        self.last_pull = Instant::now();
    }

    fn update_dynamic_gradients(&mut self, image: &ColorImage) {
        if !self.skin_manager.current_theme().use_gradient {
            self.clear_dynamic_gradients();
            return;
        }
        let components = &self.skin_manager.current_theme().components;
        let root_direction = gradient_direction_from_background(&components.root.background);
        let panel_direction = gradient_direction_from_background(&components.panel.background);
        self.dynamic_root_gradient = dynamic_gradient_from_image(image, root_direction);
        self.dynamic_panel_gradient = dynamic_gradient_from_image(image, panel_direction);
    }

    fn clear_dynamic_gradients(&mut self) {
        self.dynamic_root_gradient = None;
        self.dynamic_panel_gradient = None;
    }

    fn process_pending_thumbnail(&mut self, ctx: &egui::Context) {
        self.drain_thumbnail_channel();

        if let Some(pending) = self.pending_thumbnail.take() {
            match pending {
                PendingThumbnail::Clear { track } => {
                    self.thumbnail_texture = None;
                    self.thumbnail_base_texture = None;
                    self.thumbnail_base_image = None;
                    self.thumbnail_vinyl_image = None;
                    self.thumbnail_hash = None;
                    self.current_thumbnail_track = track.filter(|t| t == &self.now);
                    self.clear_dynamic_gradients();
                    self.vinyl_spin.reset();
                    self.vinyl_last_frame = None;
                }
                PendingThumbnail::Update {
                    track,
                    hash,
                    base_image,
                    vinyl_image,
                } => {
                    if track != self.now {
                        return;
                    }

                    if self.thumbnail_hash == Some(hash)
                        && self.current_thumbnail_track.as_ref() == Some(&track)
                    {
                        return;
                    }

                    self.update_dynamic_gradients(&base_image);

                    self.thumbnail_base_image = Some(base_image.clone());

                    let base_texture = ctx.load_texture(
                        "now_playing.thumbnail.base",
                        base_image.clone(),
                        TextureOptions::LINEAR,
                    );
                    self.thumbnail_base_texture = Some(base_texture);

                    let theme_disables_vinyl =
                        self.skin_manager.current_theme().disable_vinyl_thumbnail;
                    let vinyl_allowed = !theme_disables_vinyl;
                    let use_vinyl_now = self.config.ui.vinyl_thumbnail.enabled && vinyl_allowed;
                    let had_vinyl = vinyl_image.is_some();
                    let display_image = if use_vinyl_now {
                        vinyl_image.clone().unwrap_or_else(|| base_image.clone())
                    } else {
                        base_image.clone()
                    };
                    self.thumbnail_vinyl_image = vinyl_image;
                    let texture = ctx.load_texture(
                        "now_playing.thumbnail",
                        display_image,
                        TextureOptions::LINEAR,
                    );
                    self.thumbnail_texture = Some(texture);
                    self.thumbnail_hash = Some(hash);
                    self.current_thumbnail_track = Some(track);
                    self.thumbnail_err = None;
                    if use_vinyl_now && had_vinyl {
                        self.vinyl_spin.reset();
                        self.vinyl_last_frame = None;
                        self.vinyl_pending_refresh = false;
                    } else if use_vinyl_now {
                        self.vinyl_pending_refresh = true;
                    } else {
                        self.vinyl_spin.reset();
                        self.vinyl_last_frame = None;
                        self.vinyl_pending_refresh = false;
                    }
                }
            }
        }
    }

    fn maybe_refresh_vinyl_thumbnail(&mut self) {
        if self.vinyl_pending_refresh
            && self.current_thumbnail_track.is_some()
            && self.thumbnail_inflight_request.is_none()
        {
            self.force_thumbnail_refresh();
        }
    }

    fn force_thumbnail_refresh(&mut self) {
        self.thumbnail_texture = None;
        self.thumbnail_base_texture = None;
        self.thumbnail_base_image = None;
        self.thumbnail_vinyl_image = None;
        self.thumbnail_hash = None;
        self.pending_thumbnail = None;
        self.vinyl_spin.reset();
        self.vinyl_last_frame = None;
        if let Some(track) = self.current_thumbnail_track.clone() {
            self.thumbnail_inflight_track = None;
            self.thumbnail_inflight_request = None;
            self.request_thumbnail_for(track);
            self.vinyl_pending_refresh = false;
        } else {
            self.vinyl_pending_refresh = true;
        }
    }

    fn drain_thumbnail_channel(&mut self) {
        let mut clear_rx = false;
        if let Some(rx) = self.thumbnail_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        if Some(msg.request_id) != self.thumbnail_inflight_request {
                            continue;
                        }
                        self.thumbnail_inflight_request = None;
                        self.thumbnail_inflight_track = None;
                        clear_rx = true;

                        let ThumbnailMessage {
                            request_id: _,
                            track,
                            hash,
                            base_image,
                            vinyl_image,
                            error,
                        } = msg;

                        if let Some(err) = error {
                            self.err = Some(err.clone());
                            self.thumbnail_err = Some(err);
                            self.pending_thumbnail =
                                Some(PendingThumbnail::Clear { track: Some(track) });
                        } else if let (Some(base_image), Some(hash)) = (base_image, hash) {
                            self.pending_thumbnail = Some(PendingThumbnail::Update {
                                track,
                                hash,
                                base_image,
                                vinyl_image,
                            });
                        } else {
                            self.pending_thumbnail =
                                Some(PendingThumbnail::Clear { track: Some(track) });
                        }
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.thumbnail_inflight_request = None;
                        self.thumbnail_inflight_track = None;
                        clear_rx = true;
                        break;
                    }
                }
            }
        }

        if clear_rx {
            self.thumbnail_rx = None;
        }
    }

    fn request_thumbnail_for(&mut self, track: NowPlaying) {
        if self.thumbnail_inflight_track.as_ref() == Some(&track) {
            return;
        }

        let request_id = self.next_thumbnail_request_id;
        self.next_thumbnail_request_id = self.next_thumbnail_request_id.wrapping_add(1);

        let vinyl_enabled = self.config.ui.vinyl_thumbnail.enabled;
        let vinyl_config = self.config.ui.vinyl_thumbnail.clone();

        let (tx, rx) = mpsc::channel();
        self.thumbnail_rx = Some(rx);
        self.thumbnail_inflight_request = Some(request_id);
        self.thumbnail_inflight_track = Some(track.clone());

        thread::spawn(move || {
            let mut com_initialized = false;

            unsafe {
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                if hr.is_ok() {
                    com_initialized = true;
                } else if hr != RPC_E_CHANGED_MODE {
                    let _ = tx.send(ThumbnailMessage {
                        request_id,
                        track,
                        hash: None,
                        base_image: None,
                        vinyl_image: None,
                        error: Some(format!("COM init failed: {hr:?}")),
                    });
                    return;
                }
            }

            let result = fetch_thumbnail_bytes();
            let message = match result {
                Ok(Some(bytes)) => {
                    let hash = hash_bytes(&bytes);
                    match decode_thumbnail_image(&bytes) {
                        Ok(base_image) => {
                            let vinyl_image = if vinyl_enabled {
                                let options = VinylThumbnailOptions::from_config(
                                    &vinyl_config,
                                    base_image.size[0],
                                    base_image.size[1],
                                );
                                Some(render_vinyl(&base_image, &options))
                            } else {
                                None
                            };

                            ThumbnailMessage {
                                request_id,
                                track,
                                hash: Some(hash),
                                base_image: Some(base_image),
                                vinyl_image,
                                error: None,
                            }
                        }
                        Err(err) => ThumbnailMessage {
                            request_id,
                            track,
                            hash: None,
                            base_image: None,
                            vinyl_image: None,
                            error: Some(err),
                        },
                    }
                }
                Ok(None) => ThumbnailMessage {
                    request_id,
                    track,
                    hash: None,
                    base_image: None,
                    vinyl_image: None,
                    error: None,
                },
                Err(err) => ThumbnailMessage {
                    request_id,
                    track,
                    hash: None,
                    base_image: None,
                    vinyl_image: None,
                    error: Some(format!("{err:?}")),
                },
            };
            let _ = tx.send(message);

            if com_initialized {
                unsafe {
                    CoUninitialize();
                }
            }
        });
    }

    fn refresh_now_playing(&mut self) {
        match fetch_session_snapshot() {
            Ok((now, timeline)) => self.apply_snapshot(now, timeline),
            Err(e) => {
                self.err = Some(format!("{e:?}"));
                self.timeline = None;
            }
        }
        self.last_pull = Instant::now();
    }

    fn playback_command<F>(&mut self, action_name: &str, action: F)
    where
        F: FnOnce(&GlobalSystemMediaTransportControlsSession) -> WinResult<bool>,
    {
        let result = current_session().and_then(|session| action(&session));

        match result {
            Ok(true) => {
                self.refresh_now_playing();
            }
            Ok(false) => {
                self.err = Some(format!(
                    "{action_name} command was rejected by the media session."
                ));
                self.refresh_now_playing();
            }
            Err(e) => {
                self.err = Some(format!("{action_name} failed: {e:?}"));
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(tx) = self.snapshot_request_tx.take() {
            let _ = tx.send(SnapshotCommand::Shutdown);
        }
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let native_options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_transparent(true),
        ..Default::default()
    };
    let run_res = eframe::run_native(
        "Now Playing",
        native_options,
        Box::new(
            |_cc| -> std::result::Result<
                Box<dyn eframe::App>,
                Box<dyn std::error::Error + Send + Sync>,
            > { Ok(Box::new(App::default())) },
        ),
    );
    if let Err(e) = run_res {
        return Err(Box::new(e));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_thumbnail_image_fails_on_garbage_input() {
        let result = decode_thumbnail_image(&[0u8, 1u8, 2u8, 3u8]);
        assert!(result.is_err());
    }

    #[test]
    fn set_vinyl_enabled_switches_between_modes() {
        let ctx = egui::Context::default();
        let mut app = App::default();
        let _ = app.skin_manager.set_skin("aurora_vinyl", &ctx);
        app.snapshot_rx = None;
        app.config.ui.vinyl_thumbnail.enabled = false;
        app.thumbnail_vinyl_image = None;

        let base_color = egui::Color32::from_rgb(120, 80, 200);
        let base_image = ColorImage::new([2, 2], vec![base_color; 4]);
        let base_texture = ctx.load_texture(
            "test.thumbnail.base",
            base_image.clone(),
            TextureOptions::LINEAR,
        );
        app.thumbnail_base_image = Some(base_image);
        app.thumbnail_base_texture = Some(base_texture.clone());
        app.thumbnail_texture = Some(base_texture.clone());
        app.current_thumbnail_track = Some(NowPlaying::default());

        app.set_vinyl_enabled(&ctx, true);
        assert!(app.config.ui.vinyl_thumbnail.enabled);
        assert!(app.thumbnail_vinyl_image.is_some());

        app.set_vinyl_enabled(&ctx, false);
        assert!(!app.config.ui.vinyl_thumbnail.enabled);
        assert_eq!(
            app.thumbnail_texture.as_ref().map(|tex| tex.id()),
            Some(base_texture.id())
        );
    }
}
