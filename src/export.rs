//! Экспорт: таймлайн превращается в граф фильтров ffmpeg.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::effects::{self, Effect};
use crate::ffmpeg::{self, Gpu};
use crate::keys::Prop;
use crate::model::{fit_size, Clip, MediaKind, Project, TrackKind};
use crate::text::{self, Raster};
use crate::transition::{TransitionKind, ZOOM_IN, ZOOM_OUT};
use crate::vfx::{self, Blend};

/// Командная строка Windows не длиннее 32 767 знаков: граф фильтров длиннее этого порога
/// передаётся ffmpeg через файл.
const MAX_INLINE_ARGS: usize = 24_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Codec {
    H264,
    H265,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    High,
    Standard,
    Compact,
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub path: PathBuf,
    /// Высота кадра; `None` — как в проекте.
    pub height: Option<u32>,
    /// Частота кадров; `None` — как в проекте.
    pub fps: Option<f64>,
    pub codec: Codec,
    pub quality: Quality,
    /// Кодировать видеокартой, если она это умеет.
    pub accel: bool,
    /// Видеокарта, которой идёт кодирование; `None` — процессор.
    pub gpu: Option<Gpu>,
}

impl ExportSettings {
    pub fn new(path: PathBuf) -> Self {
        Self { path, height: None, fps: None, codec: Codec::H264, quality: Quality::Standard, accel: true, gpu: None }
    }
}

/// Параметры кодировщика видео: на видеокарте кодирование в несколько раз быстрее.
fn encoder(s: &ExportSettings) -> Vec<String> {
    let crf = match s.quality {
        Quality::High => 17,
        Quality::Standard => 21,
        Quality::Compact => 26,
    };
    let h264 = s.codec == Codec::H264;
    let q = if h264 { crf } else { crf + 4 };
    let name = |gpu: Gpu| if h264 { gpu.encoders().0 } else { gpu.encoders().1 };
    // У аппаратных кодировщиков шкала качества чуть грубее, чем у программных.
    let mut args: Vec<String> = match s.gpu {
        None => vec!["-c:v".into(), if h264 { "libx264" } else { "libx265" }.into(), "-preset".into(), "medium".into(), "-crf".into(), q.to_string()],
        Some(gpu @ Gpu::Nvidia) => {
            let quality = (q + 3).to_string();
            ["-c:v", name(gpu), "-preset", "p5", "-rc", "vbr", "-cq", &quality, "-b:v", "0"].map(String::from).to_vec()
        }
        Some(gpu @ Gpu::Intel) => {
            let quality = (q + 2).to_string();
            ["-c:v", name(gpu), "-preset", "medium", "-global_quality", &quality].map(String::from).to_vec()
        }
        Some(gpu @ Gpu::Amd) => {
            let quality = (q + 2).to_string();
            ["-c:v", name(gpu), "-quality", "balanced", "-rc", "cqp", "-qp_i", &quality, "-qp_p", &quality, "-qp_b", &quality]
                .map(String::from)
                .to_vec()
        }
    };
    if !h264 {
        args.extend(["-tag:v".to_owned(), "hvc1".to_owned()]);
    }
    args
}

/// Размер выходного кадра.
pub fn output_size(project: &Project, s: &ExportSettings) -> (u32, u32) {
    match s.height {
        Some(h) if h != project.height => {
            let w = project.width as f32 * h as f32 / project.height as f32;
            (ffmpeg::even(w), ffmpeg::even(h as f32))
        }
        _ => (ffmpeg::even(project.width as f32), ffmpeg::even(project.height as f32)),
    }
}

fn atempo(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-3 {
        String::new()
    } else if speed < 0.5 {
        format!("atempo=0.5,atempo={:.4},", speed / 0.5)
    } else {
        format!("atempo={speed:.4},")
    }
}

/// Эффект, который перед экспортом отрисовывается во временный файл.
pub struct EffectInput {
    pub path: PathBuf,
    pub effect: Effect,
    pub frames: u32,
    pub fps: f64,
}

/// Картинка, которая перед экспортом записывается во временный файл: надпись или маска эффекта.
pub struct TextInput {
    pub path: PathBuf,
    pub raster: Raster,
}

/// Что нужно для экспорта: аргументы ffmpeg, а также эффекты, надписи и маски, которые ему понадобятся как файлы.
pub struct Plan {
    pub args: Vec<String>,
    pub effects: Vec<EffectInput>,
    pub texts: Vec<TextInput>,
}

/// Имя временного файла, не повторяющееся между экспортами.
fn temp_file(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lumicut-{}-{n}-{name}", std::process::id()))
}

/// Доля перехода 0..1 как выражение ffmpeg: он начинается в `from` и длится `len` секунд.
fn share(from: f64, len: f64) -> String {
    format!("clip((t-{from:.6})/{len:.6},0,1)")
}

/// Та же доля с плавным разгоном и торможением.
fn eased(from: f64, len: f64) -> String {
    let q = share(from, len);
    format!("({q}*{q}*(3-2*{q}))")
}

/// Слагаемые выражения ffmpeg, которые ведут значение от одной закреплённой точки к другой.
/// `points` — моменты таймлайна и значения в них, `k` — множитель значения, `origin` — момент
/// таймлайна, от которого считает время фильтр, `digits` — знаков после запятой.
fn slopes(points: &[(f64, f32)], k: f64, origin: f64, digits: usize) -> String {
    let mut expr = String::new();
    for w in points.windows(2) {
        let d = (w[1].1 - w[0].1) as f64 * k;
        if d.abs() > 1e-6 {
            expr += &format!("{d:+.digits$}*{}", share(w[0].0 - origin, w[1].0 - w[0].0));
        }
    }
    expr
}

/// Свойство клипа как выражение ffmpeg от времени: закреплённые значения и равномерный ход между ними.
fn ramp(clip: &Clip, prop: Prop, k: f64, origin: f64) -> String {
    let points = clip.ramp(prop);
    format!("{:.4}{}", points[0].1 as f64 * k, slopes(&points, k, origin, 4))
}

/// Непрозрачность ниже этой уже отличается от полной.
const OPAQUE: f32 = 0.999;

/// Непрозрачность, закреплённая в нескольких моментах: на каждый участок между точками свой фильтр,
/// включённый только на этом участке. `origin` — момент таймлайна, с которого начинается поток клипа.
/// Возвращает фильтры и сдвиг времени в кадрах, с которым они работают.
fn fading(points: &[(f64, f32)], origin: f64, fps: f64) -> (String, u64) {
    let points: Vec<(f64, f64)> = points.iter().map(|p| (p.0 - origin, p.1.clamp(0.0, 1.0) as f64)).collect();
    // Участок — отрезок проявления (или затухания) от нуля до единицы, который начался бы
    // в `from` и длился `len`; сам участок — его часть между точками.
    let line = |a: (f64, f64), b: (f64, f64)| {
        let len = (b.0 - a.0) / (b.1 - a.1).abs();
        let done = if b.1 > a.1 { a.1 } else { 1.0 - a.1 };
        (a.0 - done * len, len)
    };
    let steady = |a: (f64, f64), b: (f64, f64)| (b.1 - a.1).abs() < 1e-4;
    // Фильтр не умеет начинать раньше нуля, поэтому время потока сдвигается вперёд на целое число кадров.
    let early = points.windows(2).filter(|w| !steady(w[0], w[1])).map(|w| line(w[0], w[1]).0).fold(0.0, f64::min);
    let frames = (-early * fps).ceil() as u64;
    let shift = frames as f64 / fps;

    let hold = |alpha: f64, when: String| {
        if alpha < OPAQUE as f64 {
            format!(",lut=a='val*{alpha:.4}':enable='{when}'")
        } else {
            String::new()
        }
    };
    let (first, last) = (points[0], points[points.len() - 1]);
    let mut filters = hold(first.1, format!("lt(t,{:.6})", first.0 + shift));
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let when = format!("gte(t,{:.6})*lt(t,{:.6})", a.0 + shift, b.0 + shift);
        if steady(a, b) {
            filters += &hold(a.1, when);
        } else {
            let (from, len) = line(a, b);
            let kind = if b.1 > a.1 { "in" } else { "out" };
            filters += &format!(",fade=t={kind}:st={:.6}:d={len:.6}:alpha=1:enable='{when}'", (from + shift).max(0.0));
        }
    }
    filters += &hold(last.1, format!("gte(t,{:.6})", last.0 + shift));
    (filters, frames)
}

