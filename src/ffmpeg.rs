//! Запуск ffmpeg/ffprobe: сведения о файлах, миниатюры, форма звуковой волны.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::model::MediaKind;

/// Сколько значений формы волны приходится на секунду звука.
pub const WAVE_RATE: f64 = 50.0;

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"];
pub const VIDEO_EXT: &[&str] = &["mp4", "mov", "mkv", "avi", "webm", "m4v", "wmv", "flv", "ts", "mts", "gif"];
pub const AUDIO_EXT: &[&str] = &["mp3", "wav", "aac", "m4a", "flac", "ogg", "opus", "wma"];
pub const PHOTO_EXT: &[&str] = IMAGE_EXT;

fn find_tool(name: &str) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            for candidate in [dir.join(&exe), dir.join("ffmpeg").join(&exe)] {
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from(name)
}

pub fn ffmpeg_path() -> &'static Path {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| find_tool("ffmpeg"))
}

pub fn ffprobe_path() -> &'static Path {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| find_tool("ffprobe"))
}

/// Команда без окна консоли и без ввода.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

pub fn ffmpeg() -> Command {
    command(ffmpeg_path())
}

/// Ключ, которым ffmpeg читает граф фильтров из файла: с версии 7 прежний ключ устарел.
pub fn filter_script_flag() -> &'static str {
    static OLD: OnceLock<bool> = OnceLock::new();
    let old = *OLD.get_or_init(|| {
        let out = ffmpeg().arg("-version").stderr(Stdio::null()).output().ok();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        // «ffmpeg version 6.1.1 …»; сборки из git начинаются не с номера и считаются новыми.
        text.split_whitespace()
            .nth(2)
            .and_then(|v| v.trim_start_matches('n').split('.').next()?.parse::<u32>().ok())
            .map_or(false, |major| major < 7)
    });
    if old {
        "-filter_complex_script"
    } else {
        "-/filter_complex"
    }
}

/// Видеокарта, которой можно кодировать видео.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gpu {
    Nvidia,
    Intel,
    Amd,
}

impl Gpu {
    pub fn label(self) -> &'static str {
        match self {
            Gpu::Nvidia => "NVIDIA",
            Gpu::Intel => "Intel",
            Gpu::Amd => "AMD",
        }
    }

    /// Имена кодировщиков H.264 и H.265 в ffmpeg.
    pub fn encoders(self) -> (&'static str, &'static str) {
        match self {
            Gpu::Nvidia => ("h264_nvenc", "hevc_nvenc"),
            Gpu::Intel => ("h264_qsv", "hevc_qsv"),
            Gpu::Amd => ("h264_amf", "hevc_amf"),
        }
    }
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

