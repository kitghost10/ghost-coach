use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(target_os = "windows")]
use std::collections::VecDeque;
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use wasapi::{get_default_device, initialize_mta, Direction, SampleType, StreamMode};

pub const TARGET_HZ: u32 = 24_000;
pub const OUT_FRAME: usize = (TARGET_HZ / 50) as usize; // 20 ms at 24 kHz

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ear {
    You,
    Them,
}

pub fn spawn_ear(
    ear: Ear,
    stop: Arc<AtomicBool>,
    pcm_tx: Option<Sender<Vec<u8>>>,
    level_tx: SyncSender<(Ear, f32)>,
    error_tx: SyncSender<(Ear, String)>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(format!("sonic-{ear:?}"))
        .spawn(move || {
            if let Err(err) = capture_loop(ear, stop, pcm_tx, level_tx) {
                eprintln!("audio {ear:?}: {err:#}");
                let _ = error_tx.try_send((ear, format!("{err:#}")));
            }
        })
        .expect("spawn audio thread")
}

fn capture_loop(
    ear: Ear,
    stop: Arc<AtomicBool>,
    pcm_tx: Option<Sender<Vec<u8>>>,
    level_tx: SyncSender<(Ear, f32)>,
) -> Result<()> {
    if ear == Ear::Them {
        #[cfg(target_os = "linux")]
        {
            return capture_linux_monitor(stop, pcm_tx, level_tx);
        }
        #[cfg(target_os = "windows")]
        {
            return capture_windows_loopback(stop, pcm_tx, level_tx);
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        anyhow::bail!("System-audio capture is not supported on this platform.");
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no default input device")?;

    let config = device
        .default_input_config()
        .context("no default input config")?;
    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let channels = config.channels as usize;
    anyhow::ensure!(channels > 0, "input device has no channels");
    let sample_hz = config.sample_rate.0;

    let mut resampler = StreamResampler::new(sample_hz, TARGET_HZ);
    let mut pcm_acc: Vec<f32> = Vec::new();
    let mut last_level = Instant::now();
    let first_channel_only = true;

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                process_samples(
                    data,
                    channels,
                    first_channel_only,
                    ear,
                    &mut resampler,
                    &mut pcm_acc,
                    &mut last_level,
                    &pcm_tx,
                    &level_tx,
                );
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                process_samples(
                    &f32_data,
                    channels,
                    first_channel_only,
                    ear,
                    &mut resampler,
                    &mut pcm_acc,
                    &mut last_level,
                    &pcm_tx,
                    &level_tx,
                );
            },
            err_fn,
            None,
        )?,
        _ => anyhow::bail!("unsupported sample format"),
    };

    stream.play()?;

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn capture_linux_monitor(
    stop: Arc<AtomicBool>,
    pcm_tx: Option<Sender<Vec<u8>>>,
    level_tx: SyncSender<(Ear, f32)>,
) -> Result<()> {
    let source = linux_monitor_source()?;
    let device_arg = format!("--device={source}");
    let mut child = Command::new("parec")
        .args([
            "--raw",
            &device_arg,
            "--format=float32le",
            "--rate=48000",
            "--channels=2",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("parec is required for Linux system audio; install pulseaudio-utils")?;
    let mut stdout = child
        .stdout
        .take()
        .context("open PulseAudio capture stream")?;
    let mut resampler = StreamResampler::new(48_000, TARGET_HZ);
    let mut pcm_acc = Vec::new();
    let mut last_level = Instant::now();
    let mut bytes = [0u8; 8192];
    let mut pending = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        let count = match stdout.read(&mut bytes) {
            Ok(count) => count,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("read system-audio monitor");
            }
        };
        if count == 0 {
            let status = child.wait().context("wait for parec")?;
            anyhow::bail!(
                "PulseAudio monitor '{source}' stopped unexpectedly (parec status: {status})."
            );
        }
        pending.extend_from_slice(&bytes[..count]);
        // Keep complete stereo frames so an odd-sized pipe read cannot shift sample alignment.
        let complete_len = pending.len() / 8 * 8;
        if complete_len == 0 {
            continue;
        }
        let samples: Vec<f32> = pending[..complete_len]
            .chunks_exact(4)
            .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
            .collect();
        pending.drain(..complete_len);
        process_samples(
            &samples,
            2,
            false,
            Ear::Them,
            &mut resampler,
            &mut pcm_acc,
            &mut last_level,
            &pcm_tx,
            &level_tx,
        );
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_monitor_source() -> Result<String> {
    let sink = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .context("pactl is required for Linux system audio; install pulseaudio-utils")?;
    anyhow::ensure!(
        sink.status.success(),
        "could not identify the default audio output"
    );
    let sink_name = String::from_utf8_lossy(&sink.stdout).trim().to_string();
    anyhow::ensure!(
        !sink_name.is_empty(),
        "no default audio output is configured"
    );

    let sources = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .context("list PulseAudio monitor sources")?;
    anyhow::ensure!(
        sources.status.success(),
        "could not list audio capture sources"
    );
    let listed = String::from_utf8_lossy(&sources.stdout);
    select_linux_monitor(&sink_name, &listed).context(
        "no playback monitor source found; check that PulseAudio or PipeWire Pulse is running",
    )
}

#[cfg(target_os = "linux")]
fn select_linux_monitor(default_sink: &str, sources: &str) -> Option<String> {
    let preferred = format!("{default_sink}.monitor");
    let names = sources
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect::<Vec<_>>();
    names
        .iter()
        .find(|name| **name == preferred)
        .or_else(|| names.iter().find(|name| name.ends_with(".monitor")))
        .map(|name| (*name).to_string())
}

#[cfg(target_os = "windows")]
fn capture_windows_loopback(
    stop: Arc<AtomicBool>,
    pcm_tx: Option<Sender<Vec<u8>>>,
    level_tx: SyncSender<(Ear, f32)>,
) -> Result<()> {
    let hr = initialize_mta();
    anyhow::ensure!(hr.is_ok(), "COM MTA failed: {hr:?}");

    let device = get_default_device(&Direction::Render).context("default playback device")?;
    let mut client = device.get_iaudioclient()?;
    let mix = client.get_mixformat().context("playback mix format")?;
    let channels = mix.get_nchannels() as usize;
    let sample_hz = mix.get_samplespersec();
    let bits = mix.get_bitspersample();
    let sample_type = mix.get_subformat().unwrap_or(SampleType::Float);
    let bytes_per_frame = mix.get_blockalign() as usize;
    anyhow::ensure!(
        channels > 0 && bytes_per_frame > 0,
        "invalid playback mix format"
    );
    let mode = StreamMode::PollingShared {
        autoconvert: true,
        buffer_duration_hns: 5_000_000,
    };
    client.initialize_client(&mix, &Direction::Capture, &mode)?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;

    let mut queue = VecDeque::new();
    let mut resampler = StreamResampler::new(sample_hz, TARGET_HZ);
    let mut pcm_acc = Vec::new();
    let mut last_level = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(8));
        capture.read_from_device_to_deque(&mut queue)?;
        if queue.len() < bytes_per_frame {
            continue;
        }
        let mut raw: Vec<u8> = queue.drain(..).collect();
        let leftover = raw.len() % bytes_per_frame;
        if leftover > 0 {
            queue.extend(raw.split_off(raw.len() - leftover));
        }
        let samples = decode_windows_mono(&raw, channels, bits, sample_type)?;
        process_samples(
            &samples,
            1,
            true,
            Ear::Them,
            &mut resampler,
            &mut pcm_acc,
            &mut last_level,
            &pcm_tx,
            &level_tx,
        );
    }
    let _ = client.stop_stream();
    Ok(())
}

