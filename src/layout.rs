use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

pub const LAYOUT_ENGINE_VERSION: &str = "1";

#[derive(Debug, Clone)]
pub struct LoadedLayout {
    pub layout: LayoutSet,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LayoutSet {
    pub default_variant: String,
    pub variants: Vec<LayoutVariant>,
}

impl LayoutSet {
    pub fn variants(&self) -> &[LayoutVariant] {
        &self.variants
    }
}

#[derive(Debug, Clone)]
pub struct LayoutVariant {
    pub id: String,
    pub display_name: String,
    pub root: LayoutNode,
}

#[derive(Debug, Clone)]
pub enum LayoutNode {
    Row(ContainerNode),
    Column(ContainerNode),
    Component(ComponentNode),
    Spacer(SpacerNode),
}

#[derive(Debug, Clone)]
pub struct ContainerNode {
    pub spacing: f32,
    pub align: LayoutAlign,
    pub fill: bool,
    pub children: Vec<LayoutNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutAlign {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone)]
pub struct ComponentNode {
    pub component: LayoutComponent,
    pub visible: bool,
    pub params: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SpacerNode {
    pub size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutComponent {
    Thumbnail,
    Title,
    MetadataGroup,
    MetadataArtist,
    MetadataAlbum,
    MetadataState,
    PlaybackControlsGroup,
    PlaybackButtonPrevious,
    PlaybackButtonPlayPause,
    PlaybackButtonNext,
    PlaybackButtonStop,
    Timeline,
    SkinWarnings,
    SkinError,
    NowPlayingError,
    ThumbnailError,
}

pub fn load_layout_from_dir(skin_dir: &Path) -> Result<LoadedLayout> {
    let mut warnings = Vec::new();
    let layout_path = skin_dir.join("layout.toml");

    let document = if layout_path.exists() {
        let data = fs::read_to_string(&layout_path).with_context(|| {
            format!("Failed to read layout file from {}", layout_path.display())
        })?;
        match toml::from_str::<LayoutDocument>(&data) {
            Ok(doc) => {
                if let Some(engine) = doc.meta.engine.as_deref() {
                    if engine != LAYOUT_ENGINE_VERSION {
                        warnings.push(format!(
                            "Layout engine version {engine} does not match {LAYOUT_ENGINE_VERSION}; using defaults"
                        ));
                        builtin_layout_document()
                    } else {
                        doc
                    }
                } else {
                    warnings.push("layout.meta.engine missing; assuming version 1".to_string());
                    doc
                }
            }
            Err(err) => {
                warnings.push(format!("Failed to parse layout: {err}"));
                builtin_layout_document()
            }
        }
    } else {
        warnings.push(format!(
            "Skin folder {} missing layout.toml; falling back to defaults",
            skin_dir.display()
        ));
        builtin_layout_document()
    };

    let layout = resolve_document(document, &mut warnings)?;
    Ok(LoadedLayout { layout, warnings })
}

fn resolve_document(doc: LayoutDocument, warnings: &mut Vec<String>) -> Result<LayoutSet> {
    let mut variants = Vec::new();

    for (idx, variant_cfg) in doc.layout.variants.iter().enumerate() {
        let Some(structure) = variant_cfg.structure.clone() else {
            warnings.push(format!(
                "Layout variant {idx} is missing structure; skipping"
            ));
            continue;
        };

        let id = variant_cfg
            .id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("variant_{idx}"));
        let display_name = variant_cfg
            .display_name
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.clone());

        if variants
            .iter()
            .any(|variant: &LayoutVariant| variant.id == id)
        {
            warnings.push(format!("Duplicate layout variant id '{id}'; skipping"));
            continue;
        }

        match resolve_node(structure, warnings, &format!("variant '{id}'")) {
            Some(root) => variants.push(LayoutVariant {
                id,
                display_name,
                root,
            }),
            None => warnings.push(format!(
                "Layout variant '{id}' resolved to no visible content; skipping"
            )),
        }
    }

    if variants.is_empty() {
        let default_doc = builtin_layout_document();
        let default_set = resolve_document(default_doc, warnings)?;
        return Ok(default_set);
    }

    let default_variant = doc
        .layout
        .default
        .as_ref()
        .and_then(|candidate| {
            let trimmed = candidate.trim();
            variants
                .iter()
                .position(|variant| variant.id == trimmed)
                .map(|idx| variants[idx].id.clone())
        })
        .unwrap_or_else(|| variants[0].id.clone());

