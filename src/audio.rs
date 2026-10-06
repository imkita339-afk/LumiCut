//! Звук предпросмотра: ffmpeg отдаёт PCM, микшер складывает клипы, cpal выводит.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::ffmpeg;
use crate::keys::Prop;
use crate::model::{Id, MediaKind, Project, TrackKind};

struct Shared {
    /// Стерео-сэмплы, готовые к выводу.
    ring: Mutex<VecDeque<f32>>,
    /// Сколько кадров звука реально проиграно с начала воспроизведения.
    played: AtomicU64,
    playing: AtomicBool,
    /// Номер запуска: устаревший микшер перестаёт писать в буфер.
    generation: AtomicU64,
    /// Текущая громкость клипов (меняется на лету).
    volumes: Mutex<HashMap<Id, f32>>,
}

#[derive(Clone)]
struct Source {
    clip: Id,
    path: PathBuf,
    start: f64,
    offset: f64,
    duration: f64,
    speed: f64,
}

pub struct AudioEngine {
    stream: Option<cpal::Stream>,
    shared: Arc<Shared>,
    rate: u32,
    base: f64,
    running: bool,
}

fn on_error(e: cpal::StreamError) {
    eprintln!("звук: {e}");
}

fn fill<T: Copy>(sh: &Shared, data: &mut [T], channels: usize, conv: impl Fn(f32) -> T) {
    let silence = conv(0.0);
    if !sh.playing.load(Ordering::Relaxed) {
        data.iter_mut().for_each(|s| *s = silence);
        return;
    }
    let mut played = 0u64;
    {
        let mut ring = sh.ring.lock().unwrap();
        for frame in data.chunks_mut(channels.max(1)) {
            if ring.len() >= 2 {
                let l = ring.pop_front().unwrap_or(0.0);
                let r = ring.pop_front().unwrap_or(0.0);
                played += 1;
                if frame.len() == 1 {
                    frame[0] = conv((l + r) * 0.5);
                } else {
                    frame[0] = conv(l);
                    frame[1] = conv(r);
                    frame.iter_mut().skip(2).for_each(|s| *s = silence);
                }
            } else {
                frame.iter_mut().for_each(|s| *s = silence);
            }
        }
    }
    sh.played.fetch_add(played, Ordering::Relaxed);
}

impl AudioEngine {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            ring: Mutex::new(VecDeque::new()),
            played: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            volumes: Mutex::new(HashMap::new()),
        });
        let mut rate = 48_000;
        let mut stream = None;
        if let Some(device) = cpal::default_host().default_output_device() {
            if let Ok(cfg) = device.default_output_config() {
                let channels = cfg.channels() as usize;
                let format = cfg.sample_format();
                let config: cpal::StreamConfig = cfg.into();
                let sh = shared.clone();
                let built = match format {
                    cpal::SampleFormat::F32 => device.build_output_stream(
                        &config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| fill(&sh, data, channels, |s| s),
                        on_error,
                        None,
                    ),
                    cpal::SampleFormat::I16 => device.build_output_stream(
                        &config,
                        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                            fill(&sh, data, channels, |s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                        },
                        on_error,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_output_stream(
                        &config,
                        move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                            fill(&sh, data, channels, |s| ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16)
                        },
                        on_error,
                        None,
                    ),
                    _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
                };
                if let Ok(s) = built {
                    if s.play().is_ok() {
                        rate = config.sample_rate.0;
                        stream = Some(s);
                    }
                }
            }
        }
        Self { stream, shared, rate, base: 0.0, running: false }
    }

    /// Есть ли устройство вывода.
    pub fn available(&self) -> bool {
        self.stream.is_some()
    }

    /// Запускает звук с момента `t` таймлайна.
    pub fn play(&mut self, project: &Project, t: f64) {
        if self.stream.is_none() {
            return;
        }
        self.set_volumes(project, t);
        let sources = collect(project, t);
        let generation = {
            let mut ring = self.shared.ring.lock().unwrap();
            ring.clear();
            self.shared.played.store(0, Ordering::Relaxed);
            self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1
        };
        self.base = t;
        self.running = true;
        self.shared.playing.store(true, Ordering::Relaxed);
        let shared = self.shared.clone();
        let rate = self.rate;
        std::thread::spawn(move || mixer(shared, generation, sources, t, rate));
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.shared.playing.store(false, Ordering::Relaxed);
        let mut ring = self.shared.ring.lock().unwrap();
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        ring.clear();
    }

    /// Время таймлайна по звуковым часам, если звук идёт.
    pub fn clock(&self) -> Option<f64> {
        (self.running && self.stream.is_some())
            .then(|| self.base + self.shared.played.load(Ordering::Relaxed) as f64 / self.rate as f64)
    }

    /// Обновляет громкость клипов без перезапуска: она берётся для момента `t` таймлайна.
    pub fn set_volumes(&self, project: &Project, t: f64) {
        let mut map = self.shared.volumes.lock().unwrap();
        map.clear();
        for track in &project.tracks {
            for clip in &track.clips {
                map.insert(clip.id, if track.enabled { clip.value(Prop::Volume, t) } else { 0.0 });
            }
        }
    }
}