type Edge = Option<(TransitionKind, f64)>;

/// Смещение клипа от середины кадра как выражения от времени: закреплённое положение,
/// движение по пути, разложенное на короткие прямые отрезки, и сдвиги переходов.
fn position(clip: &Clip, tr_in: Edge, tr_out: Edge, k: f64, fps: f64) -> (String, String) {
    let moving = clip.motion.as_ref().map_or(false, |m| m.is_active());
    let steps = if moving { ((clip.duration * fps / 2.0).ceil() as usize).clamp(1, 120) } else { 0 };
    let start = clip.position(clip.start);
    let mut x = format!("{:.2}", start.0 as f64 * k) + &slopes(&clip.ramp(Prop::PosX), k, 0.0, 2);
    let mut y = format!("{:.2}", start.1 as f64 * k) + &slopes(&clip.ramp(Prop::PosY), k, 0.0, 2);
    // Смещение, которое даёт путь: оно складывается с собственным положением клипа.
    let path = |t: f64| clip.motion.as_ref().and_then(|m| m.at(clip.progress(t))).unwrap_or([0.0, 0.0]);
    let mut prev = path(clip.start);
    let step = clip.duration / steps.max(1) as f64;
    for i in 0..steps {
        let from = clip.start + step * i as f64;
        let next = path(from + step);
        for (expr, d) in [(&mut x, next[0] - prev[0]), (&mut y, next[1] - prev[1])] {
            if d.abs() > 1e-3 {
                *expr += &format!("{:+.2}*{}", d as f64 * k, share(from, step));
            }
        }
        prev = next;
    }
    for (edge, at, incoming) in [(tr_in, clip.start, true), (tr_out, clip.end(), false)] {
        let Some((kind, half)) = edge else { continue };
        let Some((dx, dy)) = kind.slide() else { continue };
        // Входящий клип приезжает с той стороны, в которую уезжает уходящий.
        let e = eased(at - half, 2.0 * half);
        let part = if incoming { format!("({e}-1)") } else { e };
        if dx != 0.0 {
            x += &format!("{:+}*W*{part}", dx as i32);
        }
        if dy != 0.0 {
            y += &format!("{:+}*H*{part}", dy as i32);
        }
    }
    (x, y)
}

