use crate::audio;
use crate::audio::Ear;
use crate::coach;
use crate::coach::CoachSettings;
use crate::engine::Engine;
use crate::events::{CoachEvt, Meters, SessionUi, StatusEvt, TranscriptEvt};
use crate::live::LiveEvent;
use crate::overlay_win::set_overlay_visible;
use crate::preview::{start_meter_preview, stop_meter_preview};
use crate::record;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
pub fn set_auto_answer(engine: State<Engine>, enabled: bool) -> Result<(), String> {
    engine.auto_answer.store(enabled, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn generate_answer(app: AppHandle, engine: State<Engine>) -> Result<(), String> {
    let settings = engine
        .coach
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Start a session first.".to_string())?;
    if !settings.enabled() {
        return Err("Add an AI key in Settings.".into());
    }
    let said = engine.last_prospect.lock().unwrap().clone();
    let hist = engine.history.lock().unwrap().clone();
    let brief = engine.brief.lock().unwrap().clone();
    if said.trim().is_empty() {
        let _ = app.emit(
            "coach",
            CoachEvt {
                question: String::new(),
                answer: String::new(),
                why: String::new(),
                tag: "WAIT".into(),
            },
        );
        return Ok(());
    }
    let app_c = app.clone();
    let log = engine.transcript.clone();
    let log_sink = engine.sink.lock().unwrap().clone();
    tauri::async_runtime::spawn(async move {
        match coach::suggest(&settings, &said, &hist, &brief).await {
            Ok(reply) => {
                if !reply.answer.is_empty() {
                    let mut lines = log.lock().unwrap();
                    lines.push(record::Entry::new("coach", reply.answer.clone(), &reply.tag));
                    if let Some(sink) = &log_sink {
                        sink.flush(&lines);
                    }
                }
                let _ = app_c.emit(
                    "coach",
                    CoachEvt {
                        question: reply.question,
                        answer: reply.answer,
                        why: reply.why,
                        tag: reply.tag,
                    },
                );
            }
            Err(err) => {
                let _ = app_c.emit(
                    "status",
                    StatusEvt {
                        text: format!("Coach: {err}"),
                        live: true,
                    },
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn start_listening(
    app: AppHandle,
    engine: State<Engine>,
    deepgram_key: String,
    openai_key: String,
    openai_model: String,
    google_api_key: String,
    google_model: String,
    openrouter_key: String,
    openrouter_model: String,
    ai_provider: String,
    auto_answer: Option<bool>,
    prospect_brief: Option<String>,
) -> Result<(), String> {
    if deepgram_key.trim().is_empty() {
        return Err("Paste a Deepgram API key first.".into());
    }
    if ai_provider.eq_ignore_ascii_case("openrouter") && !openrouter_key.trim().is_empty() && openrouter_model.trim().is_empty() {
        return Err("Enter an OpenRouter model ID in Connections.".into());
    }
    if !engine.stop.load(Ordering::Relaxed) {
        return Err("Already listening.".into());
    }
    engine.stop.store(false, Ordering::Relaxed);
    engine.you_talking.store(false, Ordering::Relaxed);
    let session_gen = engine.session_gen.clone();
    let gen = session_gen.fetch_add(1, Ordering::Relaxed) + 1;
    stop_meter_preview(&engine);

    let stop = engine.stop.clone();
    let you_talking = engine.you_talking.clone();
    let handle = app.clone();
    let saved = crate::config::load_config(app.clone()).ok();
    let dg = nonempty(deepgram_key, saved.as_ref().map(|s| s.deepgram_key.as_str()));
    let openai_key = nonempty(openai_key, saved.as_ref().map(|s| s.openai_key.as_str()));
    let openai_model = nonempty(openai_model, saved.as_ref().map(|s| s.openai_model.as_str()));
    let google_api_key = nonempty(
        google_api_key,
        saved.as_ref().map(|s| s.google_api_key.as_str()),
    );
    let google_model = nonempty(
        google_model,
        saved.as_ref().map(|s| s.google_model.as_str()),
    );
    let openrouter_key = nonempty(
        openrouter_key,
        saved.as_ref().map(|s| s.openrouter_key.as_str()),
    );
    let openrouter_model = openrouter_model.trim().to_string();
    let ai_provider = nonempty(ai_provider, saved.as_ref().map(|s| s.ai_provider.as_str()));
    let coach_cfg = CoachSettings {
        provider: if ai_provider.is_empty() {
            "openrouter".into()
        } else {
            ai_provider.to_ascii_lowercase()
        },
        google_api_key,
        google_model,
        openai_key,
        openai_model,
        openrouter_key,
        openrouter_model,
    };
    *engine.coach.lock().unwrap() = Some(coach_cfg.clone());
    *engine.last_prospect.lock().unwrap() = String::new();
    *engine.history.lock().unwrap() = Vec::new();
    *engine.brief.lock().unwrap() = prospect_brief.unwrap_or_default();
    let auto_on = auto_answer.unwrap_or(true);
    engine.auto_answer.store(auto_on, Ordering::Relaxed);
    let auto_answer = engine.auto_answer.clone();
    let session_ui = SessionUi {
        auto_answer: auto_on,
        coach_ready: coach_cfg.enabled(),
    };
    let _ = app.emit("session-ui", session_ui.clone());

    set_overlay_visible(&app, true);
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("session-ui", session_ui);
        let _ = app.emit(
            "status",
            StatusEvt {
                text: if coach_cfg.enabled() {
                    "Starting… auto-suggest on.".into()
                } else {
                    "Starting… captions only (coach key missing).".into()
                },
                live: true,
            },
        );
        let result = run_session(
            handle.clone(),
            stop,
            you_talking,
            auto_answer,
            dg,
            coach_cfg.clone(),
            session_gen.clone(),
            gen,
        )
        .await;
        let still_current = session_gen.load(Ordering::Relaxed) == gen;
        if still_current {
            start_meter_preview(handle.clone(), handle.state::<Engine>().preview_stop.clone());
            set_overlay_visible(&handle, false);
            match result {
                Err(err) => {
                    let _ = app.emit(
                        "status",
                        StatusEvt {
                            text: format!("{err:#}"),
                            live: false,
                        },
                    );
                }
                Ok(()) => {
                    let _ = app.emit(
                        "status",
                        StatusEvt {
                            text: "Call saved. Writing the post-call brief…".into(),
                            live: false,
                        },
                    );
                    let _ = app.emit("calls-updated", ());
                    spawn_auto_recap(handle.clone(), coach_cfg.clone(), gen, session_gen.clone());
                }
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn stop_listening(app: AppHandle, engine: State<Engine>) -> Result<(), String> {
    engine.stop.store(true, Ordering::Relaxed);
    set_overlay_visible(&app, false);
    let _ = app.emit(
        "status",
        StatusEvt {
            text: "Session ended.".into(),
            live: false,
        },
    );
    Ok(())
}

async fn run_session(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    you_talking: Arc<AtomicBool>,
    auto_answer: Arc<AtomicBool>,
    deepgram_key: String,
    coach_cfg: CoachSettings,
    session_gen: Arc<AtomicU64>,
    gen: u64,
) -> anyhow::Result<()> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let (call_id, started_unix) = record::new_call_id();
    let call_dir = record::calls_dir(&data)?.join(&call_id);
    fs::create_dir_all(&call_dir)?;
    let session_started = Instant::now();

    let (you_cap_tx, you_cap_rx) = mpsc::sync_channel::<Vec<u8>>(256);
    let (them_cap_tx, them_cap_rx) = mpsc::sync_channel::<Vec<u8>>(256);
    let (you_dg_tx, you_dg_rx) = mpsc::sync_channel::<Vec<u8>>(256);
    let (them_dg_tx, them_dg_rx) = mpsc::sync_channel::<Vec<u8>>(256);
    let (you_rec_tx, you_rec_rx) = mpsc::sync_channel::<Vec<u8>>(512);
    let (them_rec_tx, them_rec_rx) = mpsc::sync_channel::<Vec<u8>>(512);
    let _keep_you_dg = you_dg_tx.clone();
    let _keep_them_dg = them_dg_tx.clone();
    let _keep_you_rec = you_rec_tx.clone();
    let _keep_them_rec = them_rec_tx.clone();
    let (lvl_tx, lvl_rx) = mpsc::sync_channel::<(Ear, f32)>(32);
    let (audio_err_tx, audio_err_rx) = mpsc::sync_channel::<(Ear, String)>(2);

    let stop_you = stop.clone();
    let stop_them = stop.clone();
    let you_h = audio::spawn_ear(
        Ear::You,
        stop_you,
        Some(you_cap_tx),
        lvl_tx.clone(),
        audio_err_tx.clone(),
    );
    let them_h = audio::spawn_ear(
        Ear::Them,
        stop_them,
        Some(them_cap_tx),
        lvl_tx,
        audio_err_tx,
    );
    record::spawn_tee(you_cap_rx, you_dg_tx, you_rec_tx);
    record::spawn_tee(them_cap_rx, them_dg_tx, them_rec_tx);
    let wav_h = record::spawn_wav_mixer(
        you_rec_rx,
        them_rec_rx,
        call_dir.join("call.wav"),
        stop.clone(),
    );

    let app_lvl = app.clone();
    let stop_lvl = stop.clone();
    let you_flag = you_talking.clone();
    std::thread::spawn(move || {
        let mut you = 0.0f32;
        let mut them = 0.0f32;
        let mut last_you = Instant::now() - Duration::from_secs(2);
        while !stop_lvl.load(Ordering::Relaxed) {
            match lvl_rx.recv_timeout(Duration::from_millis(120)) {
                Ok((Ear::You, v)) => {
                    you = v;
                    if v > 0.04 {
                        last_you = Instant::now();
                    }
                }
                Ok((Ear::Them, v)) => them = v,
                Err(_) => {}
            }
            let talking = last_you.elapsed() < Duration::from_millis(450);
            you_flag.store(talking, Ordering::Relaxed);
            let _ = app_lvl.emit("meters", Meters { you, them });
            let _ = app_lvl.emit("you-talking", talking);
        }
    });

    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel::<LiveEvent>();
    let transcribe_you = spawn_labeled_transcriber(
        deepgram_key.clone(),
        you_dg_rx,
        ev_tx.clone(),
        stop.clone(),
        "you",
        false,
    );
    let transcribe_them = spawn_labeled_transcriber(
        deepgram_key,
        them_dg_rx,
        ev_tx,
        stop.clone(),
        "them",
        true,
    );

    let _ = app.emit(
        "status",
        StatusEvt {
            text: "Connecting to Deepgram…".into(),
            live: true,
        },
    );

    let transcript = app.state::<Engine>().transcript.clone();
    transcript.lock().unwrap().clear();
    let sink = Arc::new(record::CallSink {
        dir: call_dir.clone(),
        id: call_id.clone(),
        started_unix,
        provider: coach_cfg.provider.clone(),
        model: if coach_cfg.provider == "openai" {
            coach_cfg.openai_model.clone()
        } else if coach_cfg.provider == "openrouter" {
            coach_cfg.openrouter_model.clone()
        } else {
            coach_cfg.google_model.clone()
        },
    });
    *app.state::<Engine>().sink.lock().unwrap() = Some(sink.clone());
    let mut partial = String::new();
    let mut history: Vec<String> = Vec::new();
    let mut last_coach = Instant::now() - Duration::from_secs(10);
    let mut last_coached_line = String::new();
    let mut fatal_error: Option<String> = None;
    let coach_tick = Arc::new(AtomicU64::new(0));
    if coach_cfg.enabled() {
        let warm = coach_cfg.clone();
        tauri::async_runtime::spawn(async move {
            coach::warmup(&warm).await;
        });
    }

    while !stop.load(Ordering::Relaxed) {
        tokio::select! {
            ev = ev_rx.recv() => {
                match ev {
                    Some(LiveEvent::Ready(who)) => {
                        let _ = app.emit(
                            "status",
                            StatusEvt {
                                text: format!("Live · recording · {who} connected"),
                                live: true,
                            },
                        );
                    }
                    Some(LiveEvent::Partial { speaker, text }) => {
                        partial = text;
                        let _ = app.emit(
                            "transcript",
                            TranscriptEvt {
                                kind: "partial".into(),
                                text: partial.clone(),
                                speaker: speaker.into(),
                            },
                        );
                    }
                    Some(LiveEvent::Final { speaker, text, turn_complete }) => {
                        partial.clear();
                        let said = text.trim().to_string();
                        if said.is_empty() {
                            continue;
                        }
                        let _ = app.emit(
                            "transcript",
                            TranscriptEvt {
                                kind: "final".into(),
                                text: said.clone(),
                                speaker: speaker.into(),
                            },
                        );
                        let label = if speaker == "you" { "You" } else { "Prospect" };
                        {
                            let mut lines = transcript.lock().unwrap();
                            lines.push(record::Entry::new(speaker, said.clone(), ""));
                            sink.flush(&lines);
                        }
                        history.push(format!("{label}: {said}"));
                        if speaker == "them" {
                            if let Ok(mut last) = app.state::<Engine>().last_prospect.lock() {
                                *last = said.clone();
                            }
                        }
                        if history.len() > 12 {
                            history.remove(0);
                        }
                        if let Ok(mut hist) = app.state::<Engine>().history.lock() {
                            *hist = history.clone();
                        }
                        if speaker == "them" && (turn_complete || said.split_whitespace().count() >= 6)
                        {
                            request_coach(
                                &app,
                                &said,
                                &history,
                                &coach_cfg,
                                &auto_answer,
                                &mut last_coach,
                                &mut last_coached_line,
                                &coach_tick,
                                &transcript,
                                &sink,
                            );
                        }
                    }
                    Some(LiveEvent::TurnEnd(speaker)) => {
                        if speaker != "them" {
                            continue;
                        }
                        let said = app
                            .state::<Engine>()
                            .last_prospect
                            .lock()
                            .map(|g| g.clone())
                            .unwrap_or_default();
                        request_coach(
                            &app,
                            &said,
                            &history,
                            &coach_cfg,
                            &auto_answer,
                            &mut last_coach,
                            &mut last_coached_line,
                            &coach_tick,
                            &transcript,
                            &sink,
                        );
                    }
                    Some(LiveEvent::Warn(msg)) => {
                        let _ = app.emit(
                            "status",
                            StatusEvt {
                                text: msg,
                                live: true,
                            },
                        );
                    }
                    Some(LiveEvent::Error(msg)) => {
                        fatal_error = Some(msg);
                        break;
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if let Ok((ear, message)) = audio_err_rx.try_recv() {
                    match ear {
                        Ear::You => {
                            let _ = app.emit(
                                "status",
                                StatusEvt {
                                    text: format!("Mic unavailable, speakers only. {message}"),
                                    live: true,
                                },
                            );
                        }
                        Ear::Them => {
                            fatal_error = Some(format!(
                                "Speaker capture failed: {message}. Set the call/YouTube output to the default playback device."
                            ));
                            break;
                        }
                    }
                }
            }
        }
    }

    if session_gen.load(Ordering::Relaxed) == gen {
        stop.store(true, Ordering::Relaxed);
    }
    transcribe_you.abort();
    transcribe_them.abort();
    let _ = you_h.join();
    let _ = them_h.join();
    let _ = wav_h.join();
    let entries = transcript.lock().unwrap().clone();
    if session_gen.load(Ordering::Relaxed) == gen {
        sink.flush(&entries);
        *app.state::<Engine>().sink.lock().unwrap() = None;
    }
    let _ = session_started;
    if let Some(message) = fatal_error {
        anyhow::bail!(message);
    }
    Ok(())
}

fn spawn_labeled_transcriber(
    deepgram_key: String,
    pcm_rx: mpsc::Receiver<Vec<u8>>,
    ev_tx: tokio::sync::mpsc::UnboundedSender<LiveEvent>,
    stop: Arc<AtomicBool>,
    speaker: &'static str,
    fatal_on_fail: bool,
) -> tauri::async_runtime::JoinHandle<()> {
    let fatal_tx = ev_tx.clone();
    tauri::async_runtime::spawn(async move {
        match crate::live::run_transcriber(deepgram_key, pcm_rx, ev_tx, speaker).await {
            Ok(()) if !stop.load(Ordering::Relaxed) => {
                let msg = format!("The {speaker} transcription stream ended unexpectedly.");
                if fatal_on_fail {
                    let _ = fatal_tx.send(LiveEvent::Error(msg));
                } else {
                    let _ = fatal_tx.send(LiveEvent::Warn(msg));
                }
            }
            Ok(()) => {}
            Err(e) => {
                eprintln!("[transcriber {speaker}] error: {e:#}");
                let msg = format!("Could not start {speaker} transcription: {e}");
                if fatal_on_fail {
                    let _ = fatal_tx.send(LiveEvent::Error(msg));
                } else {
                    let _ = fatal_tx.send(LiveEvent::Warn(msg));
                }
            }
        }
    })
}

fn nonempty(passed: String, saved: Option<&str>) -> String {
    let trimmed = passed.trim().to_string();
    if !trimmed.is_empty() {
        return trimmed;
    }
    saved.unwrap_or("").trim().to_string()
}

/// After a successful session, wait for the recorder to finish flushing and
/// write an AI post-call brief for the newest call.
fn spawn_auto_recap(app: AppHandle, coach_cfg: CoachSettings, gen: u64, session_gen: Arc<AtomicU64>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        if session_gen.load(Ordering::Relaxed) != gen {
            return; // a new session started; don't touch its data
        }
        let base = match app.path().app_data_dir() {
            Ok(base) => base,
            Err(_) => return,
        };
        let calls = match record::list_calls(&base) {
            Ok(calls) => calls,
            Err(_) => return,
        };
        let Some(latest) = calls.first() else {
            return;
        };
        let id = latest.id.clone();
        let Ok(dir) = record::call_dir(&base, &id) else {
            return;
        };
        let detail = match record::load_call(&base, &id) {
            Ok(detail) => detail,
            Err(_) => return,
        };
        let mut history: Vec<String> = detail
            .entries
            .iter()
            .filter(|entry| entry.role != "coach")
            .map(|entry| {
                let who = if entry.role == "you" { "You" } else { "Prospect" };
                format!("{who}: {}", entry.text)
            })
            .collect();
        if history.is_empty() {
            return;
        }
        if history.len() > 120 {
            history = history.split_off(history.len() - 120);
        }
        let _ = app.emit(
            "status",
            StatusEvt {
                text: "Writing the post-call brief…".into(),
                live: false,
            },
        );
        let saved_user_note = record::load_brief(&dir).map(|b| b.user_note).unwrap_or_default();
        let brief = match coach::recap(&coach_cfg, &history).await {
            Ok(out) => record::PostCallBrief {
                about: out.about,
                key_points: out.key_points,
                next_step: out.next_step,
                user_note: saved_user_note,
                generated: true,
            },
            Err(err) => {
                let _ = app.emit(
                    "status",
                    StatusEvt {
                        text: format!("Post-call brief skipped: {err:#}"),
                        live: false,
                    },
                );
                return;
            }
        };
        let _ = record::save_brief(&dir, &brief);
        let _ = app.emit("calls-updated", ());
        let _ = app.emit(
            "status",
            StatusEvt {
                text: "Call saved. Post-call brief ready in Call review.".into(),
                live: false,
            },
        );
    });
}

fn request_coach(
    app: &AppHandle,
    said: &str,
    history: &[String],
    coach_cfg: &CoachSettings,
    auto_answer: &Arc<AtomicBool>,
    last_coach: &mut Instant,
    last_coached_line: &mut String,
    coach_tick: &Arc<AtomicU64>,
    transcript: &Arc<Mutex<Vec<record::Entry>>>,
    sink: &Arc<record::CallSink>,
) {
    let said = said.trim();
    if said.is_empty() {
        return;
    }
    let _ = app.emit(
        "coach",
        CoachEvt {
            question: said.to_string(),
            answer: String::new(),
            why: if auto_answer.load(Ordering::Relaxed) && coach_cfg.enabled() {
                "Drafting the next line…".into()
            } else {
                String::new()
            },
            tag: "CAPTION".into(),
        },
    );
    if !coach_cfg.enabled() {
        let _ = app.emit(
            "status",
            StatusEvt {
                text: "Captions are live. Add a coach API key in Settings to auto-suggest the next line.".into(),
                live: true,
            },
        );
        return;
    }
    if !auto_answer.load(Ordering::Relaxed) {
        return;
    }
    if said == last_coached_line {
        return;
    }
    if last_coach.elapsed() < Duration::from_millis(400) {
        return;
    }
    *last_coach = Instant::now();
    last_coached_line.clear();
    last_coached_line.push_str(said);
    let settings = coach_cfg.clone();
    let hist = history.to_vec();
    let brief = app.state::<Engine>().brief.lock().unwrap().clone();
    let app_c = app.clone();
    let log = transcript.clone();
    let log_sink = sink.clone();
    let prompt = said.to_string();
    let req_id = coach_tick.fetch_add(1, Ordering::Relaxed) + 1;
    let coach_tick_c = coach_tick.clone();
    tauri::async_runtime::spawn(async move {
        match coach::suggest(&settings, &prompt, &hist, &brief).await {
            Ok(reply) if !reply.answer.is_empty() => {
                if coach_tick_c.load(Ordering::Relaxed) != req_id {
                    return;
                }
                {
                    let mut lines = log.lock().unwrap();
                    lines.push(record::Entry::new(
                        "coach",
                        reply.answer.clone(),
                        &reply.tag,
                    ));
                    log_sink.flush(&lines);
                }
                let _ = app_c.emit(
                    "coach",
                    CoachEvt {
                        question: reply.question,
                        answer: reply.answer,
                        why: reply.why,
                        tag: reply.tag,
                    },
                );
            }
            Ok(reply) => {
                let _ = app_c.emit(
                    "coach",
                    CoachEvt {
                        question: reply.question,
                        answer: String::new(),
                        why: if reply.why.is_empty() {
                            "Waiting for a clearer line from them.".into()
                        } else {
                            reply.why
                        },
                        tag: "WAIT".into(),
                    },
                );
            }
            Err(err) => {
                let _ = app_c.emit(
                    "status",
                    StatusEvt {
                        text: format!("Coach: {err}"),
                        live: true,
                    },
                );
                let _ = app_c.emit(
                    "coach",
                    CoachEvt {
                        question: prompt,
                        answer: String::new(),
                        why: format!("{err}"),
                        tag: "ERROR".into(),
                    },
                );
            }
        }
    });
}
