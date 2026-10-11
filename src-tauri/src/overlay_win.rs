use crate::config::OverlayPrefs;
use crate::engine::Engine;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State, WebviewWindow,
};

pub fn overlay_pixel_size(prefs: &OverlayPrefs) -> (f64, f64, f64) {
    match prefs.size.as_str() {
        "compact" => (920.0, 300.0, 0.32),
        "large" => (1240.0, 560.0, 0.55),
        _ => (1080.0, 420.0, 0.42),
    }
}

pub fn place_overlay(win: &WebviewWindow, prefs: &OverlayPrefs) {
    let monitor = win
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let origin = monitor.position();
    let screen = monitor.size();
    let scale = monitor.scale_factor();
    let (logical_w, logical_h, max_h) = overlay_pixel_size(prefs);

    let width = ((logical_w * scale) as u32).min((screen.width as f64 * 0.92) as u32);
    let height = ((logical_h * scale) as u32).min((screen.height as f64 * max_h) as u32);
    let width = width.max(640);
    let height = height.max(240);
    let _ = win.set_size(Size::Physical(PhysicalSize::new(width, height)));

    let x = origin.x + (screen.width as i32 - width as i32) / 2;
    let y = if prefs.position == "bottom" {
        origin.y + screen.height as i32 - height as i32 - (20.0 * scale).round() as i32
    } else {
        origin.y + (16.0 * scale).round() as i32
    };
    let _ = win.set_position(Position::Physical(PhysicalPosition::new(x, y.max(origin.y))));
}

fn pin_overlay(win: &WebviewWindow) {
    let _ = win.unminimize();
    let _ = win.set_skip_taskbar(true);
    let _ = win.set_always_on_top(true);
}

fn stored_prefs(app: &AppHandle) -> OverlayPrefs {
    app.state::<Engine>()
        .overlay
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default()
}

pub fn style_overlay(win: &WebviewWindow) {
    let _ = win.set_ignore_cursor_events(false);
    place_overlay(win, &OverlayPrefs::default());
    pin_overlay(win);
}

pub fn set_overlay_visible(app: &AppHandle, visible: bool) {
    if let Some(overlay) = app.get_webview_window("overlay") {
        if visible {
            place_overlay(&overlay, &stored_prefs(app));
            let _ = overlay.show();
            pin_overlay(&overlay);
        } else {
            let _ = overlay.hide();
        }
    }
}

#[tauri::command]
pub fn set_overlay_prefs(
    app: AppHandle,
    engine: State<Engine>,
    prefs: OverlayPrefs,
) -> Result<OverlayPrefs, String> {
    let prefs = prefs.sanitized();
    *engine.overlay.lock().unwrap() = prefs.clone();
    if let Some(overlay) = app.get_webview_window("overlay") {
        if overlay.is_visible().unwrap_or(false) {
            place_overlay(&overlay, &prefs);
            pin_overlay(&overlay);
        }
    }
    let _ = app.emit("overlay-prefs", &prefs);
    Ok(prefs)
}

#[tauri::command]
pub fn set_click_through(app: AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        overlay
            .set_ignore_cursor_events(enabled)
            .map_err(|e| e.to_string())?;
        if !enabled {
            pin_overlay(&overlay);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn hide_overlay(app: AppHandle) -> Result<(), String> {
    set_overlay_visible(&app, false);
    Ok(())
}

#[tauri::command]
pub fn show_overlay(app: AppHandle) -> Result<(), String> {
    set_overlay_visible(&app, true);
    Ok(())
}
