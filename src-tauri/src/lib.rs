mod audio;
mod coach;
mod config;
mod engine;
mod events;
mod live;
mod overlay_win;
mod playbook;
mod preview;
mod record;
mod session;

use config::{list_calls, load_call, load_config, open_call_audio, open_calls_folder, save_call_brief, save_config, generate_call_recap};
use engine::Engine;
use overlay_win::{hide_overlay, set_overlay_prefs, show_overlay};
use preview::{restart_meter_preview, start_meter_preview};
use session::{generate_answer, polish_line, set_auto_answer, start_listening, stop_listening};
use std::sync::atomic::Ordering;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Engine::default())
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            set_overlay_prefs,
            hide_overlay,
            show_overlay,
            set_auto_answer,
            generate_answer,
            polish_line,
            start_listening,
            stop_listening,
            list_calls,
            load_call,
            save_call_brief,
            generate_call_recap,
            open_calls_folder,
            open_call_audio,
            restart_meter_preview
        ])
        .setup(|app| {
            if let Some(overlay) = app.get_webview_window("overlay") {
                overlay_win::style_overlay(&overlay);
                let _ = overlay.hide();
            }

            if std::env::args().any(|arg| arg == "--preview-coach") {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Let the webview load before showing its idle preview.
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    if let Err(error) = overlay_win::set_overlay_visible(&handle, true) {
                        eprintln!("Floating coach: {error}");
                    }
                });
            }

            start_meter_preview(
                app.handle().clone(),
                app.state::<Engine>().preview_stop.clone(),
            );

            if let Some(main) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                let stop = app.state::<Engine>().stop.clone();
                let preview_stop = app.state::<Engine>().preview_stop.clone();
                let overlay_handle = app.get_webview_window("overlay");
                let sink = app.state::<Engine>().sink.clone();
                let transcript = app.state::<Engine>().transcript.clone();
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        if let Some(sink) = sink.lock().unwrap().take() {
                            let _ = sink.flush(&transcript.lock().unwrap());
                        }
                        stop.store(true, Ordering::Relaxed);
                        preview_stop.store(true, Ordering::Relaxed);
                        if let Some(ref overlay) = overlay_handle {
                            let _ = overlay.close();
                        }
                        app_handle.exit(0);
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
