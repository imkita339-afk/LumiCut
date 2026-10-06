//! Модель проекта: медиафайлы, дорожки, клипы и операции над ними.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::effects::{Effect, EffectKind};
use crate::keys::{Keys, Prop};
use crate::look::Look;
use crate::motion::Motion;
use crate::text::Text;
use crate::transition::{Transition, TransitionKind, Tweak};
use crate::vfx::Vfx;

pub type Id = u64;

/// Минимальная длина клипа на таймлайне, секунды.
pub const MIN_CLIP: f64 = 0.05;
const EPS: f64 = 1e-6;
/// Клипы считаются стоящими вплотную, если зазор между ними меньше этого, секунды.
const ADJACENT: f64 = 1e-3;

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: Id,
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    /// Длительность в секундах (для изображений 0).
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub media: Id,
    /// Начало на таймлайне, секунды.
    pub start: f64,
    /// Смещение внутри исходного файла, секунды.
    pub offset: f64,
    /// Длительность на таймлайне, секунды.
    pub duration: f64,
    pub speed: f64,
    pub volume: f32,
    pub scale: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub rotation: f32,
    pub opacity: f32,
    /// Переход на стыке с предыдущим клипом дорожки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Движение по пути поверх положения клипа.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    /// Процедурный эффект: у такого клипа нет исходного файла.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    /// Значения свойств, закреплённые в моментах времени.
    #[serde(default, skip_serializing_if = "Keys::is_empty")]
    pub keys: Keys,
    /// Надпись: у такого клипа, как и у эффекта, нет исходного файла.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    /// Фильтр, цветокоррекция и хромакей.
    #[serde(default, skip_serializing_if = "Look::is_neutral")]
    pub look: Look,
    /// Эффекты, маска и режим наложения.
    #[serde(default, skip_serializing_if = "Vfx::is_empty")]
    pub vfx: Vfx,
    /// Отражение по горизонтали и по вертикали.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_h: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_v: bool,
}

/// На сколько прямых отрезков раскладывается плавный ход между двумя закреплёнными точками.
const EASE_STEPS: usize = 8;

impl Clip {
    fn new(id: Id, media: Id, start: f64, duration: f64) -> Self {
        Clip {
            id,
            media,
            start,
            offset: 0.0,
            duration,
            speed: 1.0,
            volume: 1.0,
            scale: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            rotation: 0.0,
            opacity: 1.0,
            transition: None,
            motion: None,
            effect: None,
            keys: Keys::default(),
            text: None,
            look: Look::default(),
            vfx: Vfx::default(),
            flip_h: false,
            flip_v: false,
        }
    }

    /// Клип без исходного файла: эффект или надпись.
    pub fn generated(&self) -> bool {
        self.effect.is_some() || self.text.is_some()
    }

