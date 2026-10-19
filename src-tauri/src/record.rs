use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SAMPLE_RATE: u32 = 24_000;
const FRAME_BYTES: usize = (SAMPLE_RATE as usize / 50) * 2; // 20 ms mono pcm16

#[derive(Serialize, Deserialize, Clone)]
pub struct CallMeta {
    pub id: String,
    pub started_unix: u64,
    pub duration_secs: u64,
    pub line_count: usize,
    pub preview: String,
}

/// One saved line: a spoken turn or an AI suggestion.
#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: String,
    pub at_unix_ms: u128,
    /// "you", "them" or "coach"
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub intent: String,
}

impl Entry {
    pub fn new(role: &str, text: String, intent: &str) -> Self {
        let at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Self {
            id: unique_id("line"),
            at_unix_ms,
            role: role.to_string(),
            text,
            intent: intent.to_string(),
        }
    }
}

/// Everything the app knows about one call session.
#[derive(Serialize, Deserialize, Clone)]
pub struct CallRecord {
    pub id: String,
    pub started_unix: u64,
    pub ended_unix: u64,
    pub duration_secs: u64,
    pub provider: String,
    pub model: String,
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CallDetail {
    pub meta: CallMeta,
    pub transcript: String,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub brief: PostCallBrief,
}

/// Post-call brief: what the meeting was about, and what happens next.
/// `summary`/`key_points`/`next_step` come from the AI recap; `user_note`
/// is the human-written part that is never overwritten by the AI.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PostCallBrief {
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub next_step: String,
    #[serde(default)]
    pub user_note: String,
    #[serde(default)]
    pub generated: bool,
}

fn brief_path(dir: &PathBuf) -> PathBuf {
    dir.join("brief.json")
}