fn collect(project: &Project, from: f64) -> Vec<Source> {
    let mut out = Vec::new();
    for track in &project.tracks {
        for clip in &track.clips {
            if clip.end() <= from {
                continue;
            }
            let Some(media) = project.media(clip.media) else { continue };
            let audible = match track.kind {
                TrackKind::Audio => media.kind == MediaKind::Audio,
                TrackKind::Video => media.kind == MediaKind::Video && media.has_audio,
            };
            if audible {
                out.push(Source {
                    clip: clip.id,
                    path: media.path.clone(),
                    start: clip.start,
                    offset: clip.offset,
                    duration: clip.duration,
                    speed: clip.speed,
                });
            }
        }
    }
    out
}

struct Active {
    clip: Id,
    child: Child,
    out: ChildStdout,
    /// Границы клипа в кадрах звука от начала воспроизведения.
    end_frame: i64,
    next_frame: i64,
    done: bool,
}

impl Drop for Active {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn atempo(speed: f64) -> Option<String> {
    if (speed - 1.0).abs() < 1e-3 {
        None
    } else if speed < 0.5 {
        Some(format!("atempo=0.5,atempo={:.4}", speed / 0.5))
    } else {
        Some(format!("atempo={speed:.4}"))
    }
}

fn start_source(s: &Source, t0: f64, rate: u32) -> Option<Active> {
    let rate_f = rate as f64;
    let local = (t0 - s.start).max(0.0);
    let src_off = s.offset + local * s.speed;
    let src_dur = (s.duration - local) * s.speed;
    if src_dur <= 0.0 {
        return None;
    }
    let mut cmd = ffmpeg::ffmpeg();
    cmd.args(["-v", "error", "-ss", &format!("{src_off:.4}"), "-t", &format!("{src_dur:.4}"), "-i"])
        .arg(&s.path)
        .arg("-vn");
    if let Some(f) = atempo(s.speed) {
        cmd.args(["-af", &f]);
    }
    cmd.args(["-f", "f32le", "-ac", "2", "-ar", &rate.to_string(), "-"]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let out = child.stdout.take()?;
    let start_frame = ((s.start - t0) * rate_f).round() as i64;
    Some(Active {
        clip: s.clip,
        child,
        out,
        end_frame: ((s.start + s.duration - t0) * rate_f).round() as i64,
        next_frame: start_frame.max(0),
        done: false,
    })
}

fn read_full(out: &mut ChildStdout, buf: &mut [u8]) -> usize {
    let mut got = 0;
    while got < buf.len() {
        match out.read(&mut buf[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    got
}

fn mixer(shared: Arc<Shared>, generation: u64, mut pending: Vec<Source>, t0: f64, rate: u32) {
    const CHUNK: usize = 2048;
    let rate_f = rate as f64;
    let target = (rate_f * 0.15) as usize * 2;
    let mut pos: i64 = 0;
    let mut active: Vec<Active> = Vec::new();
    let mut bytes = vec![0u8; CHUNK * 8];
    let mut mix = vec![0f32; CHUNK * 2];

    loop {
        if shared.generation.load(Ordering::SeqCst) != generation {
            break;
        }
        if shared.ring.lock().unwrap().len() > target {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        let chunk_end = pos + CHUNK as i64;
        let t_end = t0 + chunk_end as f64 / rate_f;
        // Запускаем источники чуть заранее, чтобы ffmpeg успел стартовать.
        pending.retain(|s| {
            if s.start < t_end + 0.5 {
                if let Some(a) = start_source(s, t0, rate) {
                    active.push(a);
                }
                false
            } else {
                true
            }
        });

        mix.iter_mut().for_each(|s| *s = 0.0);
        let volumes = shared.volumes.lock().unwrap().clone();
        for a in active.iter_mut() {
            let from = a.next_frame.max(pos);
            let to = a.end_frame.min(chunk_end);
            if from >= to {
                if a.end_frame <= pos {
                    a.done = true;
                }
                continue;
            }
            let n = (to - from) as usize;
            let buf = &mut bytes[..n * 8];
            let frames = read_full(&mut a.out, buf) / 8;
            let vol = volumes.get(&a.clip).copied().unwrap_or(1.0);
            let off = (from - pos) as usize;
            for i in 0..frames {
                let b = &buf[i * 8..i * 8 + 8];
                let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                mix[(off + i) * 2] += l * vol;
                mix[(off + i) * 2 + 1] += r * vol;
            }
            if frames < n {
                a.done = true;
            }
            a.next_frame = to;
        }
        active.retain(|a| !a.done);
        mix.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));

        let mut ring = shared.ring.lock().unwrap();
        if shared.generation.load(Ordering::SeqCst) != generation {
            break;
        }
        ring.extend(mix.iter().copied());
        drop(ring);
        pos = chunk_end;
    }
}