    Ok(LayoutSet {
        default_variant,
        variants,
    })
}

fn resolve_node(
    cfg: LayoutNodeConfig,
    warnings: &mut Vec<String>,
    context: &str,
) -> Option<LayoutNode> {
    match cfg {
        LayoutNodeConfig::Row(node) => {
            resolve_container(node, warnings, context).map(LayoutNode::Row)
        }
        LayoutNodeConfig::Column(node) => {
            resolve_container(node, warnings, context).map(LayoutNode::Column)
        }
        LayoutNodeConfig::Component(node) => {
            resolve_component(node, warnings, context).map(LayoutNode::Component)
        }
        LayoutNodeConfig::Spacer(node) => {
            let size = node.size.unwrap_or(8.0).max(0.0);
            if size <= f32::EPSILON {
                None
            } else {
                Some(LayoutNode::Spacer(SpacerNode { size }))
            }
        }
    }
}

fn resolve_container(
    cfg: ContainerConfig,
    warnings: &mut Vec<String>,
    context: &str,
) -> Option<ContainerNode> {
    if cfg.visible == Some(false) {
        return None;
    }

    let align = cfg
        .align
        .as_deref()
        .and_then(parse_align)
        .unwrap_or(LayoutAlign::Start);

    let spacing = cfg.spacing.unwrap_or(8.0).max(0.0);
    let fill = cfg.fill.unwrap_or(false);

    let mut children = Vec::new();
    for (child_idx, child_cfg) in cfg.children.into_iter().enumerate() {
        let child_context = format!("{context} > child #{child_idx}");
        if let Some(child) = resolve_node(child_cfg, warnings, &child_context) {
            children.push(child);
        }
    }

    if children.is_empty() {
        warnings.push(format!("{context} has no visible children"));
        return None;
    }

    Some(ContainerNode {
        spacing,
        align,
        fill,
        children,
    })
}

fn resolve_component(
    cfg: ComponentConfig,
    warnings: &mut Vec<String>,
    context: &str,
) -> Option<ComponentNode> {
    if cfg.visible == Some(false) {
        return None;
    }

    let Some(id) = cfg.id.as_deref() else {
        warnings.push(format!("{context} component missing id"));
        return None;
    };

    match parse_component(id) {
        Some(component) => Some(ComponentNode {
            component,
            visible: true,
            params: cfg.params.unwrap_or_default(),
        }),
        None => {
            warnings.push(format!("Unknown component '{id}' in {context}; skipping"));
            None
        }
    }
}

fn parse_align(value: &str) -> Option<LayoutAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "start" | "top" | "left" => Some(LayoutAlign::Start),
        "center" | "middle" => Some(LayoutAlign::Center),
        "end" | "bottom" | "right" => Some(LayoutAlign::End),
        _ => None,
    }
}

