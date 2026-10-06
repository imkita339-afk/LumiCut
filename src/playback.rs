//! Предпросмотр видео: кадры декодирует ffmpeg, сюда они приходят по каналу.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::effects::{self, Effect};
use crate::ffmpeg;
use crate::keys::Prop;
use crate::model::{fit_size, Clip, Id, MediaItem, MediaKind, Project, TrackKind};
use crate::text::{self, Text};
use crate::vfx::{self, Blend, Mattes, Recipe};

/// Предел размера кадра предпросмотра.
const PREVIEW_MAX_W: f32 = 960.0;
const PREVIEW_MAX_H: f32 = 540.0;

/// Слой кадра: текстура клипа и то, как он стоит в кадре в этот момент
/// (с учётом закреплённых значений, движения по пути и переходов).
pub struct Layer {
    pub clip: Id,
    pub texture: TextureId,
    /// Размер клипа в пикселях проекта при масштабе 100%.
    pub base_w: f32,
    pub base_h: f32,
    pub scale: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub rotation: f32,
    pub opacity: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Белая вспышка перехода поверх кадра, 0..1.
    pub flash: f32,
}

/// Надпись, лежащая в текстуре, и её размер в пикселях проекта.
struct TextFrame {
    text: Text,
    blend: Blend,
    texture: TextureHandle,
    w: u32,
    h: u32,
}

/// Кадр процедурного эффекта, лежащий в текстуре.
struct EffectFrame {
    effect: Effect,
    blend: Blend,
    progress: f32,
    texture: TextureHandle,
}

#[derive(Clone, PartialEq)]
struct Signature {
    media: Id,
    offset: f64,
    speed: f64,
    clip_start: f64,
    fps: f64,
    w: u32,
    h: u32,
}

/// Кадр от декодера: исходный и, если у клипа есть оформление, уже готовый к показу.
struct Frame {
    raw: Vec<u8>,
    /// Готовый кадр и номер оформления, с которым он сделан.
    cooked: Option<(u64, Vec<u8>)>,
}

/// Оформление клипа и его номер: поток декодера готовит по нему кадры заранее,
/// чтобы окно при воспроизведении только показывало их.
type Shared = Arc<Mutex<(u64, Recipe)>>;