#[cfg(target_os = "windows")]
fn decode_windows_mono(
    bytes: &[u8],
    channels: usize,
    bits: u16,
    kind: SampleType,
) -> Result<Vec<f32>> {
    let sample_bytes = (bits / 8) as usize;
    anyhow::ensure!(sample_bytes > 0, "bad sample width");
    let frame_bytes = sample_bytes * channels;
    anyhow::ensure!(frame_bytes > 0, "bad frame");
    let mut out = Vec::with_capacity(bytes.len() / frame_bytes);
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0f32;
        for channel in 0..channels {
            let sample = &frame[channel * sample_bytes..(channel + 1) * sample_bytes];
            let value = match (kind, sample_bytes) {
                (SampleType::Float, 4) => f32::from_le_bytes(sample.try_into().unwrap()),
                (SampleType::Float, 8) => f64::from_le_bytes(sample.try_into().unwrap()) as f32,
                (SampleType::Int, 2) => {
                    i16::from_le_bytes(sample.try_into().unwrap()) as f32 / 32768.0
                }
                (SampleType::Int, 4) => {
                    i32::from_le_bytes(sample.try_into().unwrap()) as f32 / 2_147_483_648.0
                }
                _ => anyhow::bail!("unsupported playback sample format {kind} {bits}-bit"),
            };
            sum += value;
        }
        out.push((sum / channels as f32).clamp(-1.0, 1.0));
    }
    Ok(out)
}