pub fn load_brief(dir: &PathBuf) -> Option<PostCallBrief> {
    fs::read_to_string(brief_path(dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn save_brief(dir: &PathBuf, brief: &PostCallBrief) -> Result<()> {
    fs::write(
        brief_path(dir),
        serde_json::to_string_pretty(brief).context("serialize brief")?,
    )
    .context("write brief")
}

pub fn calls_dir(base: &PathBuf) -> Result<PathBuf> {
    let dir = base.join("calls");
    fs::create_dir_all(&dir).context("create calls folder")?;
    Ok(dir)
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Collision-safe id: millisecond clock + a per-process counter.
pub fn unique_id(prefix: &str) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{now_ms:x}-{seq:04x}")
}

pub fn new_call_id() -> (String, u64) {
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (unique_id("call"), started_unix)
}

pub fn spawn_tee(src: Receiver<Vec<u8>>, dg: Sender<Vec<u8>>, rec: SyncSender<Vec<u8>>) {
    thread::spawn(move || {
        let mut dg_open = true;
        let mut rec_open = true;
        while let Ok(chunk) = src.recv() {
            // Keep the recording and transcription paths independent. A failed
            // Deepgram stream must never stop local audio from being saved.
            if rec_open && rec.send(chunk.clone()).is_err() {
                rec_open = false;
            }
            if dg_open && dg.send(chunk).is_err() {
                dg_open = false;
            }
            if !rec_open && !dg_open {
                break;
            }
        }
    });
}

pub fn spawn_wav_mixer(
    you_rx: Receiver<Vec<u8>>,
    them_rx: Receiver<Vec<u8>>,
    wav_path: PathBuf,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("sonic-wav".into())
        .spawn(move || {
            if let Err(err) = mix_to_wav(you_rx, them_rx, wav_path, stop) {
                eprintln!("wav recorder: {err:#}");
            }
        })
        .expect("spawn wav mixer")
}

fn mix_to_wav(
    you_rx: Receiver<Vec<u8>>,
    them_rx: Receiver<Vec<u8>>,
    wav_path: PathBuf,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut file = BufWriter::new(File::create(&wav_path).context("create wav")?);
    file.write_all(&wav_header(0))?;
    let mut you_q: VecDeque<u8> = VecDeque::new();
    let mut them_q: VecDeque<u8> = VecDeque::new();
    let mut data_bytes: u32 = 0;
    let mut you_open = true;
    let mut them_open = true;
    let started = Instant::now();
    let mut last_checkpoint = Instant::now();

    while !stop.load(Ordering::Relaxed) || !you_q.is_empty() || !them_q.is_empty() || you_open || them_open
    {
        if you_open {
            match you_rx.try_recv() {
                Ok(chunk) => you_q.extend(chunk),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => you_open = false,
            }
        }
        if them_open {
            match them_rx.try_recv() {
                Ok(chunk) => them_q.extend(chunk),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => them_open = false,
            }
        }

        if you_q.len() >= FRAME_BYTES || them_q.len() >= FRAME_BYTES {
            let stereo = take_stereo_frame(&mut you_q, &mut them_q);
            if !stereo.is_empty() {
                file.write_all(&stereo)?;
                data_bytes += stereo.len() as u32;
                if last_checkpoint.elapsed() >= Duration::from_secs(2) {
                    checkpoint_wav(&mut file, data_bytes)?;
                    last_checkpoint = Instant::now();
                }
            }
        } else if stop.load(Ordering::Relaxed) && !you_open && !them_open {
            break;
        } else if started.elapsed() > Duration::from_secs(60 * 60 * 3) {
            break;
        } else {
            thread::sleep(Duration::from_millis(12));
        }
    }

    checkpoint_wav(&mut file, data_bytes)?;
    let mut raw = file.into_inner().context("flush wav")?;
    raw.flush()?;
    Ok(())
}

/// Flush recorded samples and rewrite the WAV header so the file is readable
/// even if the app crashes or the machine loses power mid-call.
fn checkpoint_wav(file: &mut BufWriter<File>, data_bytes: u32) -> Result<()> {
    file.flush()?;
    let raw = file.get_mut();
    raw.seek(SeekFrom::Start(0))?;
    raw.write_all(&wav_header(data_bytes))?;
    raw.seek(SeekFrom::Start(44 + u64::from(data_bytes)))?;
    Ok(())
}

fn take_stereo_frame(you: &mut VecDeque<u8>, them: &mut VecDeque<u8>) -> Vec<u8> {
    let samples = FRAME_BYTES / 2;
    let mut out = Vec::with_capacity(samples * 4);
    for _ in 0..samples {
        let y0 = you.pop_front().unwrap_or(0);
        let y1 = you.pop_front().unwrap_or(0);
        let t0 = them.pop_front().unwrap_or(0);
        let t1 = them.pop_front().unwrap_or(0);
        out.extend_from_slice(&[y0, y1, t0, t1]);
    }
    out
}

fn wav_header(data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    let chunk = 36 + data_bytes;
    h[4..8].copy_from_slice(&chunk.to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // pcm
    h[22..24].copy_from_slice(&2u16.to_le_bytes()); // stereo
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * 2 * 2;
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&4u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// Writes the session to disk as it happens, so a crash or a hard quit
/// still leaves a complete, readable call.
pub struct CallSink {
    pub dir: PathBuf,
    pub id: String,
    pub started_unix: u64,
    pub provider: String,
    pub model: String,
}

impl CallSink {
    pub fn flush(&self, entries: &[Entry]) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let duration_secs = now.saturating_sub(self.started_unix);
        let record = CallRecord {
            id: self.id.clone(),
            started_unix: self.started_unix,
            ended_unix: now,
            duration_secs,
            provider: self.provider.clone(),
            model: self.model.clone(),
            entries: entries.to_vec(),
        };
        let meta = CallMeta {
            id: self.id.clone(),
            started_unix: self.started_unix,
            duration_secs,
            line_count: entries.iter().filter(|e| e.role != "coach").count(),
            preview: entries
                .iter()
                .find(|e| e.role == "them")
                .or_else(|| entries.iter().find(|e| e.role != "coach"))
                .map(|e| e.text.clone())
                .unwrap_or_default(),
        };
        write_transcript(&self.dir, &record)?;
        write_meta(&self.dir, &meta)?;
        Ok(())
    }
}

pub fn write_transcript(dir: &PathBuf, record: &CallRecord) -> Result<()> {
    let mut md = format!(
        "# Call transcript\n\nSession id: {}\nYou = mic. Prospect = speakers. Ghost Coach = AI suggestion.\n\n",
        record.id
    );
    for entry in &record.entries {
        let who = match entry.role.as_str() {
            "you" => "You",
            "coach" => "Ghost Coach",
            _ => "Prospect",
        };
        if entry.role == "coach" && !entry.intent.is_empty() {
            md.push_str(&format!("**{who} ({}):** {}\n\n", entry.intent, entry.text));
        } else {
            md.push_str(&format!("**{who}:** {}\n\n", entry.text));
        }
    }
    fs::write(dir.join("transcript.md"), md).context("write transcript")?;
    fs::write(
        dir.join("session.json"),
        serde_json::to_string_pretty(record).unwrap_or_else(|_| "{}".into()),
    )
    .context("write session json")?;
    Ok(())
}

pub fn write_meta(dir: &PathBuf, meta: &CallMeta) -> Result<()> {
    fs::write(dir.join("meta.json"), serde_json::to_string_pretty(meta)?)
        .context("write call meta")?;
    Ok(())
}

pub fn list_calls(base: &PathBuf) -> Result<Vec<CallMeta>> {
    let dir = calls_dir(base)?;
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let meta_path = entry.path().join("meta.json");
            if let Ok(raw) = fs::read_to_string(meta_path) {
                if let Ok(meta) = serde_json::from_str::<CallMeta>(&raw) {
                    out.push(meta);
                }
            }
        }
    }
    out.sort_by(|a, b| b.started_unix.cmp(&a.started_unix));
    out.retain(|meta| meta.line_count > 0 || meta.duration_secs > 0);
    Ok(out)
}

pub fn load_call(base: &PathBuf, id: &str) -> Result<CallDetail> {
    let dir = calls_dir(base)?.join(id);
    let meta: CallMeta = serde_json::from_str(&fs::read_to_string(dir.join("meta.json"))?)?;
    let transcript = fs::read_to_string(dir.join("transcript.md")).unwrap_or_default();
    let entries = fs::read_to_string(dir.join("session.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<CallRecord>(&raw).ok())
        .map(|record| record.entries)
        .unwrap_or_default();
    Ok(CallDetail {
        meta,
        transcript,
        entries,
        brief: load_brief(&dir).unwrap_or_default(),
    })
}

pub fn call_dir(base: &PathBuf, id: &str) -> Result<PathBuf> {
    let dir = calls_dir(base)?.join(id);
    anyhow::ensure!(
        dir.exists(),
        "No saved call with that id."
    );
    Ok(dir)
}

pub fn call_wav_path(base: &PathBuf, id: &str) -> PathBuf {
    let mut p = base.clone();
    p.push("calls");
    p.push(id);
    p.push("call.wav");
    p
}

#[cfg(test)]
mod brief_tests {
    use super::{
        checkpoint_wav, load_brief, save_brief, spawn_tee, spawn_wav_mixer, wav_header,
        PostCallBrief, FRAME_BYTES,
    };
    use std::fs::{self, File};
    use std::io::{BufWriter, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn brief_round_trips_through_disk() {
        let dir = PathBuf::from(std::env::temp_dir().join(format!(
            "ghost-coach-test-{}",
            std::process::id()
        )));
        fs::create_dir_all(&dir).unwrap();
        let brief = PostCallBrief {
            about: "Weekly sync about the website rebuild".into(),
            key_points: vec!["Launch moved to Friday".into(), "Budget confirmed".into()],
            next_step: "Send the updated quote".into(),
            user_note: "They prefer email".into(),
            generated: true,
        };
        save_brief(&dir, &brief).unwrap();
        let loaded = load_brief(&dir).unwrap();
        assert_eq!(loaded.about, brief.about);
        assert_eq!(loaded.key_points, brief.key_points);
        assert_eq!(loaded.next_step, brief.next_step);
        assert_eq!(loaded.user_note, brief.user_note);
        assert!(loaded.generated);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_brief_loads_as_none_and_defaults_are_empty() {
        let dir = PathBuf::from(std::env::temp_dir().join("ghost-coach-test-missing"));
        fs::remove_dir_all(&dir).ok();
        assert!(load_brief(&dir).is_none());
        let brief = PostCallBrief::default();
        assert!(brief.about.is_empty());
        assert!(brief.key_points.is_empty());
    }

    #[test]
    fn recording_continues_when_transcription_channel_fails() {
        let (src_tx, src_rx) = mpsc::channel::<Vec<u8>>();
        let (dg_tx, dg_rx) = mpsc::channel::<Vec<u8>>();
        let (rec_tx, rec_rx) = mpsc::sync_channel::<Vec<u8>>(4);
        drop(dg_rx);
        spawn_tee(src_rx, dg_tx, rec_tx);

        src_tx.send(vec![1, 2, 3]).unwrap();
        src_tx.send(vec![4, 5, 6]).unwrap();
        drop(src_tx);

        assert_eq!(
            rec_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            rec_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![4, 5, 6]
        );
    }

    #[test]
    fn wav_mixer_writes_both_sides_and_a_valid_header() {
        let path = PathBuf::from(std::env::temp_dir().join(format!(
            "ghost-coach-wav-test-{}.wav",
            std::process::id()
        )));
        fs::remove_file(&path).ok();
        let (you_tx, you_rx) = mpsc::channel::<Vec<u8>>();
        let (them_tx, them_rx) = mpsc::channel::<Vec<u8>>();
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_wav_mixer(you_rx, them_rx, path.clone(), stop.clone());

        you_tx.send(vec![1u8; FRAME_BYTES]).unwrap();
        them_tx.send(vec![2u8; FRAME_BYTES]).unwrap();
        drop(you_tx);
        drop(them_tx);
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        let wav = fs::read(&path).unwrap();
        let data_bytes = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(data_bytes, FRAME_BYTES * 2);
        assert_eq!(wav.len(), 44 + data_bytes);
        assert_eq!(&wav[44..48], &[1, 1, 2, 2]);
        fs::remove_file(path).ok();
    }

    #[test]
    fn interrupted_recording_still_has_a_readable_header() {
        let path = PathBuf::from(std::env::temp_dir().join(format!(
            "ghost-coach-checkpoint-test-{}.wav",
            std::process::id()
        )));
        let mut file = BufWriter::new(File::create(&path).unwrap());
        file.write_all(&wav_header(0)).unwrap();
        file.write_all(&[9u8; 8]).unwrap();
        checkpoint_wav(&mut file, 8).unwrap();
        drop(file);

        let wav = fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(wav.len(), 52);
        fs::remove_file(path).ok();
    }
}
