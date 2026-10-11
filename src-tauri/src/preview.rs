use crate::audio;
use crate::audio::Ear;
use crate::engine::Engine;
use crate::events::Meters;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

static PREVIEW_GEN: AtomicU64 = AtomicU64::new(0);

pub fn stop_meter_preview(engine: &Engine) {
    engine.preview_stop.store(true, Ordering::Relaxed);
    PREVIEW_GEN.fetch_add(1, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(150));
}

pub fn start_meter_preview(app: AppHandle, preview_stop: Arc<AtomicBool>) {
    let gen = PREVIEW_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    preview_stop.store(false, Ordering::Relaxed);
    let (lvl_tx, lvl_rx) = mpsc::sync_channel::<(Ear, f32)>(32);
    let (err_tx, err_rx) = mpsc::sync_channel::<(Ear, String)>(2);
    let _ = audio::spawn_ear(
        Ear::You,
        preview_stop.clone(),
        None,
        lvl_tx.clone(),
        err_tx.clone(),
    );
    let _ = audio::spawn_ear(Ear::Them, preview_stop.clone(), None, lvl_tx, err_tx);
    std::thread::spawn(move || {
        let mut you = 0.0f32;
        let mut them = 0.0f32;
        while !preview_stop.load(Ordering::Relaxed) && PREVIEW_GEN.load(Ordering::Relaxed) == gen {
            while let Ok((ear, message)) = err_rx.try_recv() {
                let _ = app.emit("audio-preview-error", format!("{ear:?}: {message}"));
            }
            match lvl_rx.recv_timeout(Duration::from_millis(80)) {
                Ok((Ear::You, v)) => you = v,
                Ok((Ear::Them, v)) => them = v,
                Err(_) => {}
            }
            let _ = app.emit("meters", Meters { you, them });
        }
    });
}

#[tauri::command]
pub fn restart_meter_preview(app: AppHandle, engine: State<Engine>) -> Result<(), String> {
    stop_meter_preview(&engine);
    start_meter_preview(app, engine.preview_stop.clone());
    Ok(())
}
