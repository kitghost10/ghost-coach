use crate::record;
use crate::record::CallMeta;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

fn default_provider() -> String {
    "openrouter".into()
}

fn default_google_model() -> String {
    "gemini-2.5-flash-lite".into()
}

fn default_openai_model() -> String {
    "gpt-4o-mini".into()
}

fn default_openrouter_model() -> String {
    "openai/gpt-6-luna".into()
}

fn default_overlay_theme() -> String {
    "light".into()
}

fn default_overlay_layout() -> String {
    "both".into()
}

fn default_overlay_size() -> String {
    "comfortable".into()
}

fn default_overlay_position() -> String {
    "top".into()
}

fn default_font_scale() -> f32 {
    1.0
}

fn default_opacity() -> f32 {
    0.88
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayPrefs {
    #[serde(default = "default_overlay_theme")]
    pub theme: String,
    #[serde(default = "default_overlay_layout")]
    pub layout: String,
    #[serde(default = "default_overlay_size")]
    pub size: String,
    #[serde(default = "default_overlay_position")]
    pub position: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

impl Default for OverlayPrefs {
    fn default() -> Self {
        Self {
            theme: default_overlay_theme(),
            layout: default_overlay_layout(),
            size: default_overlay_size(),
            position: default_overlay_position(),
            font_scale: default_font_scale(),
            opacity: default_opacity(),
        }
    }
}

impl OverlayPrefs {
    pub fn sanitized(self) -> Self {
        let theme = if self.theme == "dark" { "dark" } else { "light" };
        let layout = match self.layout.as_str() {
            "captions" | "suggest" => self.layout.as_str(),
            _ => "both",
        };
        let size = match self.size.as_str() {
            "compact" | "large" => self.size.as_str(),
            _ => "comfortable",
        };
        let position = if self.position == "bottom" {
            "bottom"
        } else {
            "top"
        };
        Self {
            theme: theme.into(),
            layout: layout.into(),
            size: size.into(),
            position: position.into(),
            font_scale: self.font_scale.clamp(0.85, 1.5),
            opacity: self.opacity.clamp(0.5, 0.96),
        }
    }
}

#[derive(Deserialize, Serialize, Default)]
pub struct SavedConfig {
    #[serde(default)]
    pub deepgram_key: String,
    #[serde(default, alias = "api_key")]
    pub openai_key: String,
    #[serde(default = "default_openai_model")]
    pub openai_model: String,
    #[serde(default)]
    pub google_api_key: String,
    #[serde(default = "default_google_model")]
    pub google_model: String,
    #[serde(default)]
    pub openrouter_key: String,
    #[serde(default = "default_openrouter_model")]
    pub openrouter_model: String,
    #[serde(default = "default_provider")]
    pub ai_provider: String,
    #[serde(default)]
    pub click_through: bool,
    #[serde(default)]
    pub overlay: OverlayPrefs,
}

pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

#[tauri::command]
pub fn list_calls(app: AppHandle) -> Result<Vec<CallMeta>, String> {
    record::list_calls(&data_dir(&app)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_call(app: AppHandle, id: String) -> Result<record::CallDetail, String> {
    record::load_call(&data_dir(&app)?, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_calls_folder(app: AppHandle) -> Result<(), String> {
    let dir = record::calls_dir(&data_dir(&app)?).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_call_audio(app: AppHandle, id: String) -> Result<(), String> {
    let path = record::call_wav_path(&data_dir(&app)?, &id);
    if !path.exists() {
        return Err("No recording for this call yet.".into());
    }
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_config(app: AppHandle) -> Result<SavedConfig, String> {
    let path = config_path(&app)?;
    if !path.exists() {
        return Ok(SavedConfig {
            click_through: false,
            ai_provider: default_provider(),
            google_model: default_google_model(),
            openai_model: default_openai_model(),
            openrouter_model: default_openrouter_model(),
            overlay: OverlayPrefs::default(),
            ..SavedConfig::default()
        });
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_config(
    app: AppHandle,
    deepgram_key: String,
    openai_key: String,
    openai_model: String,
    google_api_key: String,
    google_model: String,
    openrouter_key: String,
    openrouter_model: String,
    ai_provider: String,
    click_through: bool,
    overlay: Option<OverlayPrefs>,
) -> Result<(), String> {
    let path = config_path(&app)?;
    let existing = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SavedConfig>(&raw).ok());
    let mut cfg = existing.unwrap_or_default();
    cfg.deepgram_key = deepgram_key;
    cfg.openai_key = openai_key;
    cfg.openai_model = openai_model;
    cfg.google_api_key = google_api_key;
    cfg.google_model = google_model;
    cfg.openrouter_key = openrouter_key;
    cfg.openrouter_model = openrouter_model;
    cfg.ai_provider = ai_provider;
    cfg.click_through = click_through;
    if let Some(overlay) = overlay {
        cfg.overlay = overlay.sanitized();
    }
    fs::write(path, serde_json::to_string_pretty(&cfg).unwrap()).map_err(|e| e.to_string())
}