#[cfg_attr(test, allow(dead_code))]
/// Ищет в фоне видеокарту для кодирования: каждой пробует закодировать крошечный ролик,
/// потому что кодировщик в сборке ffmpeg ещё не значит, что есть подходящее железо.
pub fn detect_gpu() {
    std::thread::spawn(|| {
        let works = |gpu: &Gpu| {
            ffmpeg()
                .args(["-v", "error", "-f", "lavfi", "-i", "color=c=black:s=256x144:r=30:d=0.2", "-c:v", gpu.encoders().0])
                .args(["-f", "null", "-"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_or(false, |s| s.success())
        };
        let _ = GPU.set([Gpu::Nvidia, Gpu::Intel, Gpu::Amd].into_iter().find(works));
    });
}

/// Видеокарта для кодирования; пока поиск не закончился, её нет.
pub fn gpu() -> Option<Gpu> {
    GPU.get().copied().flatten()
}

/// Доступны ли ffmpeg и ffprobe.
pub fn available() -> bool {
    let ok = |p: &Path| {
        command(p)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    ok(ffmpeg_path()) && ok(ffprobe_path())
}

pub struct Probe {
    pub kind: MediaKind,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

fn parse_ratio(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let a: f64 = a.parse().unwrap_or(0.0);
            let b: f64 = b.parse().unwrap_or(0.0);
            if b > 0.0 {
                a / b
            } else {
                0.0
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn probe(path: &Path) -> Result<Probe, String> {
    let out = command(ffprobe_path())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|e| format!("не удалось запустить ffprobe: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if msg.is_empty() { "файл не распознан".to_owned() } else { msg });
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;

    let mut width = 0u32;
    let mut height = 0u32;
    let mut fps = 0.0f64;
    let mut has_video = false;
    let mut has_audio = false;
    let mut stream_duration = 0.0f64;

    for s in v["streams"].as_array().into_iter().flatten() {
        let dur = s["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);
        match s["codec_type"].as_str() {
            Some("video") => {
                // Обложка внутри аудиофайла — не видео.
                if s["disposition"]["attached_pic"].as_i64() == Some(1) || has_video {
                    continue;
                }
                has_video = true;
                width = s["width"].as_u64().unwrap_or(0) as u32;
                height = s["height"].as_u64().unwrap_or(0) as u32;
                fps = parse_ratio(s["avg_frame_rate"].as_str().unwrap_or("0"));
                if fps <= 0.0 || !fps.is_finite() {
                    fps = parse_ratio(s["r_frame_rate"].as_str().unwrap_or("0"));
                }
                let mut rotation = s["tags"]["rotate"]
                    .as_str()
                    .and_then(|r| r.parse::<f64>().ok())
                    .unwrap_or(0.0);
                for sd in s["side_data_list"].as_array().into_iter().flatten() {
                    if let Some(r) = sd["rotation"].as_f64() {
                        rotation = r;
                    }
                }
                if (rotation.abs() as i64) % 180 == 90 {
                    std::mem::swap(&mut width, &mut height);
                }
                stream_duration = stream_duration.max(dur);
            }
            Some("audio") => {
                has_audio = true;
                stream_duration = stream_duration.max(dur);
            }
            _ => {}
        }
    }

    let mut duration = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    if duration <= 0.0 {
        duration = stream_duration;
    }

    let is_image = IMAGE_EXT.contains(&extension(path).as_str());
    let kind = if has_video && is_image {
        MediaKind::Image
    } else if has_video {
        MediaKind::Video
    } else if has_audio {
        MediaKind::Audio
    } else {
        return Err("в файле нет видео или звука".to_owned());
    };
    if kind == MediaKind::Image {
        duration = 0.0;
        has_audio = false;
        fps = 0.0;
    } else if duration <= 0.0 {
        return Err("не удалось определить длительность".to_owned());
    }
    if kind != MediaKind::Audio && (width == 0 || height == 0) {
        return Err("не удалось определить размер кадра".to_owned());
    }
    Ok(Probe { kind, duration, width, height, fps, has_audio })
}

/// Один кадр в RGBA заданного размера.
pub fn frame_rgba(path: &Path, t: f64, w: u32, h: u32) -> Option<Vec<u8>> {
    let out = ffmpeg()
        .args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={w}:{h}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.stdout.len() == (w as usize) * (h as usize) * 4).then_some(out.stdout)
}

/// Пики громкости: `WAVE_RATE` значений 0..1 на секунду.
pub fn waveform(path: &Path) -> Option<Vec<f32>> {
    const RATE: usize = 8000;
    let out = ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let block = (RATE as f64 / WAVE_RATE) as usize;
    let mut peaks = Vec::with_capacity(out.stdout.len() / 2 / block + 1);
    for chunk in out.stdout.chunks(block * 2) {
        let mut peak = 0i32;
        for s in chunk.chunks_exact(2) {
            let v = i16::from_le_bytes([s[0], s[1]]) as i32;
            peak = peak.max(v.abs());
        }
        peaks.push(peak as f32 / 32768.0);
    }
    (!peaks.is_empty()).then_some(peaks)
}

/// Чётное число не меньше 2 — размеры кадра для кодеков.
pub fn even(v: f32) -> u32 {
    (((v / 2.0).round() as i64).max(1) * 2) as u32
}
