use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::connect_async;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

pub enum LiveEvent {
    Ready(&'static str),
    Partial {
        speaker: &'static str,
        text: String,
    },
    Final {
        speaker: &'static str,
        text: String,
        turn_complete: bool,
    },
    /// Prospect paused. Transcript is often empty; use the last finalized line.
    TurnEnd(&'static str),
    Error(String),
    Warn(String),
}

pub async fn run_transcriber(
    api_key: String,
    pcm_rx: Receiver<Vec<u8>>,
    events: UnboundedSender<LiveEvent>,
    speaker: &'static str,
) -> Result<()> {
    let url = "wss://api.deepgram.com/v1/listen?\
model=nova-3&\
language=en&\
smart_format=true&\
interim_results=true&\
endpointing=150&\
utterance_end_ms=1000&\
vad_events=true&\
filler_words=false&\
encoding=linear16&\
sample_rate=24000&\
channels=1";

    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Token {api_key}").parse().context("auth header")?,
    );

    let (ws, _) = connect_async(request)
        .await
        .context("connect Deepgram Nova-3")?;
    let _ = events.send(LiveEvent::Ready(speaker));
    let (mut write, mut read) = ws.split();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    std::thread::spawn(move || {
        while let Ok(chunk) = pcm_rx.recv() {
            if tx.blocking_send(chunk).is_err() {
                break;
            }
        }
    });

    let mut keep = tokio::time::interval(Duration::from_secs(8));
    keep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut drain = tokio::time::interval(Duration::from_millis(100));
    drain.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut audio_open = true;
    let mut closing_since: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = keep.tick(), if audio_open => {
                let _ = write.send(Message::Text(r#"{"type":"KeepAlive"}"#.into())).await;
            }
            chunk = rx.recv(), if audio_open => {
                match chunk {
                    Some(bytes) => {
                        write.send(Message::Binary(bytes.into())).await?;
                    }
                    None => {
                        // Ask Deepgram for its final transcript, then keep the
                        // websocket open briefly so those results are not lost.
                        audio_open = false;
                        let _ = write.send(Message::Text(r#"{"type":"CloseStream"}"#.into())).await;
                        closing_since = Some(Instant::now());
                    }
                }
            }
            _ = drain.tick(), if !audio_open => {
                if closing_since
                    .map(|started| started.elapsed() >= Duration::from_secs(3))
                    .unwrap_or(false)
                {
                    break;
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => handle_event(&text, &events, speaker),
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(err)) => {
                        let _ = events.send(LiveEvent::Error(err.to_string()));
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn handle_event(text: &str, events: &UnboundedSender<LiveEvent>, speaker: &'static str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if ty == "Error" || v.get("err_code").is_some() {
        let msg = v
            .get("message")
            .or_else(|| v.get("err_msg"))
            .and_then(|x| x.as_str())
            .unwrap_or(text);
        let _ = events.send(LiveEvent::Error(msg.to_string()));
        return;
    }
    let transcript = v
        .pointer("/channel/alternatives/0/transcript")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let speech_final = v.get("speech_final").and_then(|x| x.as_bool()).unwrap_or(false);
    let is_final = v.get("is_final").and_then(|x| x.as_bool()).unwrap_or(false);
    // UtteranceEnd / speech_final often arrive with an empty transcript. Those still
    // mean the speaker paused — do not drop them or the coach never runs.
    if ty == "UtteranceEnd" || (transcript.is_empty() && speech_final) {
        let _ = events.send(LiveEvent::TurnEnd(speaker));
        return;
    }
    if transcript.is_empty() {
        return;
    }
    if speech_final || is_final {
        let _ = events.send(LiveEvent::Final {
            speaker,
            text: transcript.to_string(),
            turn_complete: speech_final,
        });
    } else {
        let _ = events.send(LiveEvent::Partial {
            speaker,
            text: transcript.to_string(),
        });
    }
}