    /// Изменена ли картинка клипа цветом, хромакеем, эффектами, маской или наложением.
    pub fn styled(&self) -> bool {
        !self.look.is_neutral() || !self.vfx.is_plain()
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// Пройденная доля клипа в момент `t`, 0..1.
    pub fn progress(&self, t: f64) -> f32 {
        ((t - self.start) / self.duration).clamp(0.0, 1.0) as f32
    }

    /// Положение центра клипа в момент `t` с учётом движения по пути.
    pub fn position(&self, t: f64) -> (f32, f32) {
        let (x, y) = (self.value(Prop::PosX, t), self.value(Prop::PosY, t));
        match self.motion.as_ref().and_then(|m| m.at(self.progress(t))) {
            Some(p) => (x + p[0], y + p[1]),
            None => (x, y),
        }
    }

    /// Момент таймлайна как время от начала клипа; за границами клипа это его край.
    fn local(&self, t: f64) -> f64 {
        (t - self.start).clamp(0.0, self.duration)
    }

    fn base(&self, prop: Prop) -> f32 {
        match prop {
            Prop::Scale => self.scale,
            Prop::PosX => self.pos_x,
            Prop::PosY => self.pos_y,
            Prop::Rotation => self.rotation,
            Prop::Opacity => self.opacity,
            Prop::Volume => self.volume,
        }
    }

    fn base_mut(&mut self, prop: Prop) -> &mut f32 {
        match prop {
            Prop::Scale => &mut self.scale,
            Prop::PosX => &mut self.pos_x,
            Prop::PosY => &mut self.pos_y,
            Prop::Rotation => &mut self.rotation,
            Prop::Opacity => &mut self.opacity,
            Prop::Volume => &mut self.volume,
        }
    }

    /// Значение свойства в момент `t`: заданное закреплёнными точками или обычное.
    pub fn value(&self, prop: Prop, t: f64) -> f32 {
        self.keys.of(prop).at(self.local(t), self.keys.is_eased(prop)).unwrap_or(self.base(prop))
    }

    /// Меняет значение свойства в момент `t`. У свойства с точками правится точка, стоящая
    /// в этом моменте, или ставится новая; без точек меняется обычное значение.
    pub fn set_value(&mut self, prop: Prop, t: f64, v: f32, tol: f64) {
        let at = self.local(t);
        if self.keys.of(prop).is_empty() {
            *self.base_mut(prop) = v;
        } else {
            self.keys.of_mut(prop).set(at, v, tol);
        }
    }

    /// Стоит ли в момент `t` точка свойства.
    pub fn key_at(&self, prop: Prop, t: f64, tol: f64) -> bool {
        self.keys.of(prop).find(self.local(t), tol).is_some()
    }

    /// Закрепляет значение свойства в момент `t` или убирает точку, которая там стоит.
    pub fn toggle_key(&mut self, prop: Prop, t: f64, tol: f64) {
        let (at, v) = (self.local(t), self.value(prop, t));
        match self.keys.of(prop).find(at, tol) {
            Some(i) => {
                self.keys.of_mut(prop).0.remove(i);
                // Без точек свойство остаётся таким, каким его видели в этот момент.
                *self.base_mut(prop) = v;
            }
            None => self.keys.of_mut(prop).set(at, v, tol),
        }
    }

    /// Плавное появление (`at_start`) или исчезновение за `len` секунд: на краю клипа свойство
    /// закрепляется нулём, а через `len` секунд от края — тем значением, которое там было.
    pub fn fade(&mut self, prop: Prop, at_start: bool, len: f64, tol: f64) {
        let len = len.min(self.duration / 2.0);
        let (edge, inner) = if at_start { (0.0, len) } else { (self.duration, self.duration - len) };
        let level = self.keys.of(prop).at(inner, self.keys.is_eased(prop)).unwrap_or(self.base(prop));
        let curve = self.keys.of_mut(prop);
        curve.set(inner, level, tol);
        curve.set(edge, 0.0, tol);
    }

    /// Убирает все точки свойства; значение остаётся таким, каким оно было в момент `t`.
    pub fn clear_keys(&mut self, prop: Prop, t: f64) {
        *self.base_mut(prop) = self.value(prop, t);
        self.keys.of_mut(prop).0.clear();
    }

    /// Ход свойства за время клипа: моменты таймлайна и значения в них, между ними оно меняется
    /// равномерно. У свойства без точек момент один — начало клипа. Плавный ход разложен
    /// на короткие прямые отрезки.
    pub fn ramp(&self, prop: Prop) -> Vec<(f64, f32)> {
        let keys = &self.keys.of(prop).0;
        let mut knots = vec![self.start];
        knots.extend(keys.iter().filter(|k| k.t > 0.0 && k.t < self.duration).map(|k| self.start + k.t));
        if !keys.is_empty() {
            knots.push(self.end());
        }
        let steps = if self.keys.is_eased(prop) { EASE_STEPS } else { 1 };
        let mut out = vec![(self.start, self.value(prop, self.start))];
        for w in knots.windows(2) {
            for i in 1..=steps {
                let t = if i == steps { w[1] } else { w[0] + (w[1] - w[0]) * i as f64 / steps as f64 };
                out.push((t, self.value(prop, t)));
            }
        }
        out
    }

    /// Наибольшее значение свойства за время клипа.
    pub fn peak(&self, prop: Prop) -> f32 {
        self.ramp(prop).into_iter().map(|p| p.1).fold(f32::MIN, f32::max)
    }

    /// Сколько секунд исходного файла занимает клип.
    pub fn source_duration(&self) -> f64 {
        self.duration * self.speed
    }

    pub fn contains(&self, t: f64) -> bool {
        t >= self.start - EPS && t < self.end() - EPS
    }

    pub fn reset_transform(&mut self) {
        self.scale = 1.0;
        self.pos_x = 0.0;
        self.pos_y = 0.0;
        self.rotation = 0.0;
        self.opacity = 1.0;
        self.flip_h = false;
        self.flip_v = false;
        for prop in Prop::TRANSFORM {
            self.keys.of_mut(prop).0.clear();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub kind: TrackKind,
    pub name: String,
    pub clips: Vec<Clip>,
    pub locked: bool,
    /// Видимость видеодорожки или звук аудиодорожки.
    pub enabled: bool,
    /// Дорожка субтитров: она одна, лежит над остальными и не принимает наложения.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub captions: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub media: Vec<MediaItem>,
    /// Дорожки в порядке отображения сверху вниз; нижняя видеодорожка рисуется первой.
    pub tracks: Vec<Track>,
    pub next_id: Id,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Self {
        let mut p = Project {
            name: "Новый проект".to_owned(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            media: Vec::new(),
            tracks: Vec::new(),
            next_id: 1,
        };
        for (kind, name) in [
            (TrackKind::Video, "Видео 2"),
            (TrackKind::Video, "Видео 1"),
            (TrackKind::Audio, "Аудио 1"),
            (TrackKind::Audio, "Аудио 2"),
        ] {
            let id = p.alloc_id();
            p.tracks.push(Track {
                id,
                kind,
                name: name.to_owned(),
                clips: Vec::new(),
                locked: false,
                enabled: true,
                captions: false,
            });
        }
        p
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: Id) -> Option<&MediaItem> {
        self.media.iter().find(|m| m.id == id)
    }

    /// С какой точностью курсор времени считается стоящим на закреплённой точке: полкадра.
    pub fn key_reach(&self) -> f64 {
        0.5 / self.fps.max(1.0) + 1e-6
    }

    pub fn media_by_path(&self, path: &Path) -> Option<&MediaItem> {
        self.media.iter().find(|m| m.path == path)
    }

    /// Длительность проекта: конец самого позднего клипа.
    pub fn duration(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end())
            .fold(0.0, f64::max)
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.clips.is_empty())
    }

    pub fn find_clip(&self, id: Id) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            if let Some(ci) = t.clips.iter().position(|c| c.id == id) {
                return Some((ti, ci));
            }
        }
        None
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        let (ti, ci) = self.find_clip(id)?;
        Some(&self.tracks[ti].clips[ci])
    }

    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        let (ti, ci) = self.find_clip(id)?;
        Some(&mut self.tracks[ti].clips[ci])
    }

    fn sort_track(&mut self, ti: usize) {
        self.tracks[ti]
            .clips
            .sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Подходит ли медиафайл для дорожки.
    pub fn compatible(&self, media: Id, track: usize) -> bool {
        match (self.media(media), self.tracks.get(track)) {
            (Some(m), Some(t)) => match t.kind {
                TrackKind::Video => m.kind != MediaKind::Audio && !t.captions,
                TrackKind::Audio => m.kind == MediaKind::Audio,
            },
            _ => false,
        }
    }

    /// Подходит ли клип для дорожки; эффектам и надписям нужна видеодорожка, а на дорожке
    /// субтитров лежат только субтитры.
    pub fn clip_fits(&self, clip: &Clip, track: usize) -> bool {
        if clip.generated() {
            let caption = clip.text.as_ref().map_or(false, |t| t.caption);
            return self.tracks.get(track).map_or(false, |t| t.kind == TrackKind::Video && (caption || !t.captions));
        }
        self.compatible(clip.media, track)
    }

    /// Верхняя незаблокированная видеодорожка: на неё ложатся наложения.
    pub fn overlay_track(&self) -> Option<usize> {
        self.tracks.iter().position(|t| t.kind == TrackKind::Video && !t.locked && !t.captions)
    }

    /// Ближайшее к `desired` начало, при котором клип длиной `dur` ни с кем не пересекается.
    pub fn free_start(&self, track: usize, ignore: Option<Id>, desired: f64, dur: f64) -> f64 {
        let desired = desired.max(0.0);
        let others: Vec<(f64, f64)> = self.tracks[track]
            .clips
            .iter()
            .filter(|c| Some(c.id) != ignore)
            .map(|c| (c.start, c.end()))
            .collect();
        let overlaps = |s: f64| others.iter().any(|&(a, b)| s < b - EPS && s + dur > a + EPS);
        if !overlaps(desired) {
            return desired;
        }
        let mut best: Option<f64> = None;
        for &(a, b) in &others {
            for cand in [b, a - dur] {
                if cand < -EPS {
                    continue;
                }
                let cand = cand.max(0.0);
                if overlaps(cand) {
                    continue;
                }
                if best.map_or(true, |x| (cand - desired).abs() < (x - desired).abs()) {
                    best = Some(cand);
                }
            }
        }
        best.unwrap_or_else(|| others.iter().map(|x| x.1).fold(0.0, f64::max))
    }

    /// Конец последнего клипа на дорожке.
    pub fn track_end(&self, track: usize) -> f64 {
        self.tracks[track].clips.iter().map(|c| c.end()).fold(0.0, f64::max)
    }

    /// Длительность клипа по умолчанию для медиафайла.
    pub fn default_duration(&self, media: Id) -> f64 {
        match self.media(media) {
            Some(m) if m.kind == MediaKind::Image => 5.0,
            Some(m) => m.duration.max(MIN_CLIP),
            None => 5.0,
        }
    }

    /// Добавляет клип на дорожку как можно ближе к `start`.
    pub fn add_clip(&mut self, track: usize, media: Id, start: f64) -> Option<Id> {
        if !self.compatible(media, track) || self.tracks[track].locked {
            return None;
        }
        let duration = self.default_duration(media);
        let start = self.free_start(track, None, start, duration);
        let id = self.alloc_id();
        self.tracks[track].clips.push(Clip::new(id, media, start, duration));
        self.sort_track(track);
        Some(id)
    }

    /// Добавляет на видеодорожку клип без исходного файла как можно ближе к `start`.
    fn add_generated(&mut self, track: usize, start: f64, duration: f64, fill: impl FnOnce(&mut Clip)) -> Option<Id> {
        let t = self.tracks.get(track)?;
        if t.kind != TrackKind::Video || t.locked {
            return None;
        }
        let start = self.free_start(track, None, start, duration);
        let id = self.alloc_id();
        let mut clip = Clip::new(id, 0, start, duration);
        fill(&mut clip);
        self.tracks[track].clips.push(clip);
        self.sort_track(track);
        Some(id)
    }

    /// Добавляет на видеодорожку клип с процедурным эффектом как можно ближе к `start`.
    pub fn add_effect(&mut self, track: usize, kind: EffectKind, start: f64) -> Option<Id> {
        // Квадратный кадр эффекта, который идёт по всему кадру проекта, растянут до его длинной стороны.
        let (long, short) = (self.width.max(self.height) as f32, self.width.min(self.height).max(1) as f32);
        // Зерно берётся из номера клипа: каждый новый эффект выглядит по-своему.
        self.add_generated(track, start, kind.duration(), |clip| {
            clip.effect = Some(Effect::new(kind, clip.id as u32));
            if kind.fills_frame() {
                clip.scale = long / short;
            }
        })
    }

    /// Добавляет на видеодорожку надпись как можно ближе к `start`.
    pub fn add_text(&mut self, track: usize, text: Text, start: f64, duration: f64) -> Option<Id> {
        self.add_generated(track, start, duration.max(MIN_CLIP), |clip| clip.text = Some(text))
    }

    // ---------- Субтитры ----------

    /// Дорожка субтитров; если её ещё нет, она появляется над остальными.
    pub fn captions_track(&mut self) -> usize {
        if let Some(i) = self.tracks.iter().position(|t| t.captions) {
            return i;
        }
        let id = self.alloc_id();
        let track = Track {
            id,
            kind: TrackKind::Video,
            name: "Субтитры".to_owned(),
            clips: Vec::new(),
            locked: false,
            enabled: true,
            captions: true,
        };
        self.tracks.insert(0, track);
        0
    }

    /// Ставит субтитр на дорожку субтитров как можно ближе к `start`. Новый субтитр выглядит
    /// и стоит в кадре так же, как уже имеющиеся.
    pub fn add_caption(&mut self, content: &str, start: f64, duration: f64) -> Option<Id> {
        let track = self.captions_track();
        let mut text = Text::caption(content, self.height);
        let mut place = (0.0, (self.height as f32 * 0.36).round(), 1.0);
        if let Some((clip, style)) = self.captions().first().and_then(|c| Some((*c, c.text.as_ref()?))) {
            text.restyle(style);
            place = (clip.pos_x, clip.pos_y, clip.scale);
        }
        self.add_generated(track, start, duration.max(MIN_CLIP), |clip| {
            clip.text = Some(text);
            (clip.pos_x, clip.pos_y, clip.scale) = place;
        })
    }

    /// Субтитры проекта по порядку.
    pub fn captions(&self) -> Vec<&Clip> {
        let mut out: Vec<&Clip> = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.text.as_ref().map_or(false, |t| t.caption))
            .collect();
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        out
    }

    /// Переносит оформление и место в кадре одного субтитра на все остальные.
    pub fn restyle_captions(&mut self, from: Id) {
        let Some(sample) = self.clip(from).cloned() else { return };
        let Some(style) = &sample.text else { return };
        for clip in self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            if let Some(text) = clip.text.as_mut().filter(|t| t.caption) {
                text.restyle(style);
                (clip.pos_x, clip.pos_y, clip.scale) = (sample.pos_x, sample.pos_y, sample.scale);
            }
        }
    }

    /// Добавляет клип в конец первой подходящей дорожки (основной).
    pub fn append_clip(&mut self, media: Id) -> Option<Id> {
        let kind = match self.media(media)?.kind {
            MediaKind::Audio => TrackKind::Audio,
            _ => TrackKind::Video,
        };
        // Основная видеодорожка — нижняя, основная аудиодорожка — верхняя.
        let track = match kind {
            TrackKind::Video => self.tracks.iter().rposition(|t| t.kind == kind && !t.locked)?,
            TrackKind::Audio => self.tracks.iter().position(|t| t.kind == kind && !t.locked)?,
        };
        let start = self.track_end(track);
        self.add_clip(track, media, start)
    }

    pub fn remove_clip(&mut self, id: Id) -> bool {
        match self.find_clip(id) {
            Some((ti, ci)) => {
                self.tracks[ti].clips.remove(ci);
                true
            }
            None => false,
        }
    }

    /// Удаляет клип и сдвигает следующие за ним клипы дорожки влево, закрывая промежуток.
    pub fn ripple_delete(&mut self, id: Id) -> bool {
        let Some((ti, ci)) = self.find_clip(id) else { return false };
        let gone = self.tracks[ti].clips.remove(ci);
        for clip in self.tracks[ti].clips.iter_mut().filter(|c| c.start >= gone.start) {
            clip.start -= gone.duration;
        }
        true
    }

    /// Вставляет копию клипа на дорожку `track` как можно ближе к `start`.
    pub fn paste(&mut self, clip: &Clip, track: usize, start: f64) -> Option<Id> {
        if track >= self.tracks.len() || !self.clip_fits(clip, track) || self.tracks[track].locked {
            return None;
        }
        let id = self.alloc_id();
        // Переход привязан к стыку, на новом месте его нет.
        let mut copy = Clip { id, transition: None, ..clip.clone() };
        copy.start = self.free_start(track, None, start, copy.duration);
        self.tracks[track].clips.push(copy);
        self.sort_track(track);
        Some(id)
    }

    /// Копия клипа на той же дорожке сразу после него (или в ближайшем свободном месте).
    pub fn duplicate(&mut self, id: Id) -> Option<Id> {
        let (ti, ci) = self.find_clip(id)?;
        let clip = self.tracks[ti].clips[ci].clone();
        self.paste(&clip, ti, clip.end())
    }

    /// Разрезает клип в момент `t`; возвращает id правой части.
    pub fn split(&mut self, id: Id, t: f64) -> Option<Id> {
        let (ti, ci) = self.find_clip(id)?;
        let c = self.tracks[ti].clips[ci].clone();
        if t <= c.start + MIN_CLIP || t >= c.end() - MIN_CLIP {
            return None;
        }
        let new_id = self.alloc_id();
        let left = t - c.start;
        let mut right = c.clone();
        right.id = new_id;
        right.start = t;
        right.offset = c.offset + left * c.speed;
        right.duration = c.duration - left;
        // Переход остаётся у левой части, движение и эффект продолжаются в правой.
        right.transition = None;
        // Закреплённые точки остаются на своих местах таймлайна у обеих частей.
        right.keys.shift(-left);
        let part = (left / c.duration) as f32;
        let first = &mut self.tracks[ti].clips[ci];
        first.duration = left;
        if let (Some(a), Some(b)) = (&mut first.motion, &mut right.motion) {
            a.split(b, part);
        }
        if let (Some(a), Some(b)) = (&mut first.effect, &mut right.effect) {
            a.split(b, part);
        }
        self.tracks[ti].clips.insert(ci + 1, right);
        Some(new_id)
    }

    /// У картинок, эффектов и надписей нет длительности исходника: их можно растягивать как угодно.
    fn is_still(&self, clip: &Clip) -> bool {
        clip.generated() || self.media(clip.media).map_or(false, |m| m.kind == MediaKind::Image)
    }

    /// Двигает левую границу клипа (обрезка начала).
    pub fn trim_start(&mut self, id: Id, new_start: f64) {
        let Some((ti, ci)) = self.find_clip(id) else { return };
        let c = self.tracks[ti].clips[ci].clone();
        let prev_end = self.tracks[ti]
            .clips
            .iter()
            .filter(|o| o.id != id && o.end() <= c.start + EPS)
            .map(|o| o.end())
            .fold(0.0, f64::max);
        let still = self.is_still(&c);
        let source_limit = if still {
            0.0
        } else {
            c.start - c.offset / c.speed
        };
        let lo = prev_end.max(source_limit).max(0.0);
        let hi = c.end() - MIN_CLIP;
        let ns = new_start.clamp(lo.min(hi), hi);
        let delta = ns - c.start;
        let clip = &mut self.tracks[ti].clips[ci];
        clip.start = ns;
        clip.duration = c.duration - delta;
        clip.keys.shift(-delta);
        if !still {
            clip.offset = (c.offset + delta * c.speed).max(0.0);
        }
    }

    /// Двигает правую границу клипа (обрезка конца).
    pub fn trim_end(&mut self, id: Id, new_end: f64) {
        let Some((ti, ci)) = self.find_clip(id) else { return };
        let c = self.tracks[ti].clips[ci].clone();
        let next_start = self.tracks[ti]
            .clips
            .iter()
            .filter(|o| o.id != id && o.start >= c.end() - EPS)
            .map(|o| o.start)
            .fold(f64::INFINITY, f64::min);
        let source_limit = match self.media(c.media) {
            Some(m) if m.kind != MediaKind::Image => c.start + (m.duration - c.offset) / c.speed,
            _ => f64::INFINITY,
        };
        let lo = c.start + MIN_CLIP;
        let hi = next_start.min(source_limit).max(lo);
        self.tracks[ti].clips[ci].duration = new_end.clamp(lo, hi) - c.start;
    }

    /// Переносит клип на дорожку `track` с началом как можно ближе к `start`.
    pub fn move_clip(&mut self, id: Id, track: usize, start: f64) {
        let Some((ti, ci)) = self.find_clip(id) else { return };
        let target = if self.clip_fits(&self.tracks[ti].clips[ci], track) && !self.tracks[track].locked {
            track
        } else {
            ti
        };
        let mut clip = self.tracks[ti].clips.remove(ci);
        clip.start = self.free_start(target, None, start, clip.duration);
        self.tracks[target].clips.push(clip);
        self.sort_track(target);
    }

    /// Меняет скорость, сохраняя использованный отрезок исходника.
    pub fn set_speed(&mut self, id: Id, speed: f64) {
        let Some((ti, ci)) = self.find_clip(id) else { return };
        let c = self.tracks[ti].clips[ci].clone();
        if self.is_still(&c) {
            return;
        }
        let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        let source = c.source_duration();
        let next_start = self.tracks[ti]
            .clips
            .iter()
            .filter(|o| o.id != id && o.start >= c.end() - EPS)
            .map(|o| o.start)
            .fold(f64::INFINITY, f64::min);
        let duration = (source / speed).min(next_start - c.start).max(MIN_CLIP);
        let clip = &mut self.tracks[ti].clips[ci];
        clip.keys.stretch(c.speed / speed);
        clip.speed = speed;
        clip.duration = duration;
    }

    // ---------- Переходы ----------

    /// Клип той же дорожки, который заканчивается ровно там, где начинается `clip`.
    pub fn prev_adjacent(&self, track: usize, clip: &Clip) -> Option<&Clip> {
        self.tracks[track]
            .clips
            .iter()
            .find(|o| o.id != clip.id && (o.end() - clip.start).abs() < ADJACENT)
    }

    /// Клип той же дорожки, который начинается ровно там, где заканчивается `clip`.
    pub fn next_adjacent(&self, track: usize, clip: &Clip) -> Option<&Clip> {
        self.tracks[track]
            .clips
            .iter()
            .find(|o| o.id != clip.id && (o.start - clip.end()).abs() < ADJACENT)
    }

    /// Наибольшая длительность перехода на стыке перед клипом: не длиннее каждого из соседей.
    pub fn transition_limit(&self, id: Id) -> Option<f64> {
        let (ti, ci) = self.find_clip(id)?;
        let clip = &self.tracks[ti].clips[ci];
        let prev = self.prev_adjacent(ti, clip)?;
        Some(prev.duration.min(clip.duration))
    }

    /// Действующий переход в начале клипа: его вид и половина длительности.
    pub fn transition_in(&self, track: usize, clip: &Clip) -> Option<(TransitionKind, f64)> {
        let tr = clip.transition.as_ref()?;
        let prev = self.prev_adjacent(track, clip)?;
        let half = tr.duration.min(prev.duration).min(clip.duration) / 2.0;
        (half > 0.0).then_some((tr.kind, half))
    }

    /// Переход в конце клипа — тот, что стоит в начале следующего.
    pub fn transition_out(&self, track: usize, clip: &Clip) -> Option<(TransitionKind, f64)> {
        self.transition_in(track, self.next_adjacent(track, clip)?)
    }

    /// На сколько секунд клип виден до своего начала и после конца: пока идёт переход,
    /// в котором оба клипа показаны одновременно.
    pub fn overhang(&self, track: usize, clip: &Clip) -> (f64, f64) {
        let side = |tr: Option<(TransitionKind, f64)>| match tr {
            Some((kind, half)) if kind.overlaps() => half,
            _ => 0.0,
        };
        (side(self.transition_in(track, clip)), side(self.transition_out(track, clip)))
    }

    /// Что переходы делают с клипом в момент `t`.
    pub fn tweak(&self, track: usize, clip: &Clip, t: f64) -> Tweak {
        let (w, h) = (self.width as f32, self.height as f32);
        let mut tweak = Tweak::default();
        if let Some((kind, half)) = self.transition_in(track, clip) {
            let p = (t - (clip.start - half)) / (2.0 * half);
            tweak = tweak.and(kind.tweak(true, p as f32, w, h));
        }
        if let Some((kind, half)) = self.transition_out(track, clip) {
            let p = (t - (clip.end() - half)) / (2.0 * half);
            tweak = tweak.and(kind.tweak(false, p as f32, w, h));
        }
        tweak
    }

    /// Ставит переход на стык перед клипом; `false`, если слева вплотную никого нет.
    pub fn set_transition(&mut self, id: Id, kind: TransitionKind) -> bool {
        let Some((ti, ci)) = self.find_clip(id) else { return false };
        let clip = &self.tracks[ti].clips[ci];
        if self.tracks[ti].kind != TrackKind::Video || self.prev_adjacent(ti, clip).is_none() {
            return false;
        }
        let clip = &mut self.tracks[ti].clips[ci];
        match &mut clip.transition {
            Some(tr) => tr.kind = kind,
            None => clip.transition = Some(Transition::new(kind)),
        }
        true
    }

    /// Убирает переходы, оставшиеся без соседа слева (клип отодвинули или соседа удалили).
    pub fn prune_transitions(&mut self) {
        for ti in 0..self.tracks.len() {
            let orphans: Vec<Id> = self.tracks[ti]
                .clips
                .iter()
                .filter(|c| c.transition.is_some() && self.prev_adjacent(ti, c).is_none())
                .map(|c| c.id)
                .collect();
            for clip in &mut self.tracks[ti].clips {
                if orphans.contains(&clip.id) {
                    clip.transition = None;
                }
            }
        }
    }

    /// Стыки видеодорожки, на которые можно поставить переход: клип справа от стыка и момент стыка.
    pub fn junctions(&self, track: usize) -> Vec<(Id, f64)> {
        let t = &self.tracks[track];
        if t.kind != TrackKind::Video {
            return Vec::new();
        }
        t.clips
            .iter()
            .filter(|c| self.prev_adjacent(track, c).is_some())
            .map(|c| (c.id, c.start))
            .collect()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Project, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        serde_json::from_slice(&data).map_err(|e| format!("файл проекта повреждён: {e}"))
    }
}

