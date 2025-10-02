use anyhow::Context;
use serde::Deserialize;
use std::{env, fs};

#[derive(Debug, Clone)]
pub struct Config {
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let mut candidates = Vec::new();

        if let Ok(current_dir) = env::current_dir() {
            candidates.push(current_dir.join("config.toml"));
            candidates.push(current_dir.join("config").join("config.toml"));
            candidates.push(current_dir.join("config").join("nowplaying.toml"));
        }

        if let Ok(exe) = env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("config.toml"));
                candidates.push(dir.join("config").join("config.toml"));
                candidates.push(dir.join("config").join("nowplaying.toml"));
            }
        }

        for path in candidates {
            if path.exists() {
                let data = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config file: {}", path.display()))?;
                let doc: ConfigDocument = toml::from_str(&data)
                    .with_context(|| format!("Failed to parse config: {}", path.display()))?;
                return Ok(doc.into());
            }
        }

        Ok(Config::default())
    }
}

#[derive(Debug, Clone)]
pub struct UiConfig {
    pub vinyl_thumbnail: VinylThumbnailConfig,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            vinyl_thumbnail: VinylThumbnailConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VinylThumbnailConfig {
    pub enabled: bool,
    pub swirl_strength: f32,
    pub label_ratio: f32,
}

impl Default for VinylThumbnailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            swirl_strength: 45.0,
            label_ratio: 0.95,
        }
    }
}

impl VinylThumbnailConfig {
    pub fn swirl_strength(&self) -> f32 {
        self.swirl_strength.clamp(0.0, 10.0)
    }

    pub fn label_ratio(&self) -> f32 {
        self.label_ratio.clamp(0.1, 0.6)
    }
}

#[derive(Debug, Default, Deserialize)]
struct ConfigDocument {
    #[serde(default)]
    ui: UiSection,
}

impl From<ConfigDocument> for Config {
    fn from(value: ConfigDocument) -> Self {
        let ui = UiConfig {
            vinyl_thumbnail: VinylThumbnailConfig {
                enabled: value.ui.vinyl_thumbnail.enabled.unwrap_or(false),
                swirl_strength: value.ui.vinyl_thumbnail.swirl_strength.unwrap_or(2.5),
                label_ratio: value.ui.vinyl_thumbnail.label_ratio.unwrap_or(0.35),
            },
        };

        Config { ui }
    }
}

#[derive(Debug, Default, Deserialize)]
struct UiSection {
    #[serde(default)]
    vinyl_thumbnail: VinylThumbnailSection,
}

#[derive(Debug, Default, Deserialize)]
struct VinylThumbnailSection {
    enabled: Option<bool>,
    swirl_strength: Option<f32>,
    label_ratio: Option<f32>,
}
