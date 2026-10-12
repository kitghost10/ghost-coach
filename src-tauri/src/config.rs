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

fn default_font_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayPrefs {
    #[serde(default = "default_overlay_theme")]
    pub theme: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
}

impl Default for OverlayPrefs {
    fn default() -> Self {
        Self {
            theme: default_overlay_theme(),
            font_scale: default_font_scale(),
        }
    }
}

impl OverlayPrefs {
    pub fn sanitized(self) -> Self {
        Self {
            theme: if self.theme == "dark" {
                "dark".into()
            } else {
                "light".into()
            },
            font_scale: self.font_scale.clamp(0.85, 1.5),
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
    if let Some(overlay) = overlay {
        cfg.overlay = overlay.sanitized();
    }
    fs::write(path, serde_json::to_string_pretty(&cfg).unwrap()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{OverlayPrefs, SavedConfig};

    #[test]
    fn legacy_config_keeps_connections_and_discards_retired_preferences() {
        let raw = r#"{
            "deepgram_key":"example-captions-key",
            "openrouter_key":"example-coach-key",
            "ai_provider":"openrouter",
            "click_through":true,
            "overlay":{"theme":"dark","font_scale":1.25,"layout":"suggest","size":"large","position":"bottom","opacity":0.5}
        }"#;
        let cfg: SavedConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.deepgram_key, "example-captions-key");
        assert_eq!(cfg.openrouter_key, "example-coach-key");
        assert_eq!(cfg.ai_provider, "openrouter");
        assert_eq!(cfg.overlay.theme, "dark");
        assert_eq!(cfg.overlay.font_scale, 1.25);
        let saved = serde_json::to_value(cfg).unwrap();
        assert!(saved.get("click_through").is_none());
        assert_eq!(
            saved["overlay"],
            serde_json::json!({"theme":"dark","font_scale":1.25})
        );
    }

    #[test]
    fn simplified_overlay_preferences_accept_text_size_and_use_defaults() {
        let prefs: OverlayPrefs =
            serde_json::from_str(r#"{"theme":"dark","font_scale":1.4}"#).unwrap();
        assert_eq!(prefs.sanitized().font_scale, 1.4);
        let defaults: OverlayPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.theme, "light");
        assert_eq!(defaults.font_scale, 1.0);
    }
}
