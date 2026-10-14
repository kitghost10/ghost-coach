use crate::config::OverlayPrefs;
use crate::engine::Engine;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State,
    WebviewWindow,
};

pub fn place_overlay(win: &WebviewWindow) -> Result<(), String> {
    let monitor = win.app_handle().get_webview_window("main")
        .and_then(|main| main.current_monitor().ok().flatten())
        .or_else(|| win.current_monitor()
        .ok()
        .flatten())
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return Err("No screen is available for the floating coach.".into());
    };
    let origin = monitor.position();
    let screen = monitor.size();
    let scale = monitor.scale_factor();
    let (logical_w, logical_h, max_h) = (1080.0, 420.0, 0.42);

    let width = ((logical_w * scale) as u32).min((screen.width as f64 * 0.92) as u32);
    let height = ((logical_h * scale) as u32).min((screen.height as f64 * max_h) as u32);
    let width = width.max(640);
    let height = height.max(240);
    win.set_size(Size::Physical(PhysicalSize::new(width, height))).map_err(|e| e.to_string())?;

    let x = origin.x + (screen.width as i32 - width as i32) / 2;
    let y = origin.y + (16.0 * scale).round() as i32;
    win.set_position(Position::Physical(PhysicalPosition::new(x, y.max(origin.y)))).map_err(|e| e.to_string())?;
    Ok(())
}

fn pin_overlay(win: &WebviewWindow) -> Result<(), String> {
    win.set_skip_taskbar(true).map_err(|e| e.to_string())?;
    win.set_always_on_top(true).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn style_overlay(win: &WebviewWindow) {
    let _ = win.set_ignore_cursor_events(false);
    let _ = place_overlay(win);
    let _ = pin_overlay(win);
}

pub fn set_overlay_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    let overlay = app.get_webview_window("overlay")
        .ok_or_else(|| "The floating coach window is missing. Reopen Ghost Coach.".to_string())?;
    if visible {
        place_overlay(&overlay)?;
        pin_overlay(&overlay)?;
        overlay.show().map_err(|e| e.to_string())?;
        overlay.unminimize().map_err(|e| e.to_string())?;
        // Raise the coach above the main app, including on XWayland.
        overlay.set_focus().map_err(|e| e.to_string())?;
    } else {
        overlay.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
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
            place_overlay(&overlay)?;
            pin_overlay(&overlay)?;
        }
    }
    let _ = app.emit("overlay-prefs", &prefs);
    Ok(prefs)
}

#[tauri::command]
pub fn hide_overlay(app: AppHandle) -> Result<(), String> {
    set_overlay_visible(&app, false)
}

#[tauri::command]
pub fn show_overlay(app: AppHandle) -> Result<(), String> {
    set_overlay_visible(&app, true)
}