/// Разбирает таймлайн на входы и граф фильтров ffmpeg.
pub fn plan(project: &Project, s: &ExportSettings) -> Result<Plan, String> {
    let total = project.duration();
    if total <= 0.0 {
        return Err("На таймлайне нет клипов".to_owned());
    }
    let (ow, oh) = output_size(project, s);
    let k = ow as f64 / project.width as f64;
    let fps = s.fps.unwrap_or(project.fps).max(1.0);

    let mut inputs: Vec<String> = Vec::new();
    let mut n_inputs = 0usize;
    let mut graph: Vec<String> = vec![format!("color=c=black:s={ow}x{oh}:r={fps:.4}:d={total:.6}[bg]")];
    let mut last = "bg".to_owned();
    let mut video_n = 0usize;
    let mut audio: Vec<String> = Vec::new();
    let mut effect_inputs: Vec<EffectInput> = Vec::new();
    let mut text_inputs: Vec<TextInput> = Vec::new();

    let mut add_audio = |graph: &mut Vec<String>, idx: usize, clip: &Clip| {
        let label = format!("a{}", audio.len());
        let delay = (clip.start * 48_000.0).round() as i64;
        // Закреплённая громкость меняется по ходу клипа; время звуковой фильтр считает от его начала.
        let volume = if clip.keys.volume.is_empty() {
            format!("{:.4}", clip.volume)
        } else {
            format!("'{}':eval=frame", ramp(clip, Prop::Volume, 1.0, clip.start))
        };
        graph.push(format!(
            "[{idx}:a]{}volume={volume},aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,adelay={delay}S:all=1[{label}]",
            atempo(clip.speed)
        ));
        audio.push(label);
    };

    // Видеодорожки: от нижней к верхней.
    for (ti, track) in project.tracks.iter().enumerate().rev() {
        if track.kind != TrackKind::Video || !track.enabled {
            continue;
        }
        for clip in &track.clips {
            let idx = n_inputs;
            // Вход ffmpeg и размер клипа в кадре проекта при масштабе 100%.
            let (bw, bh) = if let Some(effect) = &clip.effect {
                let path = temp_file(&format!("effect{}.mov", effect_inputs.len()));
                inputs.extend(["-i".to_owned(), path.to_string_lossy().into_owned()]);
                let frames = effects::frame_count(clip.duration, fps);
                effect_inputs.push(EffectInput { path, effect: effect.clone(), frames, fps });
                let side = project.width.min(project.height) as f32;
                (side, side)
            } else if let Some(text) = &clip.text {
                // Надпись — неподвижная картинка на всё время клипа.
                let path = temp_file(&format!("text{}.png", text_inputs.len()));
                inputs.extend(["-loop", "1", "-framerate", &format!("{fps:.4}"), "-t", &format!("{:.6}", clip.duration), "-i"].map(String::from));
                inputs.push(path.to_string_lossy().into_owned());
                let raster = text::render(text);
                let size = (raster.w as f32, raster.h as f32);
                text_inputs.push(TextInput { path, raster });
                size
            } else {
                let Some(media) = project.media(clip.media) else { continue };
                if media.kind == MediaKind::Audio {
                    continue;
                }
                let path = media.path.to_string_lossy().into_owned();
                if media.kind == MediaKind::Image {
                    inputs.extend([
                        "-loop".to_owned(),
                        "1".to_owned(),
                        "-framerate".to_owned(),
                        format!("{fps:.4}"),
                        "-t".to_owned(),
                        format!("{:.6}", clip.duration),
                        "-i".to_owned(),
                        path,
                    ]);
                } else {
                    inputs.extend([
                        "-ss".to_owned(),
                        format!("{:.6}", clip.offset),
                        "-t".to_owned(),
                        format!("{:.6}", clip.source_duration()),
                        "-i".to_owned(),
                        path,
                    ]);
                }
                fit_size(media.width as f32, media.height as f32, project.width as f32, project.height as f32)
            };
            n_inputs += 1;
            let media = project.media(clip.media).filter(|_| !clip.generated());
            let video = media.map_or(false, |m| m.kind == MediaKind::Video);

            // Пока идёт переход, клип виден и за своими границами: там он замирает на крайнем кадре.
            let (lead, tail) = project.overhang(ti, clip);
            let (tr_in, tr_out) = (project.transition_in(ti, clip), project.transition_out(ti, clip));
            // Клип с закреплённым масштабом готовится в наибольшем размере, а меняется уже по кадрам.
            let top = clip.peak(Prop::Scale);
            let w = ffmpeg::even(bw * top * k as f32);
            let h = ffmpeg::even(bh * top * k as f32);
            let mut chain = format!("[{idx}:v]");
            if video {
                chain += &format!("setpts=(PTS-STARTPTS)/{:.4},", clip.speed);
            } else {
                chain += "setpts=PTS-STARTPTS,";
            }
            // «Пиксели» эффекта увеличиваются без сглаживания.
            let sharp = if clip.effect.is_some() { ":flags=neighbor" } else { "" };
            // Хромакей, цвет, эффекты и маска клипа; картинки-маски идут в ffmpeg отдельными входами.
            let style = match media {
                Some(_) => vfx::filters(&clip.look, &clip.vfx, w, h, &format!("x{video_n}_"), n_inputs),
                None => vfx::Filters { chain: String::new(), rasters: Vec::new() },
            };
            for raster in style.rasters {
                let path = temp_file(&format!("matte{}.png", text_inputs.len()));
                inputs.extend(["-i".to_owned(), path.to_string_lossy().into_owned()]);
                text_inputs.push(TextInput { path, raster });
                n_inputs += 1;
            }
            // Цвет правится в RGB, эффекты и надписи в нём приходят; остальным клипам хватает исходного
            // YUV с прозрачностью — без пересчёта цвета туда и обратно фильтры идут вдвое быстрее.
            let format = if media.is_none() || !style.chain.is_empty() { "rgba" } else { "yuva420p" };
            chain += &format!("fps={fps:.4},scale={w}:{h}{sharp},setsar=1,format={format}{}", style.chain);
            if clip.flip_h {
                chain += ",hflip";
            }
            if clip.flip_v {
                chain += ",vflip";
            }
            if lead > 0.0 || tail > 0.0 {
                chain += &format!(
                    ",tpad=start_mode=clone:start_duration={lead:.6}:stop_mode=clone:stop_duration={tail:.6}"
                );
            }
            let turning = !clip.keys.rotation.is_empty();
            if !turning && clip.rotation.abs() > 0.01 {
                let a = (clip.rotation as f64).to_radians();
                // При сложении и в режиме «Экран» цвет углов прибавился бы к кадру: они заливаются чёрным.
                let fill = if clip.vfx.blend == Blend::Normal { "none" } else { "black@0" };
                chain += &format!(",rotate={a:.6}:ow=rotw({a:.6}):oh=roth({a:.6}):c={fill}");
            }
            let opacity = clip.ramp(Prop::Opacity);
            if opacity.len() == 1 && clip.opacity < OPAQUE {
                chain += &format!(",lut=a='val*{:.4}'", clip.opacity.clamp(0.0, 1.0));
            }
            // Проявление и затухание отсчитываются от начала потока клипа, то есть от `start - lead`.
            match tr_in {
                Some((TransitionKind::Dissolve | TransitionKind::Zoom, half)) => {
                    chain += &format!(",fade=t=in:st=0:d={:.6}:alpha=1", 2.0 * half);
                }
                Some((TransitionKind::Fade, half)) => chain += &format!(",fade=t=in:st=0:d={half:.6}:alpha=1"),
                _ => {}
            }
            if let Some((TransitionKind::Fade, half)) = tr_out {
                chain += &format!(",fade=t=out:st={:.6}:d={half:.6}:alpha=1", lead + clip.duration - half);
            }
            let label = format!("v{video_n}");
            // Закреплённая непрозрачность считает время со сдвигом; привязка ко времени его снимает.
            let mut ahead = String::new();
            if opacity.len() > 1 {
                let (filters, frames) = fading(&opacity, clip.start - lead, fps);
                if frames > 0 {
                    chain += &format!(",setpts=PTS+{frames}");
                    ahead = format!("-{frames}");
                }
                chain += &filters;
            }
            chain += &format!(",setpts=PTS{ahead}+{:.6}/TB", clip.start - lead);
            // Закреплённый поворот: кадр расширен до диагонали, чтобы клип помещался под любым углом,
            // а фон заливается прозрачным заново в каждом кадре, иначе за клипом тянется след.
            if turning {
                let a = ramp(clip, Prop::Rotation, std::f64::consts::PI / 180.0, 0.0);
                chain += &format!(",rotate=a='{a}':ow='hypot(iw,ih)':oh=ow:c=black@0");
            }
            // Режим наложения: непрозрачность к этому месту уже окончательная, а размер кадра ещё постоянный.
            let (prep, over) = vfx::blend_filters(clip.vfx.blend, &format!("m{video_n}"));
            chain += &prep;
            // «Наезд» и закреплённый масштаб меняют размер кадр за кадром, поэтому стоят последними,
            // после привязки ко времени.
            let mut zoom: Vec<String> = Vec::new();
            if !clip.keys.scale.is_empty() {
                zoom.push(format!("({})", ramp(clip, Prop::Scale, 1.0 / top as f64, 0.0)));
            }
            if let Some((TransitionKind::Zoom, half)) = tr_in {
                zoom.push(format!("({ZOOM_IN:.4}+{:.4}*{})", 1.0 - ZOOM_IN, eased(clip.start - half, 2.0 * half)));
            }
            if let Some((TransitionKind::Zoom, half)) = tr_out {
                zoom.push(format!("(1+{:.4}*{})", ZOOM_OUT - 1.0, eased(clip.end() - half, 2.0 * half)));
            }
            if !zoom.is_empty() {
                let z = zoom.join("*");
                chain += &format!(",scale=w='iw*{z}':h='ih*{z}':eval=frame{sharp}");
            }
            chain += &format!("[{label}]");
            graph.push(chain);

            let (x, y) = position(clip, tr_in, tr_out, k, fps);
            let out = format!("o{video_n}");
            graph.push(format!(
                "[{last}][{label}]overlay=x='(W-w)/2+({x})':y='(H-h)/2+({y})':eof_action=pass{over}:enable='between(t,{:.6},{:.6})'[{out}]",
                clip.start - lead,
                clip.end() + tail
            ));
            last = out;

            // Вспышка перехода: белый кадр поверх стыка, проявляется и гаснет.
            if let Some((TransitionKind::Flash, half)) = tr_in {
                let (white, out) = (format!("w{video_n}"), format!("f{video_n}"));
                let from = clip.start - half;
                graph.push(format!(
                    "color=c=white:s={ow}x{oh}:r={fps:.4}:d={:.6},format=rgba,fade=t=in:st=0:d={half:.6}:alpha=1,fade=t=out:st={half:.6}:d={half:.6}:alpha=1,setpts=PTS+{from:.6}/TB[{white}]",
                    2.0 * half
                ));
                graph.push(format!(
                    "[{last}][{white}]overlay=eof_action=pass:enable='between(t,{from:.6},{:.6})'[{out}]",
                    clip.start + half
                ));
                last = out;
            }
            video_n += 1;

            if let Some(media) = media {
                if video && media.has_audio && clip.peak(Prop::Volume) > 0.001 {
                    add_audio(&mut graph, idx, clip);
                }
            }
        }
    }

    for track in &project.tracks {
        if track.kind != TrackKind::Audio || !track.enabled {
            continue;
        }
        for clip in &track.clips {
            let Some(media) = project.media(clip.media) else { continue };
            if media.kind != MediaKind::Audio || clip.peak(Prop::Volume) <= 0.001 {
                continue;
            }
            let idx = n_inputs;
            n_inputs += 1;
            inputs.extend([
                "-ss".to_owned(),
                format!("{:.6}", clip.offset),
                "-t".to_owned(),
                format!("{:.6}", clip.source_duration()),
                "-i".to_owned(),
                media.path.to_string_lossy().into_owned(),
            ]);
            add_audio(&mut graph, idx, clip);
        }
    }

    graph.push(format!("[{last}]format=yuv420p[vout]"));
    let has_audio = !audio.is_empty();
    if audio.len() == 1 {
        graph.push(format!("[{}]anull[aout]", audio[0]));
    } else if audio.len() > 1 {
        let ins: String = audio.iter().map(|l| format!("[{l}]")).collect();
        graph.push(format!(
            "{ins}amix=inputs={}:normalize=0:dropout_transition=0:duration=longest[aout]",
            audio.len()
        ));
    }

    let mut args: Vec<String> = ["-y", "-hide_banner", "-v", "error", "-progress", "pipe:1", "-nostats"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.extend(inputs);
    args.extend(["-filter_complex".to_owned(), graph.join(";"), "-map".to_owned(), "[vout]".to_owned()]);
    if has_audio {
        args.extend(["-map".to_owned(), "[aout]".to_owned()]);
    }
    args.extend(encoder(s));
    if has_audio {
        args.extend(["-c:a", "aac", "-b:a", "192k"].iter().map(|s| s.to_string()));
    }
    args.extend([
        "-t".to_owned(),
        format!("{total:.6}"),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
        s.path.to_string_lossy().into_owned(),
    ]);
    Ok(Plan { args, effects: effect_inputs, texts: text_inputs })
}

/// Записывает надпись в картинку с прозрачностью.
fn render_text(input: &TextInput) -> Result<(), String> {
    let mut child = ffmpeg::ffmpeg()
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", input.raster.w, input.raster.h), "-i", "-", "-frames:v", "1"])
        .arg(&input.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Не удалось запустить ffmpeg: {e}"))?;
    let written = child.stdin.take().map_or(false, |mut pipe| pipe.write_all(&input.raster.rgba).is_ok());
    let done = child.wait().map_or(false, |status| status.success());
    if written && done {
        Ok(())
    } else {
        Err("Не удалось отрисовать надпись".to_owned())
    }
}

/// Отрисовывает эффект во временный файл с прозрачностью: кадры уходят в ffmpeg по каналу.
fn render_effect(input: &EffectInput, cancel: &AtomicBool) -> Result<(), String> {
    let side = effects::side(&input.effect);
    let mut child = ffmpeg::ffmpeg()
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{side}x{side}"), "-r", &format!("{:.4}", input.fps), "-i", "-", "-c:v", "png"])
        .arg(&input.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Не удалось запустить ffmpeg: {e}"))?;
    let mut pipe = child.stdin.take().ok_or("нет ввода ffmpeg")?;
    for n in 0..input.frames {
        let frame = effects::render(&input.effect, n as f32 / input.frames as f32);
        if cancel.load(Ordering::SeqCst) || pipe.write_all(&frame).is_err() {
            break;
        }
    }
    drop(pipe);
    let done = child.wait().map_or(false, |status| status.success());
    if done || cancel.load(Ordering::SeqCst) {
        Ok(())
    } else {
        Err("Не удалось отрисовать эффект".to_owned())
    }
}

pub enum ExportMsg {
    Progress(f32),
    Done,
    Failed(String),
}

pub struct ExportJob {
    pub rx: Receiver<ExportMsg>,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl ExportJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        // Экспорт мог ещё не дойти до запуска ffmpeg: флаг остановит и подготовку.
        self.cancel();
    }
}

/// Запускает экспорт в фоне.
pub fn start(project: &Project, settings: &ExportSettings) -> Result<ExportJob, String> {
    // Запасной план на процессоре: если видеокарта не справится с таким кадром, экспорт начнётся заново без неё.
    let spare = match settings.gpu {
        Some(_) => Some(plan(project, &ExportSettings { gpu: None, ..settings.clone() })?),
        None => None,
    };
    let plan = plan(project, settings)?;
    let total = project.duration();
    let child = Arc::new(Mutex::new(None));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();

    let child2 = child.clone();
    let cancel2 = cancel.clone();
    let out_path = settings.path.clone();
    std::thread::spawn(move || {
        let mut temp: Vec<PathBuf> = Vec::new();
        let mut result = run(plan, total, &tx, &cancel2, &child2, &mut temp);
        if let (true, Some(spare), false) = (result.is_err(), spare, cancel2.load(Ordering::SeqCst)) {
            let _ = tx.send(ExportMsg::Progress(0.0));
            result = run(spare, total, &tx, &cancel2, &child2, &mut temp);
        }
        for path in temp {
            let _ = std::fs::remove_file(path);
        }
        let msg = if cancel2.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&out_path);
            ExportMsg::Failed("Экспорт отменён".to_owned())
        } else {
            match result {
                Ok(()) => ExportMsg::Done,
                Err(e) => ExportMsg::Failed(e),
            }
        };
        let _ = tx.send(msg);
    });

    Ok(ExportJob { rx, cancel, child })
}