fn parse_component(value: &str) -> Option<LayoutComponent> {
    match value.trim().to_ascii_lowercase().as_str() {
        "thumbnail" | "artwork" => Some(LayoutComponent::Thumbnail),
        "title" => Some(LayoutComponent::Title),
        "metadata" | "metadata_group" | "details" => Some(LayoutComponent::MetadataGroup),
        "metadata.artist" | "artist" => Some(LayoutComponent::MetadataArtist),
        "metadata.album" | "album" => Some(LayoutComponent::MetadataAlbum),
        "metadata.state" | "state" | "playstate" => Some(LayoutComponent::MetadataState),
        "playback_controls" | "controls" => Some(LayoutComponent::PlaybackControlsGroup),
        "button.previous" | "previous" => Some(LayoutComponent::PlaybackButtonPrevious),
        "button.play" | "playpause" | "button.playpause" | "button.pause" => {
            Some(LayoutComponent::PlaybackButtonPlayPause)
        }
        "button.next" | "next" => Some(LayoutComponent::PlaybackButtonNext),
        "button.stop" | "stop" => Some(LayoutComponent::PlaybackButtonStop),
        "timeline" | "progress" => Some(LayoutComponent::Timeline),
        "skin_warnings" | "warnings" => Some(LayoutComponent::SkinWarnings),
        "skin_error" => Some(LayoutComponent::SkinError),
        "error" | "now_playing_error" => Some(LayoutComponent::NowPlayingError),
        "thumbnail_error" => Some(LayoutComponent::ThumbnailError),
        _ => None,
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct LayoutDocument {
    meta: LayoutMeta,
    layout: LayoutVariants,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct LayoutMeta {
    engine: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct LayoutVariants {
    default: Option<String>,
    variants: Vec<LayoutVariantConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct LayoutVariantConfig {
    id: Option<String>,
    display_name: Option<String>,
    structure: Option<LayoutNodeConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LayoutNodeConfig {
    Row(ContainerConfig),
    Column(ContainerConfig),
    Component(ComponentConfig),
    Spacer(SpacerConfig),
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ContainerConfig {
    align: Option<String>,
    spacing: Option<f32>,
    fill: Option<bool>,
    visible: Option<bool>,
    children: Vec<LayoutNodeConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct ComponentConfig {
    id: Option<String>,
    visible: Option<bool>,
    params: Option<HashMap<String, String>>,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
struct SpacerConfig {
    size: Option<f32>,
}

impl Default for LayoutDocument {
    fn default() -> Self {
        LayoutDocument {
            meta: LayoutMeta::default(),
            layout: LayoutVariants::default(),
        }
    }
}

impl Default for LayoutMeta {
    fn default() -> Self {
        LayoutMeta {
            engine: Some(LAYOUT_ENGINE_VERSION.to_string()),
        }
    }
}

impl Default for LayoutVariants {
    fn default() -> Self {
        LayoutVariants {
            default: Some("art_left".to_string()),
            variants: Vec::new(),
        }
    }
}

impl Default for LayoutVariantConfig {
    fn default() -> Self {
        LayoutVariantConfig {
            id: None,
            display_name: None,
            structure: None,
        }
    }
}

impl Default for ContainerConfig {
    fn default() -> Self {
        ContainerConfig {
            align: None,
            spacing: None,
            fill: None,
            visible: None,
            children: Vec::new(),
        }
    }
}

impl Default for ComponentConfig {
    fn default() -> Self {
        ComponentConfig {
            id: None,
            visible: None,
            params: None,
        }
    }
}

impl Default for SpacerConfig {
    fn default() -> Self {
        SpacerConfig { size: None }
    }
}

fn builtin_layout_document() -> LayoutDocument {
    toml::from_str(DEFAULT_LAYOUT_TOML).expect("Embedded default layout must parse")
}

const DEFAULT_LAYOUT_TOML: &str = r##"
[meta]
engine = "1"

[layout]
default = "art_left"

[[layout.variants]]
id = "art_left"
display_name = "Artwork Left"

[layout.variants.structure]
type = "row"
spacing = 16
fill = true

[[layout.variants.structure.children]]
type = "component"
id = "thumbnail"

[[layout.variants.structure.children]]
type = "column"
spacing = 8
fill = true

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "title"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "metadata"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "playback_controls"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "timeline"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "skin_warnings"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "skin_error"

  [[layout.variants.structure.children.children]]
    type = "component"
    id = "thumbnail_error"

    [[layout.variants.structure.children.children]]
  type = "component"
  id = "error"

[[layout.variants]]
id = "art_right"
display_name = "Artwork Right"

[layout.variants.structure]
type = "row"
spacing = 16
fill = true
align = "end"

[[layout.variants.structure.children]]
type = "column"
spacing = 8
fill = true

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "title"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "metadata"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "playback_controls"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "timeline"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "skin_warnings"

  [[layout.variants.structure.children.children]]
  type = "component"
  id = "skin_error"

  [[layout.variants.structure.children.children]]
    type = "component"
    id = "thumbnail_error"

    [[layout.variants.structure.children.children]]
  type = "component"
  id = "error"

[[layout.variants.structure.children]]
type = "component"
id = "thumbnail"

[[layout.variants]]
id = "art_top"
display_name = "Artwork Top"

[layout.variants.structure]
type = "column"
spacing = 12
fill = true
align = "center"

[[layout.variants.structure.children]]
type = "component"
id = "thumbnail"

[[layout.variants.structure.children]]
type = "component"
id = "title"

[[layout.variants.structure.children]]
type = "component"
id = "metadata"

[[layout.variants.structure.children]]
type = "component"
id = "playback_controls"
    [layout.variants.structure.children.params]
    centered = "true"

[[layout.variants.structure.children]]
type = "component"
id = "timeline"
    [layout.variants.structure.children.params]
    centered = "true"

[[layout.variants.structure.children]]
type = "component"
id = "skin_warnings"

[[layout.variants.structure.children]]
type = "component"
id = "skin_error"

[[layout.variants.structure.children]]
type = "component"
id = "thumbnail_error"

[[layout.variants.structure.children]]
type = "component"
id = "error"
"##;