/// История правок: снимки дорожек.
#[derive(Default)]
pub struct History {
    undo: Vec<Vec<Track>>,
    redo: Vec<Vec<Track>>,
}

impl History {
    pub fn push(&mut self, tracks: &[Track]) {
        self.undo.push(tracks.to_vec());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut project.tracks, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut project.tracks, next));
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Размер, вписанный в рамку с сохранением пропорций.
pub fn fit_size(w: f32, h: f32, box_w: f32, box_h: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (box_w, box_h);
    }
    let k = (box_w / w).min(box_h / h);
    (w * k, h * k)
}

/// Тайм-код ЧЧ:ММ:СС:КК.
pub fn timecode(t: f64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as u64;
    let frames = (t.max(0.0) * fps + 1e-4).floor() as u64;
    let f = frames % fps_i;
    let s = frames / fps_i;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

/// Короткая запись длительности ММ:СС.
pub fn short_time(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_media() -> (Project, Id, Id) {
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
            has_audio: true,
        });
        let a = p.alloc_id();
        p.media.push(MediaItem {
            id: a,
            path: PathBuf::from("a.mp3"),
            name: "a.mp3".into(),
            kind: MediaKind::Audio,
            duration: 20.0,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: true,
        });
        (p, v, a)
    }

    #[test]
    fn append_goes_to_main_tracks_without_overlap() {
        let (mut p, v, a) = project_with_media();
        let c1 = p.append_clip(v).unwrap();
        let c2 = p.append_clip(v).unwrap();
        let c3 = p.append_clip(a).unwrap();
        assert_eq!(p.find_clip(c1).unwrap().0, 1);
        assert_eq!(p.clip(c2).unwrap().start, 10.0);
        assert_eq!(p.find_clip(c3).unwrap().0, 2);
        assert_eq!(p.duration(), 20.0);
    }

    #[test]
    fn audio_does_not_go_to_video_track() {
        let (mut p, _, a) = project_with_media();
        assert!(p.add_clip(0, a, 0.0).is_none());
    }

    #[test]
    fn free_start_snaps_to_nearest_gap() {
        let (mut p, v, _) = project_with_media();
        p.add_clip(1, v, 0.0).unwrap();
        // Пытаемся поставить поверх: ближайшее свободное место — сразу после клипа.
        let id = p.add_clip(1, v, 6.0).unwrap();
        assert_eq!(p.clip(id).unwrap().start, 10.0);
    }

    #[test]
    fn split_keeps_source_continuity() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        p.set_speed(id, 2.0);
        assert_eq!(p.clip(id).unwrap().duration, 5.0);
        let right = p.split(id, 2.0).unwrap();
        let l = p.clip(id).unwrap().clone();
        let r = p.clip(right).unwrap().clone();
        assert_eq!(l.duration, 2.0);
        assert_eq!(r.start, 2.0);
        assert_eq!(r.offset, 4.0);
        assert_eq!(r.duration, 3.0);
    }

    #[test]
    fn trim_respects_source_and_neighbours() {
        let (mut p, v, _) = project_with_media();
        let a = p.append_clip(v).unwrap();
        let b = p.append_clip(v).unwrap();
        // Конец первого клипа нельзя увести за начало второго.
        p.trim_end(a, 15.0);
        assert_eq!(p.clip(a).unwrap().end(), 10.0);
        p.trim_end(a, 4.0);
        assert_eq!(p.clip(a).unwrap().duration, 4.0);
        // Начало второго можно сдвинуть вправо, но не левее начала исходника.
        p.trim_start(b, 12.0);
        let c = p.clip(b).unwrap().clone();
        assert_eq!((c.start, c.offset, c.duration), (12.0, 2.0, 8.0));
        p.trim_start(b, 0.0);
        let c = p.clip(b).unwrap().clone();
        assert_eq!((c.start, c.offset), (10.0, 0.0));
    }

    #[test]
    fn transition_lives_on_a_junction() {
        let (mut p, v, _) = project_with_media();
        let a = p.append_clip(v).unwrap();
        let b = p.split(a, 4.0).unwrap();
        // Переход нельзя поставить перед клипом, у которого нет соседа слева.
        assert!(!p.set_transition(a, TransitionKind::Dissolve));
        assert!(p.set_transition(b, TransitionKind::Dissolve));
        assert_eq!(p.junctions(1), vec![(b, 4.0)]);

        let (left, right) = (p.clip(a).unwrap().clone(), p.clip(b).unwrap().clone());
        // Растворение показывает оба клипа сразу: каждый виден на половину перехода дольше.
        assert_eq!(p.overhang(1, &left), (0.0, 0.3));
        assert_eq!(p.overhang(1, &right), (0.3, 0.0));
        assert_eq!(p.tweak(1, &right, 4.0).alpha, 0.5);
        assert_eq!(p.tweak(1, &left, 4.0).alpha, 1.0);

        // Затухание идёт по очереди: клипы за свои границы не выходят.
        p.set_transition(b, TransitionKind::Fade);
        assert_eq!(p.overhang(1, &left), (0.0, 0.0));
        assert_eq!(p.tweak(1, &left, 4.0).alpha, 0.0);
        assert_eq!(p.tweak(1, &right, 4.3).alpha, 1.0);

        // Клип отодвинули — стыка нет, переход пропадает.
        p.move_clip(b, 1, 6.0);
        p.prune_transitions();
        assert!(p.clip(b).unwrap().transition.is_none());
    }

    #[test]
    fn transition_is_not_longer_than_its_clips() {
        let (mut p, v, _) = project_with_media();
        let a = p.append_clip(v).unwrap();
        let b = p.split(a, 9.8).unwrap();
        p.set_transition(b, TransitionKind::SlideLeft);
        let right = p.clip(b).unwrap().clone();
        let (_, half) = p.transition_in(1, &right).unwrap();
        assert!((half - 0.1).abs() < 1e-9, "половина перехода {half}");
    }

    #[test]
    fn split_keeps_transition_on_the_left_part() {
        let (mut p, v, _) = project_with_media();
        let a = p.append_clip(v).unwrap();
        let b = p.split(a, 4.0).unwrap();
        p.set_transition(b, TransitionKind::Zoom);
        let c = p.split(b, 7.0).unwrap();
        assert!(p.clip(b).unwrap().transition.is_some());
        assert!(p.clip(c).unwrap().transition.is_none());
    }

    #[test]
    fn effect_clip_stretches_freely_and_splits_in_two_halves() {
        let (mut p, v, a) = project_with_media();
        assert!(p.add_effect(2, EffectKind::Explosion, 0.0).is_none(), "эффекту нужна видеодорожка");
        let id = p.add_effect(0, EffectKind::Explosion, 2.0).unwrap();
        assert_eq!(p.clip(id).unwrap().duration, EffectKind::Explosion.duration());
        assert!(p.clip_fits(p.clip(id).unwrap(), 1));
        assert!(!p.clip_fits(p.clip(id).unwrap(), 2));
        let _ = (v, a);

        // У эффекта нет исходника: обе границы двигаются свободно.
        p.trim_start(id, 1.0);
        p.trim_end(id, 5.0);
        let c = p.clip(id).unwrap().clone();
        assert_eq!((c.start, c.end()), (1.0, 5.0));

        let right = p.split(id, 2.0).unwrap();
        assert_eq!(p.clip(id).unwrap().effect.as_ref().unwrap().span, [0.0, 0.25]);
        assert_eq!(p.clip(right).unwrap().effect.as_ref().unwrap().span, [0.25, 1.0]);
    }

    #[test]
    fn motion_path_moves_clip_and_survives_save() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        let clip = p.clip_mut(id).unwrap();
        clip.pos_x = 10.0;
        clip.motion = Some(Motion { points: vec![[0.0, 0.0], [200.0, 100.0]], smooth: false, ..Motion::default() });
        let clip = p.clip(id).unwrap();
        assert_eq!(clip.position(0.0), (10.0, 0.0));
        assert_eq!(clip.position(5.0), (110.0, 50.0));
        assert_eq!(clip.position(99.0), (210.0, 100.0));

        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clip(id).unwrap().motion, p.clip(id).unwrap().motion);
    }

    #[test]
    fn old_project_files_still_open() {
        // Клип, сохранённый до появления переходов, пути, эффектов и закреплённых точек.
        let old = r#"{"id":5,"media":1,"start":0.0,"offset":0.0,"duration":3.0,"speed":1.0,"volume":1.0,
            "scale":1.0,"pos_x":0.0,"pos_y":0.0,"rotation":0.0,"opacity":1.0}"#;
        let clip: Clip = serde_json::from_str(old).unwrap();
        assert!(clip.transition.is_none() && clip.motion.is_none() && clip.effect.is_none());
        assert!(clip.keys.is_empty());
    }

    /// Допуск, с которым точка считается стоящей под курсором: полкадра при 30 к/с.
    const TOL: f64 = 0.5 / 30.0;

    #[test]
    fn pinned_values_change_between_points_and_survive_save() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        p.move_clip(id, 1, 2.0);
        let clip = p.clip_mut(id).unwrap();
        clip.opacity = 0.2;
        // Без точек правка меняет обычное значение.
        clip.set_value(Prop::Opacity, 5.0, 0.4, TOL);
        assert_eq!((clip.opacity, clip.keys.is_empty()), (0.4, true));

        // Закрепили в одном моменте, передвинули курсор и поменяли значение — точек стало две.
        clip.toggle_key(Prop::Opacity, 4.0, TOL);
        clip.set_value(Prop::Opacity, 8.0, 1.0, TOL);
        assert_eq!(clip.keys.opacity.0.len(), 2);
        assert_eq!(clip.value(Prop::Opacity, 0.0), 0.4);
        assert_eq!(clip.value(Prop::Opacity, 4.0), 0.4);
        assert!((clip.value(Prop::Opacity, 6.0) - 0.7).abs() < 1e-6);
        assert_eq!(clip.value(Prop::Opacity, 11.0), 1.0);
        // Правка в том же моменте меняет точку, а не плодит новую.
        clip.set_value(Prop::Opacity, 8.01, 0.9, TOL);
        assert_eq!(clip.keys.opacity.0.len(), 2);
        assert!(clip.key_at(Prop::Opacity, 8.0, TOL) && !clip.key_at(Prop::Opacity, 6.0, TOL));
        // Остальные свойства остались обычными.
        assert_eq!(clip.value(Prop::Scale, 6.0), 1.0);

        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clip(id).unwrap().keys, p.clip(id).unwrap().keys);

        // Убрали точки по одной: свойство остаётся таким, каким его видели.
        let clip = p.clip_mut(id).unwrap();
        clip.toggle_key(Prop::Opacity, 8.0, TOL);
        clip.toggle_key(Prop::Opacity, 4.0, TOL);
        assert_eq!((clip.opacity, clip.keys.is_empty()), (0.4, true));
    }

    #[test]
    fn pinned_position_adds_up_with_the_path() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        let clip = p.clip_mut(id).unwrap();
        clip.toggle_key(Prop::PosX, 0.0, TOL);
        clip.set_value(Prop::PosX, 10.0, 100.0, TOL);
        clip.motion = Some(Motion { points: vec![[0.0, 0.0], [0.0, 50.0]], smooth: false, ..Motion::default() });
        assert_eq!(clip.position(5.0), (50.0, 25.0));
    }

    #[test]
    fn pinned_points_stay_in_place_when_clip_is_cut_trimmed_or_retimed() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        let clip = p.clip_mut(id).unwrap();
        clip.toggle_key(Prop::Scale, 2.0, TOL);
        clip.set_value(Prop::Scale, 8.0, 4.0, TOL);
        let before: Vec<f32> = (0..=10).map(|t| clip.value(Prop::Scale, t as f64)).collect();

        // Разрез: обе части вместе идут так же, как шёл целый клип.
        let right = p.split(id, 5.0).unwrap();
        for (t, want) in before.iter().enumerate() {
            let part = if t < 5 { id } else { right };
            assert_eq!(p.clip(part).unwrap().value(Prop::Scale, t as f64), *want, "момент {t}");
        }
        assert_eq!(p.clip(right).unwrap().ramp(Prop::Scale), vec![(5.0, 2.5), (8.0, 4.0), (10.0, 4.0)]);

        // Обрезка начала не сдвигает точки по таймлайну.
        p.trim_start(right, 7.0);
        assert_eq!(p.clip(right).unwrap().value(Prop::Scale, 8.0), 4.0);
        assert_eq!(p.clip(right).unwrap().value(Prop::Scale, 7.0), 3.5);

        // Вдвое быстрее — точки вдвое ближе к началу клипа.
        p.set_speed(id, 2.0);
        let left = p.clip(id).unwrap();
        assert_eq!((left.duration, left.keys.scale.0[0].t), (2.5, 1.0));

        // Сброс трансформации убирает и точки.
        p.clip_mut(right).unwrap().reset_transform();
        assert!(p.clip(right).unwrap().keys.is_empty());
    }

    #[test]
    fn fades_pin_the_value_to_zero_at_the_clip_edges() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        p.move_clip(id, 1, 2.0);
        let clip = p.clip_mut(id).unwrap();
        clip.opacity = 0.8;
        clip.fade(Prop::Opacity, true, 0.5, TOL);
        clip.fade(Prop::Opacity, false, 0.5, TOL);
        // Клип идёт со 2-й по 12-ю секунду: проявляется за полсекунды и за полсекунды гаснет.
        assert_eq!(clip.ramp(Prop::Opacity), vec![(2.0, 0.0), (2.5, 0.8), (11.5, 0.8), (12.0, 0.0)]);
        assert!((clip.value(Prop::Opacity, 2.25) - 0.4).abs() < 1e-6);
        // Повторное нажатие ничего не добавляет.
        clip.fade(Prop::Opacity, true, 0.5, TOL);
        assert_eq!(clip.keys.opacity.0.len(), 4);
        // У короткого клипа появление и исчезновение делят его пополам.
        let short = p.split(id, 2.6).unwrap();
        let _ = short;
        let clip = p.clip_mut(id).unwrap();
        clip.keys.opacity.0.clear();
        clip.fade(Prop::Volume, true, 0.5, TOL);
        assert_eq!(clip.keys.volume.0.iter().map(|k| k.t).collect::<Vec<f64>>(), vec![0.0, clip.duration / 2.0]);
    }

    #[test]
    fn eased_values_start_and_stop_gently() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        let clip = p.clip_mut(id).unwrap();
        clip.toggle_key(Prop::PosX, 2.0, TOL);
        clip.set_value(Prop::PosX, 6.0, 400.0, TOL);
        let straight = clip.ramp(Prop::PosX);
        assert_eq!(straight, vec![(0.0, 0.0), (2.0, 0.0), (6.0, 400.0), (10.0, 400.0)]);

        clip.keys.set_eased(Prop::PosX, true);
        assert_eq!(clip.value(Prop::PosX, 4.0), 200.0);
        assert!(clip.value(Prop::PosX, 2.5) < 25.0 && clip.value(Prop::PosX, 5.5) > 375.0);
        // Для экспорта плавный ход раскладывается на короткие прямые отрезки с теми же концами.
        let eased = clip.ramp(Prop::PosX);
        assert_eq!(eased.len(), 1 + 3 * EASE_STEPS);
        assert!(eased.contains(&(2.0, 0.0)) && eased.contains(&(6.0, 400.0)) && eased.contains(&(4.0, 200.0)));
        assert!(eased.windows(2).all(|w| w[1].0 > w[0].0 && w[1].1 >= w[0].1));

        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert!(back.clip(id).unwrap().keys.is_eased(Prop::PosX));
    }

    #[test]
    fn text_clip_lives_on_video_tracks_and_survives_save() {
        let (mut p, _, _) = project_with_media();
        let text = crate::text::TextPreset::Title.text(p.height);
        assert!(p.add_text(2, text.clone(), 0.0, 3.0).is_none(), "надписи нужна видеодорожка");
        let id = p.add_text(0, text.clone(), 1.0, 3.0).unwrap();
        let clip = p.clip(id).unwrap().clone();
        assert!(clip.generated() && clip.effect.is_none());
        assert_eq!((clip.start, clip.duration, clip.text.as_ref()), (1.0, 3.0, Some(&text)));
        // Надпись, как и эффект, растягивается свободно и режется на две надписи.
        p.trim_end(id, 9.0);
        let right = p.split(id, 5.0).unwrap();
        assert_eq!(p.clip(right).unwrap().text.as_ref(), Some(&text));

        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clip(id).unwrap().text, p.clip(id).unwrap().text);
    }

    #[test]
    fn captions_get_their_own_track_and_share_a_style() {
        let (mut p, v, _) = project_with_media();
        p.append_clip(v).unwrap();
        let a = p.add_caption("Первая", 1.0, 2.0).unwrap();
        let b = p.add_caption("Вторая", 4.0, 1.5).unwrap();
        // Дорожка субтитров одна, лежит сверху; прежние дорожки сдвинулись на одну вниз.
        assert_eq!(p.tracks.len(), 5);
        assert!(p.tracks[0].captions && p.tracks[0].clips.len() == 2);
        assert_eq!(p.find_clip(a).unwrap().0, 0);
        assert_eq!(p.tracks[2].clips.len(), 1, "основная дорожка на месте");
        // Наложения на неё не попадают, файлы тоже; субтитр можно унести на обычную видеодорожку.
        assert_eq!(p.overlay_track(), Some(1));
        assert!(!p.compatible(v, 0));
        let effect = p.add_effect(1, EffectKind::Smoke, 0.0).unwrap();
        assert!(!p.clip_fits(p.clip(effect).unwrap(), 0) && p.clip_fits(p.clip(a).unwrap(), 1));

        // Новый субтитр стоит внизу кадра; оформление одного переносится на все.
        assert_eq!(p.clip(a).unwrap().pos_y, (1080.0f32 * 0.36).round());
        let clip = p.clip_mut(b).unwrap();
        clip.pos_y = 300.0;
        clip.text.as_mut().unwrap().color = [255, 255, 0];
        p.restyle_captions(b);
        let first = p.clip(a).unwrap();
        assert_eq!((first.pos_y, first.text.as_ref().unwrap().color), (300.0, [255, 255, 0]));
        assert_eq!(first.text.as_ref().unwrap().content, "Первая");
        // Следующий субтитр сразу выглядит так же.
        let c = p.add_caption("Третья", 7.0, 1.0).unwrap();
        assert_eq!(p.clip(c).unwrap().text.as_ref().unwrap().color, [255, 255, 0]);
        let order: Vec<&str> = p.captions().iter().map(|c| c.text.as_ref().unwrap().content.as_str()).collect();
        assert_eq!(order, ["Первая", "Вторая", "Третья"]);

        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert!(back.tracks[0].captions && !back.tracks[1].captions);
    }

    #[test]
    fn clips_are_duplicated_pasted_and_deleted_with_a_shift() {
        let (mut p, v, _) = project_with_media();
        let a = p.append_clip(v).unwrap();
        let b = p.split(a, 4.0).unwrap();
        let c = p.split(b, 7.0).unwrap();
        p.set_transition(b, TransitionKind::Fade);
        p.clip_mut(b).unwrap().flip_h = true;

        // Копия встаёт в ближайшее свободное место после оригинала и не уносит с собой переход.
        let copy = p.duplicate(b).unwrap();
        let clip = p.clip(copy).unwrap();
        assert_eq!((clip.start, clip.duration, clip.flip_h), (10.0, 3.0, true));
        assert!(clip.transition.is_none() && clip.id != b);
        // Вставка на другую дорожку — у курсора.
        let pasted = p.paste(&p.clip(b).unwrap().clone(), 0, 2.0).unwrap();
        assert_eq!((p.find_clip(pasted).unwrap().0, p.clip(pasted).unwrap().start), (0, 2.0));
        assert!(p.paste(&p.clip(b).unwrap().clone(), 2, 0.0).is_none(), "видео не ложится на звуковую дорожку");

        // Удаление со сдвигом: следующие клипы дорожки подтягиваются на место удалённого.
        assert!(p.ripple_delete(b));
        assert_eq!((p.clip(a).unwrap().start, p.clip(c).unwrap().start, p.clip(copy).unwrap().start), (0.0, 4.0, 7.0));
        assert_eq!(p.clip(pasted).unwrap().start, 2.0, "другая дорожка не тронута");
    }

    #[test]
    fn colour_and_flip_are_saved_only_when_set() {
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        let plain = serde_json::to_string(p.clip(id).unwrap()).unwrap();
        assert!(!plain.contains("look") && !plain.contains("flip") && !plain.contains("text") && !plain.contains("vfx"));
        let clip = p.clip_mut(id).unwrap();
        clip.look.filter = crate::look::FilterKind::Noir;
        clip.flip_v = true;
        let json = serde_json::to_string(p.clip(id).unwrap()).unwrap();
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!((back.look.clone(), back.flip_h, back.flip_v), (p.clip(id).unwrap().look.clone(), false, true));
        // Сброс трансформации возвращает и отражение.
        p.clip_mut(id).unwrap().reset_transform();
        assert!(!p.clip(id).unwrap().flip_v);
    }

    #[test]
    fn clip_effects_survive_save_and_follow_the_clip() {
        use crate::vfx::{Blend, Fx, FxKind, Mask, MaskShape};
        let (mut p, v, _) = project_with_media();
        let id = p.append_clip(v).unwrap();
        assert!(!p.clip(id).unwrap().styled());
        let clip = p.clip_mut(id).unwrap();
        clip.vfx.blend = Blend::Screen;
        clip.vfx.mask = Some(Mask::new(MaskShape::Ellipse));
        clip.vfx.stack = vec![Fx::new(FxKind::Glow), Fx { on: false, ..Fx::new(FxKind::Blur) }];
        assert!(clip.styled());
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clip(id).unwrap().vfx, p.clip(id).unwrap().vfx);
        // Обе части разрезанного клипа и его копия выглядят так же.
        let right = p.split(id, 4.0).unwrap();
        let copy = p.duplicate(right).unwrap();
        assert_eq!(p.clip(right).unwrap().vfx, p.clip(id).unwrap().vfx);
        assert_eq!(p.clip(copy).unwrap().vfx.stack.len(), 2);
    }

    #[test]
    fn frame_filling_effect_covers_the_whole_frame() {
        let (mut p, _, _) = project_with_media();
        let snow = p.add_effect(0, EffectKind::Snow, 0.0).unwrap();
        assert!((p.clip(snow).unwrap().scale - 1920.0 / 1080.0).abs() < 1e-6);
        let boom = p.add_effect(0, EffectKind::Explosion, 5.0).unwrap();
        assert_eq!(p.clip(boom).unwrap().scale, 1.0);
    }

    #[test]
    fn undo_redo_restores_tracks() {
        let (mut p, v, _) = project_with_media();
        let mut h = History::default();
        h.push(&p.tracks);
        p.append_clip(v).unwrap();
        assert!(h.undo(&mut p));
        assert!(p.is_empty());
        assert!(h.redo(&mut p));
        assert!(!p.is_empty());
    }

    #[test]
    fn timecode_format() {
        assert_eq!(timecode(14.0 + 8.0 / 30.0, 30.0), "00:00:14:08");
        assert_eq!(short_time(134.0), "02:14");
    }
}