/// Сам экспорт: эффекты и надписи во временные файлы, затем ffmpeg. Созданные по пути файлы попадают в `temp`.
fn run(
    plan: Plan,
    total: f64,
    tx: &Sender<ExportMsg>,
    cancel: &AtomicBool,
    slot: &Mutex<Option<Child>>,
    temp: &mut Vec<PathBuf>,
) -> Result<(), String> {
    temp.extend(plan.effects.iter().map(|e| e.path.clone()));
    temp.extend(plan.texts.iter().map(|t| t.path.clone()));
    for effect in &plan.effects {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        render_effect(effect, cancel)?;
    }
    for text in &plan.texts {
        render_text(text)?;
    }

    let mut args = plan.args;
    if args.iter().map(|a| a.chars().count() + 3).sum::<usize>() > MAX_INLINE_ARGS {
        if let Some(at) = args.iter().position(|a| a == "-filter_complex") {
            let script = temp_file("graph.txt");
            std::fs::write(&script, &args[at + 1]).map_err(|e| format!("Не удалось записать граф фильтров: {e}"))?;
            args[at] = ffmpeg::filter_script_flag().to_owned();
            args[at + 1] = script.to_string_lossy().into_owned();
            temp.push(script);
        }
    }

    let mut child = ffmpeg::ffmpeg()
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось запустить ffmpeg: {e}"))?;
    let stdout = child.stdout.take().ok_or("нет вывода ffmpeg")?;
    let mut stderr = child.stderr.take().ok_or("нет вывода ffmpeg")?;
    {
        let mut slot = slot.lock().unwrap();
        // Отмену могли запросить, пока рисовались эффекты.
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
        }
        *slot = Some(child);
    }

    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(v) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = v.trim().parse::<f64>() {
                let _ = tx.send(ExportMsg::Progress((us / 1e6 / total).clamp(0.0, 1.0) as f32));
            }
        }
    }
    let status = slot.lock().unwrap().take().map(|mut c| c.wait());
    let err = err_thread.join().unwrap_or_default();
    if matches!(status, Some(Ok(s)) if s.success()) {
        return Ok(());
    }
    let tail: Vec<&str> = err.lines().rev().take(4).collect();
    let text = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Err(if text.is_empty() { "ffmpeg завершился с ошибкой".to_owned() } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::EffectKind;
    use crate::model::{Id, MediaItem};
    use crate::motion::Motion;

    fn graph_of(p: &Project) -> String {
        let args = plan(p, &ExportSettings::new(PathBuf::from("out.mp4"))).unwrap().args;
        args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1].clone()
    }

    /// Проект с десятисекундным видео, разрезанным на четвёртой секунде.
    fn cut_project() -> (Project, Id, Id) {
        let mut p = Project::new();
        let v = p.alloc_id();
        p.media.push(MediaItem {
            id: v,
            path: PathBuf::from("a.mp4"),
            name: "a.mp4".into(),
            kind: MediaKind::Video,
            duration: 10.0,
            width: 1920,
            height: 1080,
            fps: 30.0,
            has_audio: false,
        });
        let a = p.append_clip(v).unwrap();
        let b = p.split(a, 4.0).unwrap();
        (p, a, b)
    }

    #[test]
    fn dissolve_freezes_both_clips_and_fades_the_new_one_in() {
        let (mut p, _, b) = cut_project();
        p.set_transition(b, TransitionKind::Dissolve);
        let graph = graph_of(&p);
        // Уходящий клип замирает после стыка, входящий — до него и проявляется весь переход.
        assert!(graph.contains("tpad=start_mode=clone:start_duration=0.000000:stop_mode=clone:stop_duration=0.300000"));
        assert!(graph.contains("tpad=start_mode=clone:start_duration=0.300000:stop_mode=clone:stop_duration=0.000000"));
        assert!(graph.contains("fade=t=in:st=0:d=0.600000:alpha=1,setpts=PTS+3.700000/TB"));
        assert!(graph.contains("enable='between(t,0.000000,4.300000)'"));
        assert!(graph.contains("enable='between(t,3.700000,10.000000)'"));
    }

    #[test]
    fn fade_and_flash_do_not_overlap_clips() {
        let (mut p, _, b) = cut_project();
        p.set_transition(b, TransitionKind::Fade);
        let graph = graph_of(&p);
        assert!(!graph.contains("tpad"));
        assert!(graph.contains("fade=t=out:st=3.700000:d=0.300000:alpha=1"));
        assert!(graph.contains("fade=t=in:st=0:d=0.300000:alpha=1,setpts=PTS+4.000000/TB"));

        p.set_transition(b, TransitionKind::Flash);
        let graph = graph_of(&p);
        assert!(!graph.contains("tpad"));
        assert!(graph.contains("color=c=white:s=1920x1080"));
        assert!(graph.contains("enable='between(t,3.700000,4.300000)'"));
    }

    #[test]
    fn slide_and_zoom_are_written_as_expressions_of_time() {
        let (mut p, _, b) = cut_project();
        p.set_transition(b, TransitionKind::SlideLeft);
        let graph = graph_of(&p);
        let q = "clip((t-3.700000)/0.600000,0,1)";
        assert!(graph.contains(&format!("-1*W*({q}*{q}*(3-2*{q}))")), "уходящий клип уезжает влево");
        assert!(graph.contains(&format!("-1*W*(({q}*{q}*(3-2*{q}))-1)")), "входящий приезжает справа");

        p.set_transition(b, TransitionKind::Zoom);
        let graph = graph_of(&p);
        assert!(graph.contains("eval=frame"));
        assert!(graph.contains("scale=w='iw*(0.6000+0.4000*("));
        assert!(graph.contains("scale=w='iw*(1+0.6000*("));
    }

    #[test]
    fn motion_path_becomes_a_chain_of_segments() {
        let (mut p, a, _) = cut_project();
        let clip = p.clip_mut(a).unwrap();
        clip.motion = Some(Motion { points: vec![[0.0, 0.0], [600.0, 0.0]], smooth: false, ..Motion::default() });
        let graph = graph_of(&p);
        // Четыре секунды при 30 к/с — 60 отрезков по 10 пикселей; по вертикали клип не движется.
        assert_eq!(graph.matches("+10.00*clip((t-").count(), 60);
        assert!(graph.contains("y='(H-h)/2+(0.00)'"));
    }

    /// Допуск, с которым точка считается стоящей под курсором.
    const REACH: f64 = 0.01;

    #[test]
    fn pinned_values_are_written_as_expressions_of_time() {
        let (mut p, a, b) = cut_project();
        let clip = p.clip_mut(b).unwrap();
        // Второй клип идёт с 4-й по 10-ю секунду: за секунды 5–7 он уменьшается вдвое,
        // поворачивается на 90° и уезжает вправо.
        for (prop, to) in [(Prop::Scale, 0.5), (Prop::Rotation, 90.0), (Prop::PosX, 300.0)] {
            clip.toggle_key(prop, 5.0, REACH);
            clip.set_value(prop, 7.0, to, REACH);
        }
        let graph = graph_of(&p);
        let q = "clip((t-5.000000)/2.000000,0,1)";
        // Клип готовится в наибольшем размере, а уменьшается по кадрам.
        assert!(graph.contains("scale=1920:1080,"));
        assert!(graph.contains(&format!("scale=w='iw*(1.0000-0.5000*{q})':h='ih*(1.0000-0.5000*{q})':eval=frame")));
        assert!(graph.contains(&format!("rotate=a='0.0000+1.5708*{q}':ow='hypot(iw,ih)':oh=ow:c=black@0")));
        assert!(graph.contains(&format!("overlay=x='(W-w)/2+(0.00+300.00*{q})':y='(H-h)/2+(0.00)'")));
        // Первый клип не тронут.
        let first = graph.split(';').find(|f| f.starts_with("[0:v]")).unwrap();
        assert!(!first.contains("eval=frame") && !first.contains("rotate"));
        let _ = a;
    }

    #[test]
    fn pinned_opacity_is_a_filter_per_stretch() {
        let (mut p, _, b) = cut_project();
        let clip = p.clip_mut(b).unwrap();
        // Клип идёт с 4-й по 10-ю секунду: с 50% в начале до 75% на шестой секунде,
        // к седьмой — 25%, и так до конца.
        clip.opacity = 0.5;
        clip.toggle_key(Prop::Opacity, 4.0, REACH);
        clip.set_value(Prop::Opacity, 6.0, 0.75, REACH);
        clip.set_value(Prop::Opacity, 7.0, 0.25, REACH);
        let graph = graph_of(&p);
        let chain = graph.split(';').find(|f| f.starts_with("[1:v]")).unwrap();
        // Проявление от нуля до единицы с такой скоростью заняло бы 8 секунд и началось бы
        // за 4 секунды до клипа, поэтому время его потока сдвинуто на 4 секунды (120 кадров).
        assert!(chain.contains(",setpts=PTS+120,"), "{chain}");
        assert!(chain.contains("lut=a='val*0.5000':enable='lt(t,4.000000)'"), "{chain}");
        assert!(chain.contains("fade=t=in:st=0.000000:d=8.000000:alpha=1:enable='gte(t,4.000000)*lt(t,6.000000)'"), "{chain}");
        assert!(chain.contains("fade=t=out:st=5.500000:d=2.000000:alpha=1:enable='gte(t,6.000000)*lt(t,7.000000)'"), "{chain}");
        assert!(chain.contains("lut=a='val*0.2500':enable='gte(t,7.000000)*lt(t,10.000000)'"), "{chain}");
        assert!(chain.contains("lut=a='val*0.2500':enable='gte(t,10.000000)'"), "{chain}");
        assert!(chain.contains(",setpts=PTS-120+4.000000/TB["), "{chain}");
    }

    #[test]
    fn pinned_volume_changes_within_the_clip() {
        let mut p = Project::new();
        let m = p.alloc_id();
        p.media.push(MediaItem {
            id: m,
            path: PathBuf::from("a.mp3"),
            name: "a.mp3".into(),
            kind: MediaKind::Audio,
            duration: 10.0,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: true,
        });
        let c = p.add_clip(2, m, 3.0).unwrap();
        let clip = p.clip_mut(c).unwrap();
        // Звук нарастает с нуля за первые две секунды клипа.
        clip.volume = 0.0;
        clip.toggle_key(Prop::Volume, 3.0, REACH);
        clip.set_value(Prop::Volume, 5.0, 1.0, REACH);
        let graph = graph_of(&p);
        assert!(graph.contains("volume='0.0000+1.0000*clip((t-0.000000)/2.000000,0,1)':eval=frame,"), "{graph}");
    }

    #[test]
    fn plain_video_stays_in_yuv_and_colour_work_goes_through_rgb() {
        let (mut p, a, b) = cut_project();
        let chain = |p: &Project, input: &str| graph_of(p).split(';').find(|f| f.starts_with(input)).unwrap().to_owned();
        // Обычному клипу пересчёт цвета не нужен.
        assert!(chain(&p, "[0:v]").contains("setsar=1,format=yuva420p,setpts"));

        let clip = p.clip_mut(a).unwrap();
        clip.look.filter = crate::look::FilterKind::Mono;
        clip.flip_h = true;
        clip.opacity = 0.5;
        let first = chain(&p, "[0:v]");
        assert!(first.contains("format=rgba,lutrgb=r='clip("), "{first}");
        assert!(first.contains(",colorchannelmixer=rr=0.2126:rg=0.7152:rb=0.0722:gr=0.2126:"), "{first}");
        assert!(first.contains(",hflip,lut=a='val*0.5000',setpts"), "{first}");
        // Соседний клип остаётся в YUV.
        assert!(chain(&p, "[1:v]").contains("format=yuva420p"));

        p.clip_mut(b).unwrap().look.key = Some(crate::look::ChromaKey::default());
        assert!(chain(&p, "[1:v]").contains("format=rgba,colorkey=0x00FF00:0.3000:0.1000,setpts"));
    }

    #[test]
    fn clip_effects_go_into_the_chain_and_masks_become_inputs() {
        use crate::vfx::{Fx, FxKind, Mask, MaskShape};
        let (mut p, a, b) = cut_project();
        let chain = |p: &Project, input: &str| graph_of(p).split(';').find(|f| f.starts_with(input)).unwrap().to_owned();
        let clip = p.clip_mut(a).unwrap();
        clip.look.key = Some(crate::look::ChromaKey { choke: 0.25, ..crate::look::ChromaKey::studio([0, 255, 0]) });
        clip.vfx.stack = vec![Fx { amount: 0.5, ..Fx::new(FxKind::Blur) }, Fx { on: false, ..Fx::new(FxKind::Invert) }];
        // Хромакей с подавлением отсвета и срезанным краем, затем включённые эффекты по порядку.
        let first = chain(&p, "[0:v]");
        assert!(
            first.contains(
                "format=rgba,colorkey=0x00FF00:0.3000:0.1000,despill=type=green:mix=0.5:expand=0:green=-0.5000:blue=0,format=gbrap\
                 ,erosion=threshold0=0:threshold1=0:threshold2=0,erosion=threshold0=0:threshold1=0:threshold2=0,format=rgba,gblur=sigma=21.60,setpts"
            ),
            "{first}"
        );
        assert!(!first.contains("lutrgb"), "выключенный эффект в экспорт не попадает");

        // Маска второго клипа — картинка размером с клип: она идёт третьим входом после двух клипов.
        p.clip_mut(b).unwrap().vfx.mask = Some(Mask::new(MaskShape::Ellipse));
        let plan = plan(&p, &ExportSettings::new(PathBuf::from("out.mp4"))).unwrap();
        assert_eq!(plan.texts.len(), 1);
        assert_eq!((plan.texts[0].raster.w, plan.texts[0].raster.h), (1920, 1080));
        let path = plan.texts[0].path.to_string_lossy().into_owned();
        let at = plan.args.iter().position(|a| *a == path).unwrap();
        assert_eq!(plan.args[at - 1], "-i");
        let graph = &plan.args[plan.args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("[1:v]setpts=(PTS-STARTPTS)/1.0000,fps=30.0000,scale=1920:1080,setsar=1,format=rgba,format=gbrap[x1_1];"), "{graph}");
        assert!(graph.contains("[2:v]format=gbrap,loop=loop=-1:size=1:start=0[x1_2];[x1_1][x1_2]blend=c3_mode=multiply:shortest=1,format=rgba,setpts"), "{graph}");
    }

    #[test]
    fn blend_modes_change_how_the_clip_is_overlaid() {
        let (mut p, a, b) = cut_project();
        let graph = graph_of(&p);
        assert!(!graph.contains("premultiply") && graph.contains("eof_action=pass:enable="));

        // Сложение: цвет умножается на непрозрачность уже после неё самой, но до смены размера по кадрам.
        let clip = p.clip_mut(a).unwrap();
        clip.vfx.blend = Blend::Add;
        clip.opacity = 0.5;
        clip.rotation = 30.0;
        clip.toggle_key(Prop::Scale, 1.0, REACH);
        clip.set_value(Prop::Scale, 3.0, 0.5, REACH);
        p.clip_mut(b).unwrap().vfx.blend = Blend::Screen;
        let graph = graph_of(&p);
        let first = graph.split(';').find(|f| f.starts_with("[0:v]")).unwrap();
        assert!(first.contains(":c=black@0,lut=a='val*0.5000',setpts=PTS+0.000000/TB,format=gbrap,premultiply=inplace=1,colorchannelmixer=aa=0,scale=w='iw*"), "{first}");
        assert!(graph.contains("eof_action=pass:alpha=premultiplied:enable='between(t,0.000000,4.000000)'"), "{graph}");
        // «Экран»: непрозрачность берётся из яркости, наложение остаётся обычным.
        assert!(graph.contains(",format=gbrap,premultiply=inplace=1,split[m1c][m1m];[m1m]extractplanes=r+g+b[m1r][m1g][m1b];"), "{graph}");
        assert!(graph.contains("[m1c][m1x]alphamerge,unpremultiply=inplace=1[v1]"), "{graph}");
        assert!(graph.contains("eof_action=pass:enable='between(t,4.000000,10.000000)'"), "{graph}");
    }

    #[test]
    fn text_is_written_to_a_picture_and_held_for_the_clip() {
        let (mut p, _, _) = cut_project();
        let text = crate::text::TextPreset::Title.text(p.height);
        let id = p.add_text(0, text.clone(), 2.0, 3.0).unwrap();
        p.clip_mut(id).unwrap().pos_y = 300.0;
        let plan = plan(&p, &ExportSettings::new(PathBuf::from("out.mp4"))).unwrap();
        assert_eq!(plan.texts.len(), 1);
        let (raster, path) = (&plan.texts[0].raster, plan.texts[0].path.to_string_lossy().into_owned());
        assert_eq!(*raster, text::render(&text));
        // Картинка идёт входом на всё время клипа; дорожки рисуются снизу вверх, надпись — последней.
        let at = plan.args.iter().position(|a| *a == path).unwrap();
        assert_eq!(plan.args[at - 7..at], ["-loop", "1", "-framerate", "30.0000", "-t", "3.000000", "-i"]);
        let graph = &plan.args[plan.args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        let (w, h) = (ffmpeg::even(raster.w as f32), ffmpeg::even(raster.h as f32));
        assert!(graph.contains(&format!("[2:v]setpts=PTS-STARTPTS,fps=30.0000,scale={w}:{h},setsar=1,format=rgba,setpts=PTS+2.000000/TB")), "{graph}");
        assert!(graph.contains("y='(H-h)/2+(300.00)':eof_action=pass:enable='between(t,2.000000,5.000000)'"));
    }

    #[test]
    fn video_card_encoders_get_their_own_settings() {
        let (p, _, _) = cut_project();
        let args = |codec: Codec, gpu: Option<Gpu>| {
            let s = ExportSettings { codec, gpu, ..ExportSettings::new(PathBuf::from("out.mp4")) };
            let args = plan(&p, &s).unwrap().args;
            let at = args.iter().position(|a| a == "-c:v").unwrap();
            args[at..args.len() - 5].join(" ")
        };
        assert_eq!(args(Codec::H264, None), "-c:v libx264 -preset medium -crf 21");
        assert_eq!(args(Codec::H265, None), "-c:v libx265 -preset medium -crf 25 -tag:v hvc1");
        assert_eq!(args(Codec::H264, Some(Gpu::Nvidia)), "-c:v h264_nvenc -preset p5 -rc vbr -cq 24 -b:v 0");
        assert_eq!(args(Codec::H265, Some(Gpu::Nvidia)), "-c:v hevc_nvenc -preset p5 -rc vbr -cq 28 -b:v 0 -tag:v hvc1");
        assert_eq!(args(Codec::H264, Some(Gpu::Intel)), "-c:v h264_qsv -preset medium -global_quality 23");
        assert_eq!(args(Codec::H265, Some(Gpu::Amd)), "-c:v hevc_amf -quality balanced -rc cqp -qp_i 27 -qp_p 27 -qp_b 27 -tag:v hvc1");
    }

    #[test]
    fn effect_is_rendered_to_a_file_and_scaled_without_smoothing() {
        let mut p = Project::new();
        p.add_effect(0, EffectKind::Explosion, 1.0).unwrap();
        let plan = plan(&p, &ExportSettings::new(PathBuf::from("out.mp4"))).unwrap();
        assert_eq!(plan.effects.len(), 1);
        assert_eq!(plan.effects[0].frames, 36);
        assert!(plan.args.contains(&plan.effects[0].path.to_string_lossy().into_owned()));
        let graph = &plan.args[plan.args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("scale=1080:1080:flags=neighbor"));
    }

    #[test]
    fn graph_contains_clips_and_audio() {
        let mut p = Project::new();
        let v = p.alloc_id();
        p.media.push(MediaItem {
            id: v,
            path: PathBuf::from("a.mp4"),
            name: "a.mp4".into(),
            kind: MediaKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            has_audio: true,
        });
        let c = p.append_clip(v).unwrap();
        p.set_speed(c, 2.0);
        let args = plan(&p, &ExportSettings::new(PathBuf::from("out.mp4"))).unwrap().args;
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("color=c=black:s=1920x1080"));
        assert!(graph.contains("setpts=(PTS-STARTPTS)/2.0000"));
        assert!(graph.contains("scale=1920:1080"));
        assert!(graph.contains("atempo=2.0000"));
        assert!(args.contains(&"[aout]".to_owned()));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    /// Полный экспорт настоящих файлов. Запуск:
    /// `LUMICUT_TEST_MEDIA=<папка> cargo test -- --ignored export_end_to_end`.
    #[test]
    #[ignore = "нужны ffmpeg и тестовые файлы в LUMICUT_TEST_MEDIA"]
    fn export_end_to_end() {
        let dir = PathBuf::from(std::env::var("LUMICUT_TEST_MEDIA").expect("LUMICUT_TEST_MEDIA"));
        let mut p = Project::new();
        let mut ids = Vec::new();
        for name in ["clip1.mp4", "clip2.mp4", "photo.png", "music.mp3"] {
            let path = dir.join(name);
            let info = ffmpeg::probe(&path).expect("probe");
            let id = p.alloc_id();
            p.media.push(MediaItem {
                id,
                path,
                name: name.into(),
                kind: info.kind,
                duration: info.duration,
                width: info.width,
                height: info.height,
                fps: info.fps,
                has_audio: info.has_audio,
            });
            ids.push(id);
            let c = p.append_clip(id).expect("append");
            if name == "clip2.mp4" {
                p.set_speed(c, 2.0);
            }
        }
        // Наложение на верхней дорожке: уменьшенное, повёрнутое, полупрозрачное.
        let overlay = p.add_clip(0, ids[1], 2.0).expect("overlay");
        p.trim_end(overlay, 6.0);
        let c = p.clip_mut(overlay).unwrap();
        c.scale = 0.4;
        c.rotation = 15.0;
        c.opacity = 0.8;
        c.pos_x = 480.0;
        c.pos_y = -240.0;
        c.volume = 0.0;
        assert!((p.duration() - 20.0).abs() < 0.2, "длительность {}", p.duration());
        p.save(&dir.join("test.lumicut")).expect("save");

        let mut settings = ExportSettings::new(dir.join("out.mp4"));
        settings.height = Some(360);
        let job = start(&p, &settings).expect("start");
        let mut result = None;
        while let Ok(msg) = job.rx.recv() {
            match msg {
                ExportMsg::Progress(_) => {}
                ExportMsg::Done => result = Some(Ok(())),
                ExportMsg::Failed(e) => result = Some(Err(e)),
            }
        }
        assert_eq!(result, Some(Ok(())));

        let out = ffmpeg::probe(&settings.path).expect("probe out");
        assert_eq!((out.width, out.height), (640, 360));
        assert!(out.has_audio);
        assert!((out.duration - 20.0).abs() < 0.3, "длительность файла {}", out.duration);
    }

    /// Экспорт всех переходов, эффекта и движения по пути. Запуск — как у `export_end_to_end`.
    #[test]
    #[ignore = "нужны ffmpeg и тестовые файлы в LUMICUT_TEST_MEDIA"]
    fn export_transitions_effects_and_motion() {
        let dir = PathBuf::from(std::env::var("LUMICUT_TEST_MEDIA").expect("LUMICUT_TEST_MEDIA"));
        let mut p = Project::new();
        let mut ids = Vec::new();
        for name in ["clip1.mp4", "clip2.mp4", "photo.png"] {
            let path = dir.join(name);
            let info = ffmpeg::probe(&path).expect("probe");
            let id = p.alloc_id();
            p.media.push(MediaItem {
                id,
                path,
                name: name.into(),
                kind: info.kind,
                duration: info.duration,
                width: info.width,
                height: info.height,
                fps: info.fps,
                has_audio: info.has_audio,
            });
            ids.push(id);
        }
        // Основная дорожка: полуторасекундные куски двух клипов по очереди, на каждом стыке свой переход.
        for (i, kind) in [None].into_iter().chain(TransitionKind::ALL.map(Some)).enumerate() {
            let clip = p.append_clip(ids[i % 2]).expect("append");
            let start = p.clip(clip).unwrap().start;
            p.trim_end(clip, start + 1.5);
            p.clip_mut(clip).unwrap().volume = 0.0;
            if let Some(kind) = kind {
                assert!(p.set_transition(clip, kind), "{kind:?}");
            }
        }
        // Сверху: картинка, которая ходит по кругу, и взрыв.
        let photo = p.add_clip(0, ids[2], 0.5).expect("photo");
        p.trim_end(photo, 4.5);
        let c = p.clip_mut(photo).unwrap();
        c.scale = 0.3;
        c.motion = Some(Motion::circle(300.0));
        p.add_effect(0, EffectKind::Explosion, 5.0).expect("effect");
        assert!((p.duration() - 13.5).abs() < 1e-6, "длительность {}", p.duration());
        p.save(&dir.join("test_new.lumicut")).expect("save");

        let mut settings = ExportSettings::new(dir.join("out_new.mp4"));
        settings.height = Some(360);
        let job = start(&p, &settings).expect("start");
        let mut result = None;
        while let Ok(msg) = job.rx.recv() {
            match msg {
                ExportMsg::Progress(_) => {}
                ExportMsg::Done => result = Some(Ok(())),
                ExportMsg::Failed(e) => result = Some(Err(e)),
            }
        }
        assert_eq!(result, Some(Ok(())));

        let out = ffmpeg::probe(&settings.path).expect("probe out");
        assert_eq!((out.width, out.height), (640, 360));
        assert!((out.duration - 13.5).abs() < 0.3, "длительность файла {}", out.duration);
        // Временные файлы эффектов убраны.
        let prefix = format!("lumicut-{}-", std::process::id());
        let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert!(left.is_empty(), "остались временные файлы: {left:?}");
    }

    /// Экспорт клипов с закреплёнными значениями и проверка готовых кадров и звука.
    /// Запуск: `cargo test -- --ignored export_pinned_values`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn export_pinned_values() {
        let dir = std::env::temp_dir().join(format!("lumicut-pinned-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let make = |source: &[&str], name: &str| {
            let path = dir.join(name);
            let done = ffmpeg::ffmpeg().args(["-y", "-v", "error", "-f", "lavfi"]).args(source).arg(&path).status();
            assert!(done.map_or(false, |s| s.success()), "{name}");
            path
        };
        let white = make(&["-i", "color=c=white:s=64x64", "-frames:v", "1"], "white.png");
        let tone = make(&["-i", "sine=frequency=440:duration=4"], "tone.wav");

        let mut p = Project::new();
        (p.width, p.height) = (320, 180);
        let mut clips = Vec::new();
        for path in [white, tone] {
            let info = ffmpeg::probe(&path).expect("probe");
            let id = p.alloc_id();
            p.media.push(MediaItem {
                id,
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                path,
                kind: info.kind,
                duration: info.duration,
                width: info.width,
                height: info.height,
                fps: info.fps,
                has_audio: info.has_audio,
            });
            clips.push(p.append_clip(id).expect("append"));
        }
        let (image, sound) = (clips[0], clips[1]);
        p.trim_end(image, 4.0);
        // Белый квадрат за первые две секунды проявляется от половины до полной непрозрачности,
        // а за следующие две уменьшается вдвое, уезжает вправо и поворачивается; звук всё это время
        // нарастает с нуля.
        let reach = p.key_reach();
        let clip = p.clip_mut(image).unwrap();
        clip.opacity = 0.5;
        clip.toggle_key(Prop::Opacity, 0.0, reach);
        clip.set_value(Prop::Opacity, 2.0, 1.0, reach);
        for (prop, to) in [(Prop::Scale, 0.5), (Prop::PosX, 60.0), (Prop::Rotation, 90.0)] {
            clip.toggle_key(prop, 2.0, reach);
            clip.set_value(prop, 4.0, to, reach);
        }
        let clip = p.clip_mut(sound).unwrap();
        clip.volume = 0.0;
        clip.toggle_key(Prop::Volume, 0.0, reach);
        clip.set_value(Prop::Volume, 4.0, 1.0, reach);

        let settings = ExportSettings::new(dir.join("out.mp4"));
        let job = start(&p, &settings).expect("start");
        let mut result = None;
        while let Ok(msg) = job.rx.recv() {
            match msg {
                ExportMsg::Progress(_) => {}
                ExportMsg::Done => result = Some(Ok(())),
                ExportMsg::Failed(e) => result = Some(Err(e)),
            }
        }
        assert_eq!(result, Some(Ok(())));

        // Яркость точки готового кадра в момент `t`.
        let frame = |t: f64| {
            let out = ffmpeg::ffmpeg()
                .args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"])
                .arg(&settings.path)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
                .output()
                .expect("кадр");
            assert_eq!(out.stdout.len(), 320 * 180, "кадр в момент {t}");
            move |x: usize, y: usize| out.stdout[y * 320 + x]
        };
        // Первая секунда: квадрат 180×180 посередине виден на три четверти.
        let at = frame(1.0);
        assert!((176..=206).contains(&at(160, 90)), "середина {}", at(160, 90));
        assert!(at(20, 90) < 40, "вне квадрата {}", at(20, 90));
        // Третья секунда: квадрат со стороной 135 сдвинут на 30 вправо и повёрнут на 45° —
        // это ромб с полудиагональю 95 вокруг точки (190, 90).
        let at = frame(3.0);
        assert!(at(190, 90) > 200, "середина {}", at(190, 90));
        assert!(at(190, 175) > 200, "нижний угол ромба {}", at(190, 175));
        assert!(at(275, 90) > 200, "правый угол ромба {}", at(275, 90));
        assert!(at(250, 150) < 40, "за стороной ромба {}", at(250, 150));
        assert!(at(60, 90) < 40, "слева от ромба {}", at(60, 90));

        // Звук: к концу он заметно громче, чем в начале, но и в начале не молчит.
        let out = ffmpeg::ffmpeg()
            .args(["-v", "error", "-i"])
            .arg(&settings.path)
            .args(["-vn", "-f", "f32le", "-ac", "1", "-ar", "8000", "-"])
            .output()
            .expect("звук");
        let samples: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let rms = |from: f64, to: f64| {
            let part = &samples[(from * 8000.0) as usize..(to * 8000.0) as usize];
            (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt()
        };
        // Громкость 10% на 0,4 секунды и 85% на 3,4 секунды.
        let (early, late) = (rms(0.2, 0.6), rms(3.2, 3.6));
        assert!(early > 0.0 && (6.0..11.0).contains(&(late / early)), "в начале {early}, в конце {late}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Экспорт надписи, фильтра и хромакея процессором и видеокартой (если она есть) с проверкой кадров.
    /// Запуск: `cargo test -- --ignored export_text_and_colour`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn export_text_and_colour() {
        let dir = std::env::temp_dir().join(format!("lumicut-colour-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::new();
        (p.width, p.height) = (320, 180);
        // Красный квадрат с чёрно-белым фильтром, за ним зелёный с хромакеем; сверху надпись.
        for (name, colour) in [("red.png", "red"), ("green.png", "0x00FF00")] {
            let path = dir.join(name);
            let source = format!("color=c={colour}:s=64x64");
            let made = ffmpeg::ffmpeg().args(["-y", "-v", "error", "-f", "lavfi", "-i", &source, "-frames:v", "1"]).arg(&path).status();
            assert!(made.map_or(false, |s| s.success()), "{name}");
            let id = p.alloc_id();
            p.media.push(MediaItem {
                id,
                path,
                name: name.into(),
                kind: MediaKind::Image,
                duration: 0.0,
                width: 64,
                height: 64,
                fps: 0.0,
                has_audio: false,
            });
            let clip = p.append_clip(id).unwrap();
            p.trim_end(clip, p.clip(clip).unwrap().start + 2.0);
        }
        let (red, green) = (p.tracks[1].clips[0].id, p.tracks[1].clips[1].id);
        p.clip_mut(red).unwrap().look.filter = crate::look::FilterKind::Mono;
        p.clip_mut(green).unwrap().look.key = Some(crate::look::ChromaKey::default());
        let title = p.add_text(0, crate::text::TextPreset::Title.text(p.height), 0.0, 4.0).unwrap();
        p.clip_mut(title).unwrap().pos_y = -60.0;

        let works = |gpu: &Gpu| {
            let probe = ffmpeg::ffmpeg()
                .args(["-v", "error", "-f", "lavfi", "-i", "color=c=black:s=256x144:r=30:d=0.2", "-c:v", gpu.encoders().0, "-f", "null", "-"])
                .stderr(Stdio::null())
                .status();
            probe.map_or(false, |s| s.success())
        };
        let card = [Gpu::Nvidia, Gpu::Intel, Gpu::Amd].into_iter().find(works);
        println!("видеокарта для кодирования: {card:?}");
        for gpu in [None, card] {
            let settings = ExportSettings { gpu, ..ExportSettings::new(dir.join("out.mp4")) };
            let job = start(&p, &settings).expect("start");
            let mut result = None;
            while let Ok(msg) = job.rx.recv() {
                match msg {
                    ExportMsg::Progress(_) => {}
                    ExportMsg::Done => result = Some(Ok(())),
                    ExportMsg::Failed(e) => result = Some(Err(e)),
                }
            }
            assert_eq!(result, Some(Ok(())), "кодирует {gpu:?}");

            let frame = |t: f64| {
                let out = ffmpeg::ffmpeg()
                    .args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"])
                    .arg(&settings.path)
                    .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                    .output()
                    .expect("кадр");
                assert_eq!(out.stdout.len(), 320 * 180 * 3, "кадр в момент {t}");
                move |x: usize, y: usize| -> [u8; 3] { out.stdout[(y * 320 + x) * 3..][..3].try_into().unwrap() }
            };
            let bright = |at: &dyn Fn(usize, usize) -> [u8; 3]| (100..220).flat_map(|x| (18..44).map(move |y| (x, y))).any(|(x, y)| at(x, y)[1] > 200);
            // Первая секунда: красный стал серым той же яркости, поверх него белая надпись.
            let at = frame(1.0);
            let [r, g, b] = at(160, 110);
            assert!(r.abs_diff(g) < 12 && g.abs_diff(b) < 12 && (35..80).contains(&g), "серый вместо красного: {:?}", [r, g, b]);
            assert!(bright(&at), "надпись над серым квадратом");
            // Третья секунда: зелёный фон стал прозрачным, виден чёрный кадр и та же надпись.
            let at = frame(3.0);
            assert!(at(160, 110).iter().all(|c| *c < 30), "чёрный вместо зелёного: {:?}", at(160, 110));
            assert!(bright(&at), "надпись над прозрачным клипом");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Экспорт клипов с эффектами, маской и режимами наложения и проверка готовых кадров.
    /// Запуск: `cargo test -- --ignored export_clip_effects`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn export_clip_effects() {
        use crate::vfx::{Fx, FxKind, Mask, MaskShape};
        let dir = std::env::temp_dir().join(format!("lumicut-vfx-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::new();
        (p.width, p.height) = (320, 180);
        let image = |p: &mut Project, name: &str, source: &str| {
            let path = dir.join(name);
            let made = ffmpeg::ffmpeg().args(["-y", "-v", "error", "-f", "lavfi", "-i", source, "-frames:v", "1"]).arg(&path).status();
            assert!(made.map_or(false, |s| s.success()), "{name}");
            let id = p.alloc_id();
            p.media.push(MediaItem {
                id,
                path,
                name: name.into(),
                kind: MediaKind::Image,
                duration: 0.0,
                width: 64,
                height: 64,
                fps: 0.0,
                has_audio: false,
            });
            id
        };
        // Внизу синий квадрат на все шесть секунд, сверху по две секунды: серый с маской и сложением,
        // чёрно-белый в режиме «Экран» и серый со всеми эффектами сразу.
        let blue = image(&mut p, "blue.png", "color=c=0x0000FF:s=64x64");
        let grey = image(&mut p, "grey.png", "color=c=0x808080:s=64x64");
        let halves = image(&mut p, "halves.png", "color=c=black:s=64x64,drawbox=x=32:y=0:w=32:h=64:color=white:t=fill");
        let base = p.append_clip(blue).unwrap();
        p.trim_end(base, 6.0);
        let top = |p: &mut Project, media, start: f64| {
            let id = p.add_clip(0, media, start).unwrap();
            p.trim_end(id, start + 2.0);
            id
        };
        let (added, screened, stacked) = (top(&mut p, grey, 0.0), top(&mut p, halves, 2.0), top(&mut p, grey, 4.0));
        let clip = p.clip_mut(added).unwrap();
        clip.vfx.blend = Blend::Add;
        clip.vfx.mask = Some(Mask { feather: 0.0, ..Mask::new(MaskShape::Ellipse) });
        p.clip_mut(screened).unwrap().vfx.blend = Blend::Screen;
        let clip = p.clip_mut(stacked).unwrap();
        clip.vfx.stack = FxKind::ALL.map(Fx::new).to_vec();
        clip.vfx.crop = [0.5, 0.0, 0.0, 0.0];

        let settings = ExportSettings::new(dir.join("out.mp4"));
        let job = start(&p, &settings).expect("start");
        let mut result = None;
        while let Ok(msg) = job.rx.recv() {
            match msg {
                ExportMsg::Progress(_) => {}
                ExportMsg::Done => result = Some(Ok(())),
                ExportMsg::Failed(e) => result = Some(Err(e)),
            }
        }
        assert_eq!(result, Some(Ok(())));

        let frame = |t: f64| {
            let out = ffmpeg::ffmpeg()
                .args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"])
                .arg(&settings.path)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .expect("кадр");
            assert_eq!(out.stdout.len(), 320 * 180 * 3, "кадр в момент {t}");
            move |x: usize, y: usize| -> [u8; 3] { out.stdout[(y * 320 + x) * 3..][..3].try_into().unwrap() }
        };
        let near = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 14);
        // Квадрат клипа 180×180 стоит посередине кадра: по горизонтали с 70-й по 250-ю точку.
        // Первая секунда: внутри эллипса серый прибавился к синему, в углу квадрата синий остался как был.
        let at = frame(1.0);
        assert!(near(at(160, 90), [128, 128, 255]), "сложение в середине: {:?}", at(160, 90));
        assert!(near(at(80, 10), [0, 0, 255]), "вне маски: {:?}", at(80, 10));
        assert!(near(at(20, 90), [0, 0, 0]), "вне клипов: {:?}", at(20, 90));
        // Третья секунда: чёрная половина пропала, белая осталась белой.
        let at = frame(3.0);
        assert!(near(at(110, 90), [0, 0, 255]), "чёрное в режиме «Экран»: {:?}", at(110, 90));
        assert!(near(at(210, 90), [255, 255, 255]), "белое в режиме «Экран»: {:?}", at(210, 90));
        // Пятая секунда: все эффекты разом — граф собрался, левая половина срезана кадрированием.
        let at = frame(5.0);
        assert!(near(at(110, 90), [0, 0, 255]), "срезанная половина: {:?}", at(110, 90));
        assert!(!near(at(210, 90), [0, 0, 255]), "оставшаяся половина: {:?}", at(210, 90));

        // Временные файлы масок убраны.
        let prefix = format!("lumicut-{}-", std::process::id());
        let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(&prefix) && name.contains("matte")
            })
            .collect();
        assert!(left.is_empty(), "остались временные файлы: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_timeline_is_an_error() {
        let p = Project::new();
        assert!(plan(&p, &ExportSettings::new(PathBuf::from("out.mp4"))).is_err());
    }
}