struct Decoder {
    child: Child,
    rx: Receiver<Frame>,
    sig: Signature,
    /// Время на таймлайне, которому соответствует кадр 0.
    start_time: f64,
    /// Номер кадра, который придёт из канала следующим.
    next_index: u64,
    /// Номер кадра, который сейчас в текстуре.
    shown: Option<u64>,
    /// Этот кадр, каким он пришёл от ffmpeg, и оформление, с которым он показан: когда фильтр
    /// или эффект меняют на паузе, кадр готовится заново без нового декодирования.
    raw: Vec<u8>,
    recipe: Recipe,
    /// Оформление, которое нужно сейчас; то же самое знает поток декодера.
    wanted: Recipe,
    version: u64,
    shared: Shared,
    mattes: Mattes,
    started: Instant,
    eof: bool,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Decoder {
    /// Сообщает потоку декодера новое оформление клипа.
    fn restyle(&mut self, recipe: &Recipe) {
        if self.wanted == *recipe {
            return;
        }
        self.version += 1;
        self.wanted = recipe.clone();
        if let Ok(mut shared) = self.shared.lock() {
            *shared = (self.version, recipe.clone());
        }
    }

    /// Кладёт пришедший кадр в текстуру. Готовый кадр годится, если сделан с нынешним оформлением,
    /// иначе он готовится здесь.
    fn show(&mut self, textures: &mut HashMap<Id, TextureHandle>, ctx: &egui::Context, clip: Id, frame: Frame, still: bool) {
        self.raw = frame.raw;
        match frame.cooked {
            Some((version, cooked)) if version == self.version && !self.wanted.is_plain() => {
                upload(textures, ctx, clip, self.sig.w, self.sig.h, &cooked, false);
                self.recipe = self.wanted.clone();
            }
            _ => self.reshow(textures, ctx, clip, still),
        }
    }

    /// Показывает последний пришедший кадр с нынешним оформлением.
    fn reshow(&mut self, textures: &mut HashMap<Id, TextureHandle>, ctx: &egui::Context, clip: Id, still: bool) {
        let (w, h) = (self.sig.w, self.sig.h);
        if self.wanted.is_plain() {
            // У картинок бывает своя прозрачность, видео непрозрачно.
            upload(textures, ctx, clip, w, h, &self.raw, still);
        } else {
            let cooked = self.wanted.cook(&self.raw, w as usize, h as usize, self.shown.unwrap_or(0), &mut self.mattes);
            upload(textures, ctx, clip, w, h, &cooked, false);
        }
        self.recipe = self.wanted.clone();
    }
}

fn spawn(
    media: &MediaItem,
    src_time: f64,
    speed: f64,
    fps: f64,
    w: u32,
    h: u32,
    shared: Shared,
) -> Option<(Child, Receiver<Frame>)> {
    let still = media.kind == MediaKind::Image;
    let mut cmd = ffmpeg::ffmpeg();
    cmd.args(["-v", "error"]);
    if !still {
        cmd.args(["-ss", &format!("{:.4}", src_time.max(0.0))]);
    }
    cmd.arg("-i").arg(&media.path);
    let vf = if still {
        cmd.args(["-frames:v", "1"]);
        format!("scale={w}:{h}:flags=bilinear")
    } else {
        format!("setpts=PTS/{speed:.4},fps={fps:.4},scale={w}:{h}:flags=bilinear")
    };
    cmd.args(["-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx) = sync_channel::<Frame>(3);
    let size = w as usize * h as usize * 4;
    std::thread::spawn(move || {
        let mut mattes = Mattes::default();
        let mut index = 0u64;
        loop {
            let mut raw = vec![0u8; size];
            if out.read_exact(&mut raw).is_err() {
                break;
            }
            let Ok((version, recipe)) = shared.lock().map(|s| s.clone()) else { break };
            // Эффекты считаются здесь, а не в потоке окна: воспроизведение остаётся плавным.
            let cooked = (!recipe.is_plain()).then(|| (version, recipe.cook(&raw, w as usize, h as usize, index, &mut mattes)));
            index += 1;
            if tx.send(Frame { raw, cooked }).is_err() {
                break;
            }
        }
    });
    Some((child, rx))
}

/// Кладёт кадр в текстуру клипа. `straight` — у кадра обычная прозрачность, иначе цвет уже умножен на неё.
fn upload(
    textures: &mut HashMap<Id, TextureHandle>,
    ctx: &egui::Context,
    clip: Id,
    w: u32,
    h: u32,
    buf: &[u8],
    straight: bool,
) {
    let size = [w as usize, h as usize];
    let img = if straight {
        ColorImage::from_rgba_unmultiplied(size, buf)
    } else {
        ColorImage::from_rgba_premultiplied(size, buf)
    };
    match textures.get_mut(&clip) {
        Some(tex) => tex.set(img, TextureOptions::LINEAR),
        None => {
            let tex = ctx.load_texture(format!("clip-{clip}"), img, TextureOptions::LINEAR);
            textures.insert(clip, tex);
        }
    }
}

/// Картинка эффекта или надписи с учётом режима наложения клипа.
fn blended(size: [usize; 2], rgba: Vec<u8>, blend: Blend) -> ColorImage {
    if blend == Blend::Normal {
        return ColorImage::from_rgba_unmultiplied(size, &rgba);
    }
    let mut buf = rgba;
    vfx::premultiply(&mut buf, size[0], blend);
    ColorImage::from_rgba_premultiplied(size, &buf)
}

pub struct Preview {
    decoders: HashMap<Id, Decoder>,
    textures: HashMap<Id, TextureHandle>,
    effects: HashMap<Id, EffectFrame>,
    texts: HashMap<Id, TextFrame>,
    pub enabled: bool,
    /// Клип, у которого вместо картинки показана его прозрачность.
    pub matte: Option<Id>,
    /// Клип, который показан без оформления: для сравнения «до и после».
    pub original: Option<Id>,
}

impl Preview {
    pub fn new(enabled: bool) -> Self {
        Self {
            decoders: HashMap::new(),
            textures: HashMap::new(),
            effects: HashMap::new(),
            texts: HashMap::new(),
            enabled,
            matte: None,
            original: None,
        }
    }

    pub fn clear(&mut self) {
        self.decoders.clear();
        self.textures.clear();
        self.effects.clear();
        self.texts.clear();
    }

    /// Что нужно сделать с кадрами клипа перед показом.
    fn recipe(&self, clip: &Clip) -> Recipe {
        if self.original == Some(clip.id) {
            return Recipe::default();
        }
        Recipe { look: clip.look.clone(), vfx: clip.vfx.clone(), matte: self.matte == Some(clip.id) }
    }

    /// Цвет исходного кадра клипа в точке (`u`, `v`) — доли ширины и высоты; среднее по квадрату 3×3.
    pub fn pick(&self, clip: Id, u: f32, v: f32) -> Option<[u8; 3]> {
        let d = self.decoders.get(&clip)?;
        let (w, h) = (d.sig.w as i32, d.sig.h as i32);
        let inside = (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v);
        if !inside || d.raw.len() != (w * h * 4) as usize {
            return None;
        }
        let (cx, cy) = ((u * w as f32) as i32, (v * h as f32) as i32);
        let (mut sum, mut n) = ([0u32; 3], 0u32);
        for y in (cy - 1).max(0)..=(cy + 1).min(h - 1) {
            for x in (cx - 1).max(0)..=(cx + 1).min(w - 1) {
                let p = &d.raw[((y * w + x) * 4) as usize..][..3];
                for (s, c) in sum.iter_mut().zip(p) {
                    *s += *c as u32;
                }
                n += 1;
            }
        }
        Some(sum.map(|s| ((s + n / 2) / n) as u8))
    }

    /// Готовит кадры для момента `t` и возвращает слои снизу вверх.
    pub fn update(&mut self, ctx: &egui::Context, project: &Project, t: f64, playing: bool) -> Vec<Layer> {
        let mut layers = Vec::new();
        if !self.enabled {
            return layers;
        }
        let fps = project.fps.max(1.0);
        let (pw, _) = fit_size(project.width as f32, project.height as f32, PREVIEW_MAX_W, PREVIEW_MAX_H);
        let k = (pw / project.width as f32).min(1.0);
        let mut wanted: Vec<Id> = Vec::new();

        for (ti, track) in project.tracks.iter().enumerate().rev() {
            if track.kind != TrackKind::Video || !track.enabled {
                continue;
            }
            for clip in &track.clips {
                // Пока идёт переход, клип виден и за своими границами: там он замирает на крайнем кадре.
                let (lead, tail) = project.overhang(ti, clip);
                let (from, to) = (clip.start - lead, clip.end() + tail);
                let visible = t >= from - 1e-6 && t < to - 1e-6;
                let upcoming = playing && from > t && from <= t + 1.0;
                if !visible && !upcoming {
                    continue;
                }
                let time = if t < clip.start {
                    clip.start
                } else if t >= clip.end() {
                    // Последний кадр клипа по сетке кадров, считая от его начала.
                    clip.start + ((clip.duration * fps - 1e-6).ceil() - 1.0).max(0.0) / fps
                } else {
                    t
                };
                let (texture, bw, bh) = if let Some(effect) = &clip.effect {
                    let side = project.width.min(project.height) as f32;
                    (Some(self.effect_frame(ctx, clip, effect, time, fps)), side, side)
                } else if let Some(text) = &clip.text {
                    let (texture, w, h) = self.text_frame(ctx, clip, text);
                    (Some(texture), w, h)
                } else {
                    let Some(media) = project.media(clip.media) else { continue };
                    if media.kind == MediaKind::Audio {
                        continue;
                    }
                    let (bw, bh) = fit_size(
                        media.width as f32,
                        media.height as f32,
                        project.width as f32,
                        project.height as f32,
                    );
                    let (mut dw, mut dh) = (ffmpeg::even(bw * k), ffmpeg::even(bh * k));
                    if dw > media.width {
                        dw = ffmpeg::even(media.width as f32);
                        dh = ffmpeg::even(media.height as f32);
                    }
                    self.sync(ctx, clip, media, time, fps, dw, dh);
                    (self.textures.get(&clip.id).map(|tex| tex.id()), bw, bh)
                };
                wanted.push(clip.id);
                if let (true, Some(texture)) = (visible, texture) {
                    let tweak = project.tweak(ti, clip, t);
                    let (x, y) = clip.position(t);
                    layers.push(Layer {
                        clip: clip.id,
                        texture,
                        base_w: bw,
                        base_h: bh,
                        scale: clip.value(Prop::Scale, t) * tweak.scale,
                        pos_x: x + tweak.dx,
                        pos_y: y + tweak.dy,
                        rotation: clip.value(Prop::Rotation, t),
                        opacity: clip.value(Prop::Opacity, t) * tweak.alpha,
                        flip_h: clip.flip_h,
                        flip_v: clip.flip_v,
                        flash: tweak.flash,
                    });
                }
            }
        }
        self.decoders.retain(|id, _| wanted.contains(id));
        self.textures.retain(|id, _| wanted.contains(id));
        self.effects.retain(|id, _| wanted.contains(id));
        self.texts.retain(|id, _| wanted.contains(id));
        layers
    }

    /// Текстура с надписью и её размер; надпись перерисовывается, только когда её изменили.
    fn text_frame(&mut self, ctx: &egui::Context, clip: &Clip, text: &Text) -> (TextureId, f32, f32) {
        let (id, blend) = (clip.id, clip.vfx.blend);
        if let Some(f) = self.texts.get(&id).filter(|f| f.text == *text && f.blend == blend) {
            return (f.texture.id(), f.w as f32, f.h as f32);
        }
        let raster = text::render(text);
        let image = blended([raster.w as usize, raster.h as usize], raster.rgba, blend);
        let texture = match self.texts.remove(&id) {
            Some(mut f) => {
                f.texture.set(image, TextureOptions::LINEAR);
                f.texture
            }
            None => ctx.load_texture(format!("text-{id}"), image, TextureOptions::LINEAR),
        };
        let shown = texture.id();
        self.texts.insert(id, TextFrame { text: text.clone(), blend, texture, w: raster.w, h: raster.h });
        (shown, raster.w as f32, raster.h as f32)
    }

    /// Текстура с кадром эффекта для момента `t`; кадр пересчитывается, только когда
    /// сменился он сам или параметры эффекта.
    fn effect_frame(&mut self, ctx: &egui::Context, clip: &Clip, effect: &Effect, t: f64, fps: f64) -> TextureId {
        let frames = effects::frame_count(clip.duration, fps);
        let index = (((t - clip.start) * fps + 1e-3).floor().max(0.0) as u32).min(frames - 1);
        let progress = index as f32 / frames as f32;
        let blend = clip.vfx.blend;
        if let Some(f) = self.effects.get(&clip.id) {
            if f.progress == progress && f.blend == blend && f.effect == *effect {
                return f.texture.id();
            }
        }
        let side = effects::side(effect) as usize;
        let image = blended([side, side], effects::render(effect, progress), blend);
        // Без сглаживания: «пиксели» эффекта остаются чёткими.
        match self.effects.get_mut(&clip.id) {
            Some(f) => {
                f.texture.set(image, TextureOptions::NEAREST);
                f.effect = effect.clone();
                f.blend = blend;
                f.progress = progress;
                f.texture.id()
            }
            None => {
                let texture = ctx.load_texture(format!("effect-{}", clip.id), image, TextureOptions::NEAREST);
                let id = texture.id();
                self.effects.insert(clip.id, EffectFrame { effect: effect.clone(), blend, progress, texture });
                id
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sync(&mut self, ctx: &egui::Context, clip: &Clip, media: &MediaItem, t: f64, fps: f64, w: u32, h: u32) {
        let still = media.kind == MediaKind::Image;
        let sig = Signature {
            media: clip.media,
            offset: if still { 0.0 } else { clip.offset },
            speed: if still { 1.0 } else { clip.speed },
            clip_start: if still { 0.0 } else { clip.start },
            fps,
            w,
            h,
        };
        let recipe = self.recipe(clip);
        let target_of = |d: &Decoder| ((t - d.start_time) * fps + 1e-3).floor() as i64;

        let restart = match self.decoders.get(&clip.id) {
            None => true,
            Some(d) if d.sig != sig => true,
            Some(_) if still => false,
            Some(d) => {
                let target = target_of(d);
                let behind = target < 0 || d.shown.map_or(false, |s| target < s as i64);
                let far = !d.eof && target > d.next_index as i64 + (fps * 1.5) as i64;
                behind || far
            }
        };

        if restart {
            // При перемотке даём свежему декодеру выдать первый кадр, а не плодим процессы.
            if let Some(d) = self.decoders.get_mut(&clip.id) {
                if d.shown.is_none() && !d.eof && d.started.elapsed() < Duration::from_millis(300) {
                    d.restyle(&recipe);
                    match d.rx.try_recv() {
                        Ok(frame) => {
                            d.shown = Some(0);
                            d.next_index = 1;
                            d.show(&mut self.textures, ctx, clip.id, frame, still);
                        }
                        Err(TryRecvError::Disconnected) => d.eof = true,
                        Err(TryRecvError::Empty) => {}
                    }
                    return;
                }
            }
            let local = (t - clip.start).max(0.0);
            let start_time = if still { clip.start } else { clip.start + (local * fps + 1e-3).floor() / fps };
            let src = clip.offset + (start_time - clip.start) * clip.speed;
            self.decoders.remove(&clip.id);
            let shared: Shared = Arc::new(Mutex::new((0, recipe.clone())));
            if let Some((child, rx)) = spawn(media, src, clip.speed, fps, w, h, shared.clone()) {
                self.decoders.insert(
                    clip.id,
                    Decoder {
                        child,
                        rx,
                        sig,
                        start_time,
                        next_index: 0,
                        shown: None,
                        raw: Vec::new(),
                        recipe: recipe.clone(),
                        wanted: recipe,
                        version: 0,
                        shared,
                        mattes: Mattes::default(),
                        started: Instant::now(),
                        eof: false,
                    },
                );
            }
            return;
        }

        let Some(d) = self.decoders.get_mut(&clip.id) else { return };
        d.restyle(&recipe);
        let target = if still { 0 } else { target_of(d).max(0) as u64 };
        let mut latest: Option<Frame> = None;
        while d.next_index <= target {
            match d.rx.try_recv() {
                Ok(frame) => {
                    latest = Some(frame);
                    d.next_index += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    d.eof = true;
                    break;
                }
            }
        }
        if let Some(frame) = latest {
            d.shown = Some(d.next_index - 1);
            d.show(&mut self.textures, ctx, clip.id, frame, still);
        } else if d.recipe != d.wanted && !d.raw.is_empty() {
            d.reshow(&mut self.textures, ctx, clip.id, still);
        }
    }

    /// Ждёт ли предпросмотр кадров от декодеров (нужна перерисовка).
    pub fn pending(&self) -> bool {
        self.decoders.values().any(|d| d.shown.is_none() && !d.eof)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use eframe::egui::{Color32, ImageData, RawInput};

    use super::*;
    use crate::look::{ChromaKey, FilterKind};
    use crate::vfx::{Fx, FxKind};

    /// Предпросмотр настоящего файла: кадр декодируется, а фильтр, хромакей и эффекты меняют его
    /// сразу, без нового декодирования. Запуск: `cargo test -- --ignored preview_shows_frames_with_the_filter`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn preview_shows_frames_with_the_filter() {
        let dir = std::env::temp_dir().join(format!("lumicut-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join("clip.mp4");
        let made = ffmpeg::ffmpeg()
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=2", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status();
        assert!(made.map_or(false, |s| s.success()));

        let mut p = Project::new();
        let media = p.alloc_id();
        p.media.push(MediaItem {
            id: media,
            path,
            name: "clip.mp4".into(),
            kind: MediaKind::Video,
            duration: 2.0,
            width: 320,
            height: 180,
            fps: 30.0,
            has_audio: false,
        });
        let clip = p.append_clip(media).unwrap();

        let ctx = egui::Context::default();
        let mut preview = Preview::new(true);
        // Картинка, которую предпросмотр положил в текстуру клипа за один проход (если положил).
        let pass = |preview: &mut Preview, p: &Project| -> Option<Vec<Color32>> {
            let out = ctx.run(RawInput::default(), |ctx| {
                preview.update(ctx, p, 0.5, false);
            });
            out.textures_delta.set.into_iter().find_map(|(_, delta)| match delta.image {
                ImageData::Color(image) => Some(image.pixels.clone()),
                ImageData::Font(_) => None,
            })
        };
        // Курсор стоит на 15-м кадре: ждём, пока декодер до него дойдёт, дальше кадры не меняются.
        let mut plain = None;
        for _ in 0..400 {
            if let Some(frame) = pass(&mut preview, &p) {
                plain = Some(frame);
            }
            if preview.decoders.values().any(|d| d.shown == Some(15)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let plain = plain.expect("кадр предпросмотра");
        assert_eq!(plain.len(), 320 * 180);
        assert!(plain.iter().any(|c| c.r().abs_diff(c.g()) > 60), "исходный кадр цветной");
        assert!(pass(&mut preview, &p).is_none(), "на паузе кадр не перезаливается");

        // Чёрно-белый фильтр: тот же кадр перекрашен в следующем же проходе.
        p.clip_mut(clip).unwrap().look.filter = FilterKind::Mono;
        let grey = pass(&mut preview, &p).expect("кадр перекрашен сразу");
        assert!(grey.iter().all(|c| c.r() == c.g() && c.g() == c.b()), "кадр серый");
        assert!(pass(&mut preview, &p).is_none(), "без правок кадр не перезаливается");

        // Хромакей по цвету точки исходного кадра делает эту точку прозрачной.
        let at = 40 * 320 + 40;
        let key = ChromaKey { color: [plain[at].r(), plain[at].g(), plain[at].b()], similarity: 0.1, blend: 0.0, ..ChromaKey::default() };
        let look = &mut p.clip_mut(clip).unwrap().look;
        (look.filter, look.key) = (FilterKind::None, Some(key));
        let keyed = pass(&mut preview, &p).expect("кадр перекрашен сразу");
        assert_eq!(keyed[at].a(), 0);
        assert!(keyed.iter().any(|c| c.a() == 255), "остальное осталось видимым");
        // Пипетка берёт цвет исходного кадра, а не того, что получился после хромакея.
        let picked = preview.pick(clip, 40.5 / 320.0, 40.5 / 180.0).expect("цвет под пипеткой");
        let around: Vec<Color32> = (39..=41).flat_map(|y| (39..=41).map(move |x| y * 320 + x)).map(|i| plain[i]).collect();
        let mean = |f: fn(&Color32) -> u8| (around.iter().map(|c| f(c) as u32).sum::<u32>() as f32 / 9.0).round() as u8;
        assert_eq!(picked, [mean(Color32::r), mean(Color32::g), mean(Color32::b)]);
        assert!(preview.pick(clip, 1.5, 0.5).is_none());

        // Эффект и режимы просмотра тоже применяются к уже показанному кадру.
        p.clip_mut(clip).unwrap().vfx.stack.push(Fx::new(FxKind::Invert));
        let inverted = pass(&mut preview, &p).expect("эффект виден сразу");
        assert_ne!(inverted, keyed);
        preview.matte = Some(clip);
        let matte = pass(&mut preview, &p).expect("прозрачность показана серым");
        assert_eq!((matte[at], matte[at].a()), (Color32::BLACK, 255));
        assert!(matte.iter().all(|c| c.r() == c.g() && c.g() == c.b()));
        preview.matte = None;
        preview.original = Some(clip);
        assert_eq!(pass(&mut preview, &p).expect("клип без оформления"), plain);

        // Поток декодера готовит кадры сам: после перемотки они приходят уже с оформлением.
        preview.original = None;
        let later = |preview: &mut Preview| {
            for _ in 0..400 {
                let out = ctx.run(RawInput::default(), |ctx| {
                    preview.update(ctx, &p, 1.5, false);
                });
                let frame = out.textures_delta.set.into_iter().find_map(|(_, delta)| match delta.image {
                    ImageData::Color(image) => Some(image.pixels.clone()),
                    ImageData::Font(_) => None,
                });
                if frame.is_some() {
                    return frame;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            None
        };
        let moved = later(&mut preview).expect("кадр после перемотки");
        assert!(moved.iter().any(|c| c.a() == 0) && moved.iter().any(|c| c.a() == 255), "хромакей действует и на новых кадрах");

        drop(preview);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