fn process_samples(
    samples: &[f32],
    channels: usize,
    first_channel_only: bool,
    ear: Ear,
    resampler: &mut StreamResampler,
    pcm_acc: &mut Vec<f32>,
    last_level: &mut Instant,
    pcm_tx: &Option<Sender<Vec<u8>>>,
    level_tx: &SyncSender<(Ear, f32)>,
) {
    if samples.is_empty() {
        return;
    }

    let mut mono = Vec::with_capacity(samples.len() / channels);
    for frame in samples.chunks_exact(channels) {
        let value = if first_channel_only || channels == 1 {
            frame[0]
        } else {
            frame.iter().sum::<f32>() / channels as f32
        };
        mono.push(value.clamp(-1.0, 1.0));
    }

    if last_level.elapsed() >= Duration::from_millis(80) {
        let _ = level_tx.try_send((ear, rms(&mono)));
        *last_level = Instant::now();
    }

    if let Some(tx) = pcm_tx {
        pcm_acc.extend_from_slice(&resampler.push(&mono));
        while pcm_acc.len() >= OUT_FRAME {
            let chunk: Vec<f32> = pcm_acc.drain(..OUT_FRAME).collect();
            let pcm16 = f32_to_pcm16(&chunk);
            // The capture path is unbounded so a slow or stalled transcription
            // stream can never make us discard recorded call audio.
            if tx.send(pcm16).is_err() {
                break;
            }
        }
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt().clamp(0.0, 1.0)
}

struct StreamResampler {
    from_hz: u32,
    to_hz: u32,
    pending: Vec<f32>,
    phase: f64,
}

impl StreamResampler {
    fn new(from_hz: u32, to_hz: u32) -> Self {
        Self {
            from_hz: from_hz.max(1),
            to_hz: to_hz.max(1),
            pending: Vec::new(),
            phase: 0.0,
        }
    }

    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() || self.from_hz == 0 {
            return Vec::new();
        }
        if self.from_hz == self.to_hz {
            return input.to_vec();
        }
        self.pending.extend_from_slice(input);
        if self.pending.len() < 2 {
            return Vec::new();
        }
        let step = self.from_hz as f64 / self.to_hz as f64;
        let mut out = Vec::new();
        loop {
            let i0 = self.phase.floor() as usize;
            if i0 + 1 >= self.pending.len() {
                break;
            }
            let frac = (self.phase - i0 as f64) as f32;
            out.push(self.pending[i0] * (1.0 - frac) + self.pending[i0 + 1] * frac);
            self.phase += step;
        }
        let drop = self.phase.floor() as usize;
        if drop > 0 {
            let drop = drop.min(self.pending.len().saturating_sub(1));
            self.pending.drain(..drop);
            self.phase -= drop as f64;
            if self.phase < 0.0 {
                self.phase = 0.0;
            }
        }
        out
    }
}

fn f32_to_pcm16(input: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 2);
    for s in input {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::select_linux_monitor;

    #[test]
    fn selects_monitor_for_default_sink() {
        let sources = "1 alsa_input.mic module-alsa-card.c s16le 2ch 48000Hz RUNNING\n\
                       2 alsa_output.usb.monitor module-alsa-card.c s16le 2ch 48000Hz IDLE";
        assert_eq!(
            select_linux_monitor("alsa_output.usb", sources).as_deref(),
            Some("alsa_output.usb.monitor")
        );
    }

    #[test]
    fn falls_back_to_an_available_monitor() {
        let sources = "1 alsa_input.mic module-alsa-card.c s16le 2ch 48000Hz RUNNING\n\
                       2 alsa_output.other.monitor module-alsa-card.c s16le 2ch 48000Hz IDLE";
        assert_eq!(
            select_linux_monitor("missing_sink", sources).as_deref(),
            Some("alsa_output.other.monitor")
        );
    }

    #[test]
    fn does_not_use_a_microphone_as_system_audio() {
        let sources = "1 alsa_input.mic module-alsa-card.c s16le 2ch 48000Hz RUNNING";
        assert_eq!(select_linux_monitor("missing_sink", sources), None);
    }
}
