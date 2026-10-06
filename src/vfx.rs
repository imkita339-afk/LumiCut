//! Визуальные эффекты клипа: стопка эффектов, маска с кадрированием и режим наложения.
//! Как и цвет, всё считается одними и теми же числами дважды: здесь — по кадрам предпросмотра,
//! при экспорте — фильтрами ffmpeg.

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::look::Look;
use crate::text::Raster;

/// Наименьший кадр, который имеет смысл делить между потоками, в точках.
const PAR_MIN: usize = 128 * 1024;
/// Наибольшее число потоков на один кадр.
const PAR_MAX: usize = 6;
/// Сколько картинок-масок помнит обработчик кадров.
const MATTES: usize = 6;

/// Сколько знаков после запятой уходит в ffmpeg; предпросмотр считает с теми же округлёнными числами.
fn rounded(x: f32) -> f32 {
    (x * 10_000.0).round() / 10_000.0
}

fn threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, PAR_MAX))
}

/// Обрабатывает кадр RGBA полосами строк в нескольких потоках: `f` получает номер первой строки
/// полосы и сами строки. Маленький кадр обрабатывается на месте.
pub fn bands(buf: &mut [u8], w: usize, f: impl Fn(usize, &mut [u8]) + Sync) {
    let row = (w * 4).max(4);
    let h = buf.len() / row;
    let n = if w * h < PAR_MIN { 1 } else { threads().min(h) };
    if n <= 1 {
        return f(0, buf);
    }
    let per = h.div_ceil(n);
    std::thread::scope(|s| {
        for (i, chunk) in buf.chunks_mut(per * row).enumerate() {
            let f = &f;
            s.spawn(move || f(i * per, chunk));
        }
    });
}

/// Как клип ложится на то, что под ним.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Normal,
    /// Чёрное прозрачно, светлое остаётся: для огня, дыма и бликов, снятых на чёрном фоне.
    Screen,
    /// Яркость клипа прибавляется к картинке под ним.
    Add,
}

impl Blend {
    pub const ALL: [Blend; 3] = [Blend::Normal, Blend::Screen, Blend::Add];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Обычное",
            Blend::Screen => "Экран",
            Blend::Add => "Сложение",
        }
    }

    fn is_normal(&self) -> bool {
        *self == Blend::Normal
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum FxKind {
    Blur,
    Glow,
    Sharpen,
    Vignette,
    Grain,
    RgbShift,
    Pixelate,
    Hue,
    Invert,
    Posterize,
    Mirror,
    Scanlines,
}

/// Ползунок эффекта: подпись, границы и значение по умолчанию в процентах.
pub struct Knob {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

const fn knob(label: &'static str, min: f32, max: f32, default: f32) -> Option<Knob> {
    Some(Knob { label, min, max, default })
}

/// Какая половина кадра остаётся при отражении; вторая становится её зеркальной копией.
pub const MIRRORS: [&str; 4] = ["Левая", "Правая", "Верхняя", "Нижняя"];

impl FxKind {
    pub const ALL: [FxKind; 12] = [
        FxKind::Blur,
        FxKind::Glow,
        FxKind::Sharpen,
        FxKind::Vignette,
        FxKind::Grain,
        FxKind::RgbShift,
        FxKind::Pixelate,
        FxKind::Hue,
        FxKind::Invert,
        FxKind::Posterize,
        FxKind::Mirror,
        FxKind::Scanlines,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FxKind::Blur => "Размытие",
            FxKind::Glow => "Свечение",
            FxKind::Sharpen => "Резкость",
            FxKind::Vignette => "Виньетка",
            FxKind::Grain => "Зерно",
            FxKind::RgbShift => "RGB-сдвиг",
            FxKind::Pixelate => "Пиксели",
            FxKind::Hue => "Оттенок",
            FxKind::Invert => "Негатив",
            FxKind::Posterize => "Постер",
            FxKind::Mirror => "Зеркало",
            FxKind::Scanlines => "ТВ-строки",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            FxKind::Blur => "Мягкое размытие всего кадра",
            FxKind::Glow => "Светлые места светятся и расплываются",
            FxKind::Sharpen => "Подчёркивает мелкие детали",
            FxKind::Vignette => "Затемняет края кадра",
            FxKind::Grain => "Плёночный шум, который меняется от кадра к кадру",
            FxKind::RgbShift => "Красный и синий расходятся в стороны, как на старой оптике",
            FxKind::Pixelate => "Кадр складывается из крупных квадратов",
            FxKind::Hue => "Поворачивает все цвета по цветовому кругу",
            FxKind::Invert => "Меняет цвета на противоположные",
            FxKind::Posterize => "Оставляет несколько ступеней яркости, как на плакате",
            FxKind::Mirror => "Одна половина кадра отражается на другую",
            FxKind::Scanlines => "Тёмные горизонтальные строки, как на кинескопе",
        }
    }

    /// Ползунки эффекта: сила и вторая величина, если она есть.
    pub fn knobs(self) -> [Option<Knob>; 2] {
        match self {
            FxKind::Blur => [knob("Сила", 0.0, 100.0, 25.0), None],
            FxKind::Glow => [knob("Сила", 0.0, 100.0, 60.0), knob("Порог", 0.0, 95.0, 55.0)],
            FxKind::Sharpen => [knob("Сила", 0.0, 100.0, 50.0), None],
            FxKind::Vignette => [knob("Затемнение", 0.0, 100.0, 60.0), knob("Размер", 0.0, 100.0, 50.0)],
            FxKind::Grain => [knob("Сила", 0.0, 100.0, 30.0), None],
            FxKind::RgbShift => [knob("Сдвиг", 0.0, 100.0, 30.0), None],
            FxKind::Pixelate => [knob("Размер", 0.0, 100.0, 20.0), None],
            FxKind::Hue => [knob("Поворот", -100.0, 100.0, 33.0), None],
            FxKind::Invert => [knob("Сила", 0.0, 100.0, 100.0), None],
            FxKind::Posterize => [knob("Сила", 0.0, 100.0, 60.0), None],
            FxKind::Mirror => [None, None],
            FxKind::Scanlines => [knob("Затемнение", 0.0, 100.0, 35.0), knob("Частота", 0.0, 100.0, 50.0)],
        }
    }
}

/// Эффект в стопке клипа.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fx {
    pub kind: FxKind,
    pub on: bool,
    /// Сила, обычно 0..1.
    pub amount: f32,
    /// Вторая величина эффекта; у зеркала — номер половины из `MIRRORS`.
    #[serde(default)]
    pub extra: f32,
}

impl Fx {
    pub fn new(kind: FxKind) -> Self {
        let [a, b] = kind.knobs();
        Self { kind, on: true, amount: a.map_or(1.0, |k| k.default / 100.0), extra: b.map_or(0.0, |k| k.default / 100.0) }
    }

    fn with(kind: FxKind, amount: f32, extra: f32) -> Self {
        Self { kind, on: true, amount, extra }
    }

    /// Меняет ли эффект картинку.
    fn active(&self) -> bool {
        self.on
            && match self.kind {
                FxKind::Mirror => true,
                FxKind::Hue => self.amount.abs() > 1e-3,
                _ => self.amount > 1e-3,
            }
    }

    // Величины, общие для предпросмотра и экспорта; `w`, `h` — размер кадра клипа в точках.

    fn sigma(&self, h: usize) -> f32 {
        (self.amount.clamp(0.0, 1.0) * 0.04 * h as f32 * 100.0).round() / 100.0
    }

    /// Порог свечения и множитель яркости над ним.
    fn glow(&self) -> (f64, f64) {
        let t = (self.extra.clamp(0.0, 0.95) * 255.0).round();
        (t as f64, rounded(self.amount.clamp(0.0, 1.0) * 255.0 / (255.0 - t)) as f64)
    }

    /// Вес соседей в ядре резкости, в шестнадцатых долях.
    fn sharp(&self) -> i32 {
        (self.amount.clamp(0.0, 1.0) * 24.0).round() as i32
    }

    /// Размах шума в ступенях яркости, как его понимает фильтр `noise`.
    fn grain(&self) -> u32 {
        (self.amount.clamp(0.0, 1.0) * 50.0).round() as u32
    }

    fn shift(&self, w: usize) -> usize {
        ((self.amount.clamp(0.0, 1.0) * 0.02 * w as f32).round() as usize).max(1)
    }

    /// Число квадратов по ширине и высоте.
    fn cells(&self, w: usize, h: usize) -> (usize, usize) {
        let block = ((self.amount.clamp(0.0, 1.0) * 0.1 * h as f32).round() as usize).max(2);
        (w.div_ceil(block), h.div_ceil(block))
    }

    fn levels(&self) -> u32 {
        (16.0 - self.amount.clamp(0.0, 1.0) * 14.0).round() as u32
    }

    fn mirror(&self) -> usize {
        (self.extra.round().max(0.0) as usize).min(MIRRORS.len() - 1)
    }

    /// Таблица яркости для эффектов, которые правят каждый цвет отдельно, и то же правило для `lutrgb`.
    fn curve(&self) -> Option<([u8; 256], String)> {
        let table = |f: &dyn Fn(f64) -> f64| -> [u8; 256] { std::array::from_fn(|v| (f(v as f64) as i32).clamp(0, 255) as u8) };
        match self.kind {
            FxKind::Invert => {
                let a = rounded(self.amount.clamp(0.0, 1.0)) as f64;
                Some((table(&|v| v + (255.0 - 2.0 * v) * a), format!("val+(255-2*val)*{a:.4}")))
            }
            FxKind::Posterize => {
                let l = self.levels() as f64;
                Some((table(&|v| (v * l / 256.0).floor() * 255.0 / (l - 1.0)), format!("floor(val*{l}/256)*255/{}", l - 1.0)))
            }
            _ => None,
        }
    }

    /// Матрица поворота цветов: яркость при этом сохраняется.
    fn hue(&self) -> [[f32; 3]; 3] {
        let (s, c) = (self.amount.clamp(-1.0, 1.0) * std::f32::consts::PI).sin_cos();
        [
            [0.213 + 0.787 * c - 0.213 * s, 0.715 - 0.715 * c - 0.715 * s, 0.072 - 0.072 * c + 0.928 * s],
            [0.213 - 0.213 * c + 0.143 * s, 0.715 + 0.285 * c + 0.140 * s, 0.072 - 0.072 * c - 0.283 * s],
            [0.213 - 0.213 * c - 0.787 * s, 0.715 - 0.715 * c + 0.715 * s, 0.072 + 0.928 * c + 0.072 * s],
        ]
        .map(|row| row.map(rounded))
    }

    /// Картинка, на которую умножается цвет кадра: яркость каждой точки 0..255.
    fn shade(&self, w: usize, h: usize) -> Option<Vec<u8>> {
        let amount = self.amount.clamp(0.0, 1.0);
        match self.kind {
            FxKind::Vignette => {
                // Затемнение начинается тем дальше от середины, чем больше «размер».
                let start = 0.15 + 0.7 * self.extra.clamp(0.0, 1.0);
                let mut out = Vec::with_capacity(w * h);
                for y in 0..h {
                    let dy = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
                    for x in 0..w {
                        let dx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                        let far = ((dx * dx + dy * dy) / 2.0).sqrt();
                        let t = ((far - start) / (1.05 - start)).clamp(0.0, 1.0);
                        out.push((255.0 * (1.0 - amount * t * t * (3.0 - 2.0 * t))).round() as u8);
                    }
                }
                Some(out)
            }
            FxKind::Scanlines => {
                let lines = 40.0 + 200.0 * self.extra.clamp(0.0, 1.0);
                let period = ((h as f32 / lines).round() as usize).max(2);
                let dark = (255.0 * (1.0 - amount)).round() as u8;
                let mut out = Vec::with_capacity(w * h);
                for y in 0..h {
                    let v = if y % period < period / 2 { dark } else { 255 };
                    out.extend(std::iter::repeat_n(v, w));
                }
                Some(out)
            }
            _ => None,
        }
    }
}

/// Готовый набор эффектов.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VfxPreset {
    Vhs,
    Dream,
    Cyber,
    Comic,
    Retro,
    Glitch,
}

impl VfxPreset {
    pub const ALL: [VfxPreset; 6] =
        [VfxPreset::Vhs, VfxPreset::Dream, VfxPreset::Cyber, VfxPreset::Comic, VfxPreset::Retro, VfxPreset::Glitch];

    pub fn label(self) -> &'static str {
        match self {
            VfxPreset::Vhs => "VHS",
            VfxPreset::Dream => "Сон",
            VfxPreset::Cyber => "Киберпанк",
            VfxPreset::Comic => "Комикс",
            VfxPreset::Retro => "Старое кино",
            VfxPreset::Glitch => "Глитч",
        }
    }

    pub fn stack(self) -> Vec<Fx> {
        use FxKind::*;
        let fx = Fx::with;
        match self {
            VfxPreset::Vhs => vec![fx(RgbShift, 0.25, 0.0), fx(Blur, 0.04, 0.0), fx(Scanlines, 0.3, 0.6), fx(Grain, 0.35, 0.0)],
            VfxPreset::Dream => vec![fx(Glow, 0.7, 0.45), fx(Blur, 0.05, 0.0), fx(Vignette, 0.35, 0.6)],
            VfxPreset::Cyber => vec![fx(Hue, -0.22, 0.0), fx(Glow, 0.5, 0.5), fx(RgbShift, 0.2, 0.0)],
            VfxPreset::Comic => vec![fx(Posterize, 0.75, 0.0), fx(Sharpen, 0.8, 0.0)],
            VfxPreset::Retro => vec![fx(Grain, 0.5, 0.0), fx(Vignette, 0.7, 0.4), fx(Blur, 0.03, 0.0)],
            VfxPreset::Glitch => vec![fx(RgbShift, 0.6, 0.0), fx(Pixelate, 0.06, 0.0), fx(Scanlines, 0.25, 0.8)],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MaskShape {
    Rect,
    Ellipse,
}

/// Маска: видимой остаётся только часть клипа внутри фигуры (или снаружи).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub shape: MaskShape,
    /// Середина фигуры в долях кадра клипа: 0.5 — середина кадра.
    pub x: f32,
    pub y: f32,
    /// Ширина и высота фигуры в долях кадра клипа.
    pub w: f32,
    pub h: f32,
    /// Ширина мягкого края, 0..1.
    pub feather: f32,
    pub invert: bool,
}

impl Mask {
    pub fn new(shape: MaskShape) -> Self {
        Self { shape, x: 0.5, y: 0.5, w: 0.6, h: 0.6, feather: 0.15, invert: false }
    }

    /// Какая доля точки с центром в (`x`, `y`) остаётся видимой в кадре `w`×`h` точек.
    fn cover(&self, x: f32, y: f32, w: f32, h: f32) -> f32 {
        let (a, b) = ((self.w * w / 2.0).max(0.5), (self.h * h / 2.0).max(0.5));
        let (dx, dy) = (x - self.x * w, y - self.y * h);
        // Расстояние до края фигуры в точках: внутри оно отрицательное.
        let d = match self.shape {
            MaskShape::Rect => {
                let (qx, qy) = (dx.abs() - a, dy.abs() - b);
                qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0)
            }
            MaskShape::Ellipse => ((dx / a).hypot(dy / b) - 1.0) * a.min(b),
        };
        let soft = (self.feather.clamp(0.0, 1.0) * a.min(b)).max(0.75);
        let t = (0.5 - d / (2.0 * soft)).clamp(0.0, 1.0);
        let inside = t * t * (3.0 - 2.0 * t);
        if self.invert {
            1.0 - inside
        } else {
            inside
        }
    }
}

/// Наибольшая доля кадра, которую можно срезать с одной стороны.
pub const MAX_CROP: f32 = 0.95;

/// Эффекты клипа.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vfx {
    #[serde(default, skip_serializing_if = "Blend::is_normal")]
    pub blend: Blend,
    /// Сколько срезано слева, сверху, справа и снизу, в долях кадра клипа.
    #[serde(default, skip_serializing_if = "no_crop")]
    pub crop: [f32; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Mask>,
    /// Эффекты в том порядке, в каком они применяются.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack: Vec<Fx>,
}

fn no_crop(crop: &[f32; 4]) -> bool {
    crop.iter().all(|c| *c <= 1e-4)
}

impl Vfx {
    /// Ничего не задано: такие эффекты в файл проекта не пишутся.
    pub fn is_empty(&self) -> bool {
        self.blend.is_normal() && no_crop(&self.crop) && self.mask.is_none() && self.stack.is_empty()
    }

    /// Картинка клипа остаётся как есть.
    pub fn is_plain(&self) -> bool {
        self.blend.is_normal() && !self.shaped() && !self.stack.iter().any(Fx::active)
    }

    /// Есть ли маска или кадрирование.
    pub fn shaped(&self) -> bool {
        self.mask.is_some() || !no_crop(&self.crop)
    }

    /// Сколько эффектов включено.
    pub fn active(&self) -> usize {
        self.stack.iter().filter(|fx| fx.active()).count()
    }

    /// Прозрачность, которую дают маска и кадрирование: 0..255 на каждую точку кадра.
    fn shape(&self, w: usize, h: usize) -> Vec<u8> {
        let [l, t, r, b] = self.crop.map(|c| c.clamp(0.0, MAX_CROP));
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let kept = u >= l && u <= 1.0 - r && v >= t && v <= 1.0 - b;
                let cover = match (&self.mask, kept) {
                    (_, false) => 0.0,
                    (Some(mask), true) => mask.cover(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32),
                    (None, true) => 1.0,
                };
                out.push((cover * 255.0).round() as u8);
            }
        }
        out
    }
}

// ---------- Размытие ----------

/// Радиусы трёх усреднений подряд, которые вместе дают размытие по Гауссу с отклонением `sigma`.
fn box_radii(sigma: f32) -> [usize; 3] {
    const N: f32 = 3.0;
    let ideal = (12.0 * sigma * sigma / N + 1.0).sqrt();
    let mut lo = ideal.floor() as i32;
    if lo % 2 == 0 {
        lo -= 1;
    }
    let lo = lo.max(1);
    let (lf, hi) = (lo as f32, lo + 2);
    let m = ((12.0 * sigma * sigma - N * lf * lf - 4.0 * N * lf - 3.0 * N) / (-4.0 * lf - 4.0)).round() as i32;
    std::array::from_fn(|i| ((if (i as i32) < m { lo } else { hi }) as usize - 1) / 2)
}

/// Делитель окна в виде множителя с 16 двоичными знаками после запятой.
fn window(r: usize) -> u32 {
    let n = (2 * r + 1) as u32;
    (65_536 + n / 2) / n
}

/// Одна строка: усреднение по окну `2r + 1` с повторением крайних точек.
fn blur_line(src: &[u8], dst: &mut [u8], r: usize) {
    let w = src.len() / 4;
    let inv = window(r);
    let at = |x: usize, c: usize| src[x.min(w - 1) * 4 + c] as u32;
    let mut sum = [0u32; 4];
    for (c, s) in sum.iter_mut().enumerate() {
        *s = at(0, c) * (r as u32 + 1) + (1..=r).map(|i| at(i, c)).sum::<u32>();
    }
    for x in 0..w {
        for (c, s) in sum.iter_mut().enumerate() {
            dst[x * 4 + c] = ((*s * inv + 32_768) >> 16).min(255) as u8;
            *s = *s + at(x + r + 1, c) - at(x.saturating_sub(r), c);
        }
    }
}

/// Все три усреднения по строкам: строки друг от друга не зависят, поэтому потоки запускаются один раз.
fn blur_rows(buf: &mut [u8], w: usize, radii: [usize; 3]) {
    let row = w * 4;
    bands(buf, w, |_, rows| {
        let mut tmp = vec![0u8; row];
        for line in rows.chunks_exact_mut(row) {
            for r in radii.into_iter().filter(|r| *r > 0) {
                blur_line(line, &mut tmp, r);
                line.copy_from_slice(&tmp);
            }
        }
    });
}

/// Усреднение по столбцам: сумма окна едет вниз сразу по всей строке.
fn blur_columns(src: &[u8], dst: &mut [u8], w: usize, r: usize) {
    let row = w * 4;
    let h = src.len() / row;
    let inv = window(r);
    bands(dst, w, |y0, rows| {
        let line = |y: isize| &src[y.clamp(0, h as isize - 1) as usize * row..][..row];
        let mut sums = vec![0u32; row];
        for k in -(r as isize)..=r as isize {
            for (s, v) in sums.iter_mut().zip(line(y0 as isize + k)) {
                *s += *v as u32;
            }
        }
        for (j, out) in rows.chunks_exact_mut(row).enumerate() {
            let y = (y0 + j) as isize;
            for (o, s) in out.iter_mut().zip(&sums) {
                *o = ((*s * inv + 32_768) >> 16).min(255) as u8;
            }
            let (add, sub) = (line(y + r as isize + 1), line(y - r as isize));
            for ((s, a), b) in sums.iter_mut().zip(add).zip(sub) {
                *s = *s + *a as u32 - *b as u32;
            }
        }
    });
}

/// Размытие по Гауссу с отклонением `sigma` точек.
fn gauss(buf: &mut Vec<u8>, w: usize, sigma: f32) {
    let radii = box_radii(sigma);
    if radii.iter().all(|r| *r == 0) {
        return;
    }
    blur_rows(buf, w, radii);
    let mut tmp = vec![0u8; buf.len()];
    for r in radii.into_iter().filter(|r| *r > 0) {
        blur_columns(buf, &mut tmp, w, r);
        std::mem::swap(buf, &mut tmp);
    }
}

// ---------- Остальные эффекты ----------

/// Правит три цвета каждой точки по таблице.
fn remap(buf: &mut [u8], w: usize, table: &[u8; 256]) {
    bands(buf, w, |_, rows| {
        for p in rows.chunks_exact_mut(4) {
            for c in &mut p[..3] {
                *c = table[*c as usize];
            }
        }
    });
}

/// Умножает цвет каждой точки на яркость картинки `shade`.
fn darken(buf: &mut [u8], w: usize, shade: &[u8]) {
    bands(buf, w, |y0, rows| {
        for (p, m) in rows.chunks_exact_mut(4).zip(&shade[y0 * w..]) {
            for c in &mut p[..3] {
                *c = (*c as u32 * *m as u32 / 255) as u8;
            }
        }
    });
}

/// Умножает прозрачность каждой точки на картинку `matte`.
fn cut(buf: &mut [u8], w: usize, matte: &[u8]) {
    bands(buf, w, |y0, rows| {
        for (p, m) in rows.chunks_exact_mut(4).zip(&matte[y0 * w..]) {
            p[3] = (p[3] as u32 * *m as u32 / 255) as u8;
        }
    });
}

fn glow(buf: &mut [u8], w: usize, h: usize, fx: &Fx) {
    let (threshold, gain) = fx.glow();
    let table: [u8; 256] = std::array::from_fn(|v| (((v as f64 - threshold) * gain) as i32).clamp(0, 255) as u8);
    let mut bright = buf.to_vec();
    remap(&mut bright, w, &table);
    gauss(&mut bright, w, glow_sigma(h));
    bands(buf, w, |y0, rows| {
        for (p, q) in rows.chunks_exact_mut(4).zip(bright[y0 * w * 4..].chunks_exact(4)) {
            for i in 0..3 {
                p[i] = 255 - ((255 - p[i] as u32) * (255 - q[i] as u32) / 255) as u8;
            }
        }
    });
}

/// Насколько расплывается свечение: доля высоты кадра.
fn glow_sigma(h: usize) -> f32 {
    (0.025 * h as f32 * 100.0).round() / 100.0
}

fn sharpen(buf: &mut [u8], w: usize, h: usize, k: i32) {
    let src = buf.to_vec();
    let row = w * 4;
    bands(buf, w, |y0, rows| {
        for (j, out) in rows.chunks_exact_mut(row).enumerate() {
            let y = y0 + j;
            let (up, mid, down) = (&src[y.saturating_sub(1) * row..], &src[y * row..], &src[(y + 1).min(h - 1) * row..]);
            for x in 0..w {
                let (l, r) = (x.saturating_sub(1) * 4, (x + 1).min(w - 1) * 4);
                for c in 0..3 {
                    let around = up[x * 4 + c] as i32 + down[x * 4 + c] as i32 + mid[l + c] as i32 + mid[r + c] as i32;
                    let sum = (16 + 4 * k) * mid[x * 4 + c] as i32 - k * around;
                    out[x * 4 + c] = ((sum as f32 * 0.0625 + 0.5) as i32).clamp(0, 255) as u8;
                }
            }
        }
    });
}

/// Случайное число, определяемое номером точки и кадра.
fn noise(i: u32, frame: u32) -> u32 {
    let mut x = i ^ frame.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn grain(buf: &mut [u8], w: usize, strength: u32, frame: u64) {
    // Шум при экспорте добавляется к яркости в её узком диапазоне, в цвете он заметнее в 255/219 раза.
    let span = ((strength as f32 * 255.0 / 219.0).round() as u32).max(1);
    let frame = frame as u32;
    bands(buf, w, |y0, rows| {
        for (i, p) in rows.chunks_exact_mut(4).enumerate() {
            let n = (noise((y0 * w + i) as u32, frame) % span) as i32 - span as i32 / 2;
            for c in &mut p[..3] {
                *c = (*c as i32 + n).clamp(0, 255) as u8;
            }
        }
    });
}

fn rgb_shift(buf: &mut [u8], w: usize, by: usize) {
    let row = w * 4;
    bands(buf, w, |_, rows| {
        let mut src = vec![0u8; row];
        for line in rows.chunks_exact_mut(row) {
            src.copy_from_slice(line);
            for x in 0..w {
                line[x * 4] = src[x.saturating_sub(by) * 4];
                line[x * 4 + 2] = src[(x + by).min(w - 1) * 4 + 2];
            }
        }
    });
}

fn pixelate(buf: &mut [u8], w: usize, h: usize, cells: (usize, usize)) {
    let (cw, ch) = cells;
    let mut sums = vec![[0u32; 5]; cw * ch];
    for y in 0..h {
        let cy = y * ch / h;
        for (x, p) in buf[y * w * 4..][..w * 4].chunks_exact(4).enumerate() {
            let s = &mut sums[cy * cw + x * cw / w];
            for c in 0..4 {
                s[c] += p[c] as u32;
            }
            s[4] += 1;
        }
    }
    let mean: Vec<[u8; 4]> = sums.iter().map(|s| [0, 1, 2, 3].map(|c| ((s[c] + s[4] / 2) / s[4].max(1)) as u8)).collect();
    bands(buf, w, |y0, rows| {
        for (j, line) in rows.chunks_exact_mut(w * 4).enumerate() {
            let cy = (y0 + j) * ch / h;
            for (x, p) in line.chunks_exact_mut(4).enumerate() {
                p.copy_from_slice(&mean[cy * cw + x * cw / w]);
            }
        }
    });
}

fn hue(buf: &mut [u8], w: usize, matrix: [[f32; 3]; 3]) {
    // Матрица в целых числах: 12 знаков после двоичной запятой.
    let m = matrix.map(|row| row.map(|v| (v * 4096.0).round() as i32));
    bands(buf, w, |_, rows| {
        for p in rows.chunks_exact_mut(4) {
            let c = [p[0] as i32, p[1] as i32, p[2] as i32];
            for (out, row) in p.iter_mut().zip(&m) {
                *out = ((row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + 2048) >> 12).clamp(0, 255) as u8;
            }
        }
    });
}

fn mirror(buf: &mut [u8], w: usize, h: usize, side: usize) {
    let row = w * 4;
    match side {
        0 | 1 => bands(buf, w, |_, rows| {
            for line in rows.chunks_exact_mut(row) {
                for x in 0..w / 2 {
                    // Слева остаётся левая половина, справа — её отражение (или наоборот).
                    let (keep, fill) = if side == 0 { (x, w - 1 - x) } else { (w - 1 - x, x) };
                    line.copy_within(keep * 4..keep * 4 + 4, fill * 4);
                }
            }
        }),
        _ => {
            for y in 0..h / 2 {
                let (keep, fill) = if side == 2 { (y, h - 1 - y) } else { (h - 1 - y, y) };
                buf.copy_within(keep * row..(keep + 1) * row, fill * row);
            }
        }
    }
}

/// Срезает край непрозрачной части кадра: каждый проход убирает по точке со всех сторон.
pub fn choke(buf: &mut [u8], w: usize, h: usize, passes: u32) {
    if passes == 0 || w == 0 || h == 0 {
        return;
    }
    let mut a: Vec<u8> = buf.chunks_exact(4).map(|p| p[3]).collect();
    let mut b = vec![0u8; a.len()];
    for _ in 0..passes {
        // Наименьшее по квадрату 3×3: сначала по строкам, потом по столбцам.
        for y in 0..h {
            let line = &a[y * w..][..w];
            for x in 0..w {
                b[y * w + x] = line[x.saturating_sub(1)].min(line[x]).min(line[(x + 1).min(w - 1)]);
            }
        }
        for y in 0..h {
            let (up, down) = (y.saturating_sub(1) * w, (y + 1).min(h - 1) * w);
            for x in 0..w {
                a[y * w + x] = b[up + x].min(b[y * w + x]).min(b[down + x]);
            }
        }
    }
    for (p, v) in buf.chunks_exact_mut(4).zip(&a) {
        p[3] = *v;
    }
}

// ---------- Обработка кадра ----------

/// Какая картинка-маска нужна: вид, величины, от которых она зависит, и размер кадра.
type MatteKey = (u8, [u32; 12], usize, usize);

/// Картинки-маски, посчитанные для недавних кадров: от кадра к кадру они не меняются.
#[derive(Default)]
pub struct Mattes(Vec<(MatteKey, Arc<Vec<u8>>)>);

impl Mattes {
    fn get(&mut self, tag: u8, nums: &[f32], w: usize, h: usize, make: impl FnOnce() -> Vec<u8>) -> Arc<Vec<u8>> {
        let mut bits = [0u32; 12];
        for (b, n) in bits.iter_mut().zip(nums) {
            *b = n.to_bits();
        }
        let key = (tag, bits, w, h);
        if let Some(i) = self.0.iter().position(|m| m.0 == key) {
            // Недавно нужная маска переезжает в конец: вытесняются давно не нужные.
            let hit = self.0.remove(i);
            self.0.push(hit);
        } else {
            if self.0.len() >= MATTES {
                self.0.remove(0);
            }
            self.0.push((key, Arc::new(make())));
        }
        self.0[self.0.len() - 1].1.clone()
    }

    fn shade(&mut self, fx: &Fx, w: usize, h: usize) -> Option<Arc<Vec<u8>>> {
        matches!(fx.kind, FxKind::Vignette | FxKind::Scanlines)
            .then(|| self.get(fx.kind as u8 + 1, &[fx.amount, fx.extra], w, h, || fx.shade(w, h).unwrap_or_default()))
    }

    fn shape(&mut self, vfx: &Vfx, w: usize, h: usize) -> Option<Arc<Vec<u8>>> {
        if !vfx.shaped() {
            return None;
        }
        let mut nums = [0f32; 11];
        nums[..4].copy_from_slice(&vfx.crop);
        if let Some(m) = &vfx.mask {
            let shape = if m.shape == MaskShape::Rect { 1.0 } else { 2.0 };
            nums[4..].copy_from_slice(&[shape, m.x, m.y, m.w, m.h, m.feather, if m.invert { 1.0 } else { 0.0 }]);
        }
        Some(self.get(0, &nums, w, h, || vfx.shape(w, h)))
    }
}

/// Всё, что делается с кадром клипа перед показом: хромакей, цвет, эффекты, маска и наложение.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recipe {
    pub look: Look,
    pub vfx: Vfx,
    /// Вместо картинки показать её прозрачность: белое видно, чёрное вырезано.
    pub matte: bool,
}

impl Recipe {
    /// Кадр показывается как есть.
    pub fn is_plain(&self) -> bool {
        !self.matte && self.look.is_neutral() && self.vfx.is_plain()
    }

    /// Готовит кадр RGBA к показу. Возвращает кадр с цветом, уже умноженным на непрозрачность.
    pub fn cook(&self, raw: &[u8], w: usize, h: usize, frame: u64, mattes: &mut Mattes) -> Vec<u8> {
        let mut buf = raw.to_vec();
        if w == 0 || h == 0 {
            return buf;
        }
        self.look.apply_key(&mut buf, w);
        if let Some(key) = &self.look.key {
            choke(&mut buf, w, h, key.choke_passes(h as u32));
        }
        self.look.apply_grade(&mut buf, w);
        for fx in self.vfx.stack.iter().filter(|fx| fx.active()) {
            apply(fx, &mut buf, w, h, frame, mattes);
        }
        if let Some(matte) = mattes.shape(&self.vfx, w, h) {
            cut(&mut buf, w, &matte);
        }
        if self.matte {
            for p in buf.chunks_exact_mut(4) {
                p.copy_from_slice(&[p[3], p[3], p[3], 255]);
            }
        } else {
            premultiply(&mut buf, w, self.vfx.blend);
        }
        buf
    }
}

fn apply(fx: &Fx, buf: &mut Vec<u8>, w: usize, h: usize, frame: u64, mattes: &mut Mattes) {
    match fx.kind {
        FxKind::Blur => gauss(buf, w, fx.sigma(h)),
        FxKind::Glow => glow(buf, w, h, fx),
        FxKind::Sharpen => sharpen(buf, w, h, fx.sharp()),
        FxKind::Vignette | FxKind::Scanlines => {
            if let Some(shade) = mattes.shade(fx, w, h) {
                darken(buf, w, &shade);
            }
        }
        FxKind::Grain => grain(buf, w, fx.grain(), frame),
        FxKind::RgbShift => rgb_shift(buf, w, fx.shift(w)),
        FxKind::Pixelate => pixelate(buf, w, h, fx.cells(w, h)),
        FxKind::Hue => hue(buf, w, fx.hue()),
        FxKind::Invert | FxKind::Posterize => {
            if let Some((table, _)) = fx.curve() {
                remap(buf, w, &table);
            }
        }
        FxKind::Mirror => mirror(buf, w, h, fx.mirror()),
    }
}

/// Умножает цвет на непрозрачность и задаёт прозрачность по режиму наложения: при сложении
/// клип ничего не закрывает, в режиме «Экран» закрывает тем сильнее, чем он ярче.
pub fn premultiply(buf: &mut [u8], w: usize, blend: Blend) {
    bands(buf, w, |_, rows| {
        for p in rows.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a < 255 {
                for c in &mut p[..3] {
                    let t = *c as u32 * a + 128;
                    *c = ((t + (t >> 8)) >> 8) as u8;
                }
            }
            match blend {
                Blend::Normal => {}
                Blend::Screen => p[3] = p[0].max(p[1]).max(p[2]),
                Blend::Add => p[3] = 0,
            }
        }
    });
}

// ---------- Экспорт ----------

/// Кусок цепочки фильтров ffmpeg для клипа и картинки, которые ему нужны входами.
pub struct Filters {
    /// Фильтры через запятую, начиная с запятой; пусто, если кадр не меняется.
    pub chain: String,
    /// Картинки-маски: каждая должна стать входом ffmpeg, первая — с номером `first_input`.
    pub rasters: Vec<Raster>,
}

struct Builder {
    chain: String,
    tag: String,
    labels: u32,
    first_input: usize,
    rasters: Vec<Raster>,
}

impl Builder {
    fn add(&mut self, filter: &str) {
        self.chain.push(',');
        self.chain.push_str(filter);
    }

    fn label(&mut self) -> String {
        self.labels += 1;
        format!("{}{}", self.tag, self.labels)
    }

    /// Умножает поток на картинку: она приходит отдельным входом и держится всё время клипа.
    fn multiply(&mut self, raster: Raster, modes: &str) {
        let (a, b) = (self.label(), self.label());
        let input = self.first_input + self.rasters.len();
        self.rasters.push(raster);
        self.chain += &format!(
            ",format=gbrap[{a}];[{input}:v]format=gbrap,loop=loop=-1:size=1:start=0[{b}];[{a}][{b}]blend={modes}:shortest=1,format=rgba"
        );
    }

    fn fx(&mut self, fx: &Fx, w: usize, h: usize) {
        match fx.kind {
            FxKind::Blur => {
                if box_radii(fx.sigma(h)).iter().any(|r| *r > 0) {
                    self.add(&format!("gblur=sigma={:.2}", fx.sigma(h)));
                }
            }
            FxKind::Glow => {
                let (t, k) = fx.glow();
                let (a, b, c) = (self.label(), self.label(), self.label());
                let curve = format!("clip((val-{t})*{k:.4},0,255)");
                self.chain += &format!(
                    ",format=gbrap,split[{a}][{b}];[{b}]lutrgb=r='{curve}':g='{curve}':b='{curve}',gblur=sigma={:.2}[{c}];\
                     [{a}][{c}]blend=c0_mode=screen:c1_mode=screen:c2_mode=screen,format=rgba",
                    glow_sigma(h)
                );
            }
            FxKind::Sharpen => {
                let k = fx.sharp();
                if k > 0 {
                    let m = format!("0 -{k} 0 -{k} {} -{k} 0 -{k} 0", 16 + 4 * k);
                    self.add(&format!(
                        "format=gbrap,convolution=0m='{m}':1m='{m}':2m='{m}':0rdiv=0.0625:1rdiv=0.0625:2rdiv=0.0625,format=rgba"
                    ));
                }
            }
            FxKind::Vignette | FxKind::Scanlines => {
                if let Some(shade) = fx.shade(w, h) {
                    let rgba = shade.iter().flat_map(|v| [*v, *v, *v, 255]).collect();
                    self.multiply(Raster { w: w as u32, h: h as u32, rgba }, "c0_mode=multiply:c1_mode=multiply:c2_mode=multiply");
                }
            }
            FxKind::Grain => self.add(&format!("format=yuva444p,noise=c0s={}:c0f=t+u,format=rgba", fx.grain())),
            FxKind::RgbShift => {
                let by = fx.shift(w);
                self.add(&format!("rgbashift=rh={by}:bh=-{by}"));
            }
            FxKind::Pixelate => {
                let (cw, ch) = fx.cells(w, h);
                self.add(&format!("scale={cw}:{ch}:flags=area,scale={w}:{h}:flags=neighbor"));
            }
            FxKind::Hue => {
                let m = fx.hue();
                self.add(&format!(
                    "colorchannelmixer=rr={:.4}:rg={:.4}:rb={:.4}:gr={:.4}:gg={:.4}:gb={:.4}:br={:.4}:bg={:.4}:bb={:.4}",
                    m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]
                ));
            }
            FxKind::Invert | FxKind::Posterize => {
                if let Some((_, curve)) = fx.curve() {
                    self.add(&format!("lutrgb=r='{curve}':g='{curve}':b='{curve}'"));
                }
            }
            FxKind::Mirror => {
                let (a, b, c, d) = (self.label(), self.label(), self.label(), self.label());
                // Половина кадра и её отражение встают рядом.
                let (crop, flip, stack) = match fx.mirror() {
                    0 => ("crop=iw/2:ih:0:0", "hflip", "hstack"),
                    1 => ("crop=iw/2:ih:iw/2:0", "hflip", "hstack"),
                    2 => ("crop=iw:ih/2:0:0", "vflip", "vstack"),
                    _ => ("crop=iw:ih/2:0:ih/2", "vflip", "vstack"),
                };
                let kept_first = fx.mirror().is_multiple_of(2);
                let (first, second) = if kept_first { (String::new(), format!(",{flip}")) } else { (format!(",{flip}"), String::new()) };
                self.chain += &format!(",split[{a}][{b}];[{a}]{crop}{first}[{c}];[{b}]{crop}{second}[{d}];[{c}][{d}]{stack}");
            }
        }
    }
}

/// Хромакей, цвет, эффекты и маска клипа как фильтры ffmpeg для кадров RGBA размером `w`×`h`.
/// `tag` — приставка для промежуточных меток, `first_input` — номер входа для первой картинки-маски.
pub fn filters(look: &Look, vfx: &Vfx, w: u32, h: u32, tag: &str, first_input: usize) -> Filters {
    let mut b = Builder { chain: look.filters(h), tag: tag.to_owned(), labels: 0, first_input, rasters: Vec::new() };
    let (w, h) = (w as usize, h as usize);
    for fx in vfx.stack.iter().filter(|fx| fx.active()) {
        b.fx(fx, w, h);
    }
    if vfx.shaped() {
        let rgba = vfx.shape(w, h).iter().flat_map(|v| [255, 255, 255, *v]).collect();
        b.multiply(Raster { w: w as u32, h: h as u32, rgba }, "c3_mode=multiply");
    }
    Filters { chain: b.chain, rasters: b.rasters }
}

/// Фильтры, которые готовят поток клипа к наложению в заданном режиме, и добавка к параметрам `overlay`.
pub fn blend_filters(blend: Blend, tag: &str) -> (String, &'static str) {
    match blend {
        Blend::Normal => (String::new(), ""),
        // Цвет, умноженный на непрозрачность, прибавляется к кадру под клипом.
        Blend::Add => (",format=gbrap,premultiply=inplace=1,colorchannelmixer=aa=0".to_owned(), ":alpha=premultiplied"),
        // Непрозрачность — самый яркий из трёх цветов точки, а цвет делится на неё обратно.
        Blend::Screen => (
            format!(
                ",format=gbrap,premultiply=inplace=1,split[{tag}c][{tag}m];[{tag}m]extractplanes=r+g+b[{tag}r][{tag}g][{tag}b];\
                 [{tag}r][{tag}g]blend=all_mode=lighten[{tag}l];[{tag}l][{tag}b]blend=all_mode=lighten[{tag}x];\
                 [{tag}c][{tag}x]alphamerge,unpremultiply=inplace=1"
            ),
            "",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::ChromaKey;

    /// Кадр `w`×`h`, каждая точка которого задана функцией.
    fn frame(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|i| f(i % w, i / w)).collect()
    }

    fn cooked(vfx: Vfx, raw: &[u8], w: usize, h: usize) -> Vec<u8> {
        Recipe { vfx, ..Recipe::default() }.cook(raw, w, h, 0, &mut Mattes::default())
    }

    fn one(kind: FxKind, amount: f32, extra: f32) -> Vfx {
        Vfx { stack: vec![Fx::with(kind, amount, extra)], ..Vfx::default() }
    }

    fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        buf[(y * w + x) * 4..][..4].try_into().unwrap()
    }

    #[test]
    fn plain_recipe_keeps_the_frame() {
        let raw = frame(8, 6, |x, y| [x as u8 * 30, y as u8 * 40, 77, 255]);
        let recipe = Recipe::default();
        assert!(recipe.is_plain() && Vfx::default().is_empty());
        assert_eq!(recipe.cook(&raw, 8, 6, 0, &mut Mattes::default()), raw);
        // Выключенный эффект и эффект с нулевой силой картинку не трогают, но в проекте остаются.
        let idle = Vfx { stack: vec![Fx { on: false, ..Fx::new(FxKind::Blur) }, Fx::with(FxKind::Grain, 0.0, 0.0)], ..Vfx::default() };
        assert!(idle.is_plain() && !idle.is_empty() && idle.active() == 0);
        assert_eq!(cooked(idle, &raw, 8, 6), raw);
    }

    #[test]
    fn blur_softens_an_edge_and_keeps_flat_colour() {
        let (w, h) = (64, 48);
        let raw = frame(w, h, |x, _| if x < w / 2 { [0, 0, 0, 255] } else { [240, 240, 240, 255] });
        let out = cooked(one(FxKind::Blur, 1.0, 0.0), &raw, w, h);
        assert_eq!(px(&out, w, 2, 20), [0, 0, 0, 255]);
        assert_eq!(px(&out, w, w - 3, 20), [240, 240, 240, 255]);
        let edge = px(&out, w, w / 2, 20)[0];
        assert!((90..150).contains(&edge), "на границе {edge}");
        let flat = frame(w, h, |_, _| [200, 100, 50, 255]);
        assert_eq!(cooked(one(FxKind::Blur, 1.0, 0.0), &flat, w, h), flat);
    }

    #[test]
    fn big_frames_are_processed_the_same_way_in_several_threads() {
        // Кадр больше порога делится на полосы; результат не должен зависеть от числа потоков.
        let (w, h) = (640, 360);
        let raw = frame(w, h, |x, y| [(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) % 256) as u8, 255]);
        let vfx = Vfx {
            stack: vec![Fx::with(FxKind::Blur, 0.3, 0.0), Fx::with(FxKind::Sharpen, 0.5, 0.0), Fx::with(FxKind::RgbShift, 0.5, 0.0)],
            ..Vfx::default()
        };
        let whole = cooked(vfx.clone(), &raw, w, h);
        // Тот же кадр по частям считать нельзя (размытие смотрит на соседей), поэтому сравнение — с повтором.
        assert_eq!(whole, cooked(vfx, &raw, w, h));
        assert_eq!(whole.len(), raw.len());
        assert!(whole.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn point_effects_change_colours_the_expected_way() {
        let raw = frame(4, 2, |_, _| [200, 100, 20, 255]);
        assert_eq!(px(&cooked(one(FxKind::Invert, 1.0, 0.0), &raw, 4, 2), 4, 0, 0), [55, 155, 235, 255]);
        // Четыре ступени яркости: 0, 85, 170 и 255.
        assert_eq!(px(&cooked(one(FxKind::Posterize, 6.0 / 7.0, 0.0), &raw, 4, 2), 4, 0, 0), [255, 85, 0, 255]);
        // Полный оборот цветов возвращает исходный цвет, половина — меняет его на противоположный по тону.
        assert_eq!(px(&cooked(one(FxKind::Hue, 0.0, 0.0), &raw, 4, 2), 4, 0, 0), [200, 100, 20, 255]);
        let turned = px(&cooked(one(FxKind::Hue, 1.0, 0.0), &raw, 4, 2), 4, 0, 0);
        assert!(turned[2] > turned[0] && turned[2] > 150, "{turned:?}");
        let dark = px(&cooked(one(FxKind::Scanlines, 1.0, 0.0), &frame(4, 80, |_, _| [200, 100, 20, 255]), 4, 80), 4, 0, 0);
        assert_eq!(dark, [0, 0, 0, 255]);
    }

    #[test]
    fn geometry_effects_move_points() {
        let (w, h) = (8, 4);
        let raw = frame(w, h, |x, y| [x as u8 * 10, y as u8 * 10, x as u8, 255]);
        let left = cooked(one(FxKind::Mirror, 1.0, 0.0), &raw, w, h);
        assert_eq!(px(&left, w, 7, 1), px(&raw, w, 0, 1));
        assert_eq!(px(&left, w, 2, 1), px(&raw, w, 2, 1));
        let bottom = cooked(one(FxKind::Mirror, 1.0, 3.0), &raw, w, h);
        assert_eq!(px(&bottom, w, 3, 0), px(&raw, w, 3, 3));
        // Красный уезжает вправо, синий — влево, зелёный стоит на месте.
        let shifted = cooked(one(FxKind::RgbShift, 0.0, 0.0), &raw, w, h);
        assert_eq!(shifted, raw, "нулевая сила эффект выключает");
        let shifted = cooked(one(FxKind::RgbShift, 0.01, 0.0), &raw, w, h);
        assert_eq!(px(&shifted, w, 3, 0), [20, 0, 4, 255]);
        // Пиксели: кадр из квадратов, внутри квадрата цвет один.
        let (w, h) = (40, 40);
        let raw = frame(w, h, |x, y| [(x * 6) as u8, (y * 6) as u8, 0, 255]);
        let blocks = cooked(one(FxKind::Pixelate, 1.0, 0.0), &raw, w, h);
        assert_eq!(px(&blocks, w, 0, 0), px(&blocks, w, 3, 3));
        assert_ne!(px(&blocks, w, 0, 0), px(&blocks, w, 4, 0));
    }

    #[test]
    fn vignette_darkens_corners_and_glow_brightens_around_light() {
        let (w, h) = (64, 36);
        let flat = frame(w, h, |_, _| [200, 200, 200, 255]);
        let out = cooked(one(FxKind::Vignette, 1.0, 0.5), &flat, w, h);
        assert_eq!(px(&out, w, w / 2, h / 2)[0], 200);
        assert!(px(&out, w, 0, 0)[0] < 40, "угол {}", px(&out, w, 0, 0)[0]);

        let (w, h) = (96, 80);
        let spot = frame(w, h, |x, y| if (44..52).contains(&x) && (36..44).contains(&y) { [255, 255, 255, 255] } else { [20, 20, 20, 255] });
        let out = cooked(one(FxKind::Glow, 1.0, 0.5), &spot, w, h);
        assert!(px(&out, w, 42, 40)[0] > 40, "рядом со светом {}", px(&out, w, 42, 40)[0]);
        assert_eq!(px(&out, w, 2, 2)[0], 20, "вдали от света ничего не меняется");
        assert_eq!(px(&out, w, 48, 40)[0], 255);
    }

    #[test]
    fn grain_differs_between_frames_and_stays_small() {
        let (w, h) = (32, 32);
        let flat = frame(w, h, |_, _| [128, 128, 128, 255]);
        let recipe = Recipe { vfx: one(FxKind::Grain, 0.4, 0.0), ..Recipe::default() };
        let (a, b) = (recipe.cook(&flat, w, h, 1, &mut Mattes::default()), recipe.cook(&flat, w, h, 2, &mut Mattes::default()));
        assert_ne!(a, b);
        assert_eq!(a, recipe.cook(&flat, w, h, 1, &mut Mattes::default()), "кадр с тем же номером повторяется точно");
        assert!(a.chunks_exact(4).all(|p| p[0].abs_diff(128) <= 13 && p[0] == p[1] && p[1] == p[2]));
        assert!(a.chunks_exact(4).any(|p| p[0] != 128));
    }

    #[test]
    fn mask_and_crop_cut_the_frame() {
        let (w, h) = (100, 60);
        let raw = frame(w, h, |_, _| [200, 150, 100, 255]);
        let cropped = cooked(Vfx { crop: [0.25, 0.0, 0.0, 0.5], ..Vfx::default() }, &raw, w, h);
        assert_eq!(px(&cropped, w, 10, 10), [0, 0, 0, 0]);
        assert_eq!(px(&cropped, w, 60, 10), [200, 150, 100, 255]);
        assert_eq!(px(&cropped, w, 60, 40), [0, 0, 0, 0]);

        let round = Mask { feather: 0.0, ..Mask::new(MaskShape::Ellipse) };
        let masked = cooked(Vfx { mask: Some(round.clone()), ..Vfx::default() }, &raw, w, h);
        assert_eq!(px(&masked, w, 50, 30), [200, 150, 100, 255]);
        assert_eq!(px(&masked, w, 22, 14)[3], 0, "угол рамки фигуры лежит вне эллипса");
        let boxed = cooked(Vfx { mask: Some(Mask { shape: MaskShape::Rect, ..round.clone() }), ..Vfx::default() }, &raw, w, h);
        assert_eq!(px(&boxed, w, 22, 14)[3], 255);
        let inverted = cooked(Vfx { mask: Some(Mask { invert: true, ..round }), ..Vfx::default() }, &raw, w, h);
        assert_eq!((px(&inverted, w, 50, 30)[3], px(&inverted, w, 2, 2)[3]), (0, 255));
        // Мягкий край: прозрачность меняется постепенно.
        let soft = cooked(Vfx { mask: Some(Mask { feather: 0.5, ..Mask::new(MaskShape::Rect) }), ..Vfx::default() }, &raw, w, h);
        let edge = px(&soft, w, 20, 30)[3];
        assert!((100..160).contains(&edge), "на краю фигуры {edge}");
    }

    #[test]
    fn blend_modes_set_transparency_from_brightness() {
        let raw = [[200, 100, 0, 255], [200, 100, 0, 128], [0, 0, 0, 255]].concat();
        let add = cooked(Vfx { blend: Blend::Add, ..Vfx::default() }, &raw, 3, 1);
        assert_eq!(add, [[200, 100, 0, 0], [100, 50, 0, 0], [0, 0, 0, 0]].concat());
        let screen = cooked(Vfx { blend: Blend::Screen, ..Vfx::default() }, &raw, 3, 1);
        assert_eq!(screen, [[200, 100, 0, 200], [100, 50, 0, 100], [0, 0, 0, 0]].concat());
        // Обычное наложение: цвет умножен на непрозрачность, сама она на месте.
        let normal = Recipe { look: Look { key: Some(ChromaKey::default()), ..Look::default() }, ..Recipe::default() };
        let keyed = normal.cook(&[[0, 255, 0, 255], [200, 60, 50, 255]].concat(), 2, 1, 0, &mut Mattes::default());
        assert_eq!(keyed, [[0, 0, 0, 0], [200, 60, 50, 255]].concat());
    }

    #[test]
    fn matte_view_shows_transparency_in_grey() {
        let recipe = Recipe { look: Look { key: Some(ChromaKey::default()), ..Look::default() }, matte: true, ..Recipe::default() };
        let out = recipe.cook(&[[0, 255, 0, 255], [200, 60, 50, 255]].concat(), 2, 1, 0, &mut Mattes::default());
        assert_eq!(out, [[0, 0, 0, 255], [255, 255, 255, 255]].concat());
    }

    #[test]
    fn choke_eats_the_edge_of_the_visible_part() {
        let (w, h) = (9, 9);
        let mut buf = frame(w, h, |x, y| [9, 9, 9, if (2..7).contains(&x) && (2..7).contains(&y) { 255 } else { 0 }]);
        choke(&mut buf, w, h, 1);
        assert_eq!((px(&buf, w, 2, 4)[3], px(&buf, w, 3, 4)[3], px(&buf, w, 4, 4)[3]), (0, 255, 255));
        choke(&mut buf, w, h, 2);
        assert!(buf.chunks_exact(4).all(|p| p[3] == 0 && p[0] == 9));
    }

    #[test]
    fn mattes_are_computed_once_per_setting() {
        let mut mattes = Mattes::default();
        let fx = Fx::new(FxKind::Vignette);
        let (a, b) = (mattes.shade(&fx, 32, 18).unwrap(), mattes.shade(&fx, 32, 18).unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        let other = mattes.shade(&Fx { amount: 0.9, ..fx.clone() }, 32, 18).unwrap();
        assert!(!Arc::ptr_eq(&a, &other));
        // Запоминается ограниченное число картинок.
        for i in 0..20 {
            mattes.shade(&fx, 32 + i, 18);
        }
        assert_eq!(mattes.0.len(), MATTES);
    }

    #[test]
    fn effects_are_written_as_ffmpeg_filters() {
        let stack = vec![
            Fx::with(FxKind::Blur, 0.5, 0.0),
            Fx::with(FxKind::RgbShift, 0.5, 0.0),
            Fx::with(FxKind::Invert, 1.0, 0.0),
            Fx::with(FxKind::Pixelate, 0.5, 0.0),
            Fx { on: false, ..Fx::new(FxKind::Glow) },
        ];
        let f = filters(&Look::default(), &Vfx { stack, ..Vfx::default() }, 1920, 1080, "x0_", 3);
        assert!(f.rasters.is_empty());
        assert_eq!(
            f.chain,
            ",gblur=sigma=21.60,rgbashift=rh=19:bh=-19,lutrgb=r='val+(255-2*val)*1.0000':g='val+(255-2*val)*1.0000':b='val+(255-2*val)*1.0000',\
             scale=36:20:flags=area,scale=1920:1080:flags=neighbor"
        );
        assert!(filters(&Look::default(), &Vfx::default(), 1920, 1080, "x0_", 3).chain.is_empty());

        // Зеркало и свечение раздваивают поток: появляются промежуточные метки.
        let f = filters(&Look::default(), &one(FxKind::Mirror, 1.0, 1.0), 640, 360, "x1_", 0);
        assert_eq!(f.chain, ",split[x1_1][x1_2];[x1_1]crop=iw/2:ih:iw/2:0,hflip[x1_3];[x1_2]crop=iw/2:ih:iw/2:0[x1_4];[x1_3][x1_4]hstack");
        let f = filters(&Look::default(), &one(FxKind::Glow, 1.0, 0.6), 640, 360, "x1_", 0);
        assert!(f.chain.starts_with(",format=gbrap,split[x1_1][x1_2];[x1_2]lutrgb=r='clip((val-153)*2.5000,0,255)':"), "{}", f.chain);
        assert!(f.chain.ends_with(",gblur=sigma=9.00[x1_3];[x1_1][x1_3]blend=c0_mode=screen:c1_mode=screen:c2_mode=screen,format=rgba"), "{}", f.chain);
    }

    #[test]
    fn masks_and_shades_become_extra_inputs() {
        let vfx = Vfx { crop: [0.5, 0.0, 0.0, 0.0], stack: vec![Fx::new(FxKind::Vignette)], ..Vfx::default() };
        let f = filters(&Look::default(), &vfx, 64, 36, "x2_", 5);
        assert_eq!(f.rasters.len(), 2);
        assert!(f.rasters.iter().all(|r| (r.w, r.h, r.rgba.len()) == (64, 36, 64 * 36 * 4)));
        // Сначала виньетка затемняет цвет, потом маска режет прозрачность.
        assert_eq!(
            f.chain,
            ",format=gbrap[x2_1];[5:v]format=gbrap,loop=loop=-1:size=1:start=0[x2_2];\
             [x2_1][x2_2]blend=c0_mode=multiply:c1_mode=multiply:c2_mode=multiply:shortest=1,format=rgba\
             ,format=gbrap[x2_3];[6:v]format=gbrap,loop=loop=-1:size=1:start=0[x2_4];[x2_3][x2_4]blend=c3_mode=multiply:shortest=1,format=rgba"
        );
        assert_eq!(f.rasters[1].rgba[..4], [255, 255, 255, 0]);
        assert_eq!(f.rasters[1].rgba[63 * 4..][..4], [255, 255, 255, 255]);
        assert_eq!(f.rasters[0].rgba[(18 * 64 + 32) * 4..][..4], [255, 255, 255, 255]);
    }

    #[test]
    fn blend_modes_prepare_the_stream_for_overlay() {
        assert_eq!(blend_filters(Blend::Normal, "m0"), (String::new(), ""));
        let (prep, option) = blend_filters(Blend::Add, "m0");
        assert_eq!((prep.as_str(), option), (",format=gbrap,premultiply=inplace=1,colorchannelmixer=aa=0", ":alpha=premultiplied"));
        let (prep, option) = blend_filters(Blend::Screen, "m0");
        assert!(prep.contains("extractplanes=r+g+b[m0r][m0g][m0b]") && prep.ends_with("[m0c][m0x]alphamerge,unpremultiply=inplace=1"), "{prep}");
        assert_eq!(option, "");
    }

    #[test]
    fn effects_are_saved_only_when_set() {
        assert_eq!(serde_json::to_string(&Vfx::default()).unwrap(), "{}");
        let vfx = Vfx {
            blend: Blend::Screen,
            crop: [0.1, 0.0, 0.0, 0.0],
            mask: Some(Mask::new(MaskShape::Ellipse)),
            stack: VfxPreset::Vhs.stack(),
        };
        let json = serde_json::to_string(&vfx).unwrap();
        assert_eq!(serde_json::from_str::<Vfx>(&json).unwrap(), vfx);
        // Каждый готовый набор что-то делает с картинкой.
        for preset in VfxPreset::ALL {
            let stack = preset.stack();
            assert!(!stack.is_empty() && stack.iter().all(Fx::active), "{preset:?}");
        }
        for kind in FxKind::ALL {
            assert!(Fx::new(kind).active(), "{kind:?}: новый эффект сразу виден");
        }
    }

    /// Те же кадры через настоящий ffmpeg: предпросмотр и экспорт должны дать одну картинку.
    /// Запуск: `cargo test -- --ignored preview_matches_ffmpeg`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn preview_matches_ffmpeg() {
        use std::io::Write;
        use std::process::Stdio;

        let (w, h) = (96usize, 64usize);
        let raw = frame(w, h, |x, y| {
            let green = x < 30;
            if green {
                [(x * 2) as u8, 220, (y * 2) as u8, 255]
            } else {
                [(x * 2 + y) as u8, (255 - y * 3) as u8, (x * y % 256) as u8, 255]
            }
        });
        let dir = std::env::temp_dir().join(format!("lumicut-vfx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Прогоняет кадр через цепочку фильтров; картинки-маски подаются входами после кадра.
        let run = |f: &Filters, tail: &str| -> Vec<u8> {
            let mut cmd = crate::ffmpeg::ffmpeg();
            cmd.args(["-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", "30", "-i", "-"]);
            for (i, raster) in f.rasters.iter().enumerate() {
                let path = dir.join(format!("matte{i}.png"));
                let mut png = crate::ffmpeg::ffmpeg()
                    .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{}x{}", raster.w, raster.h)])
                    .args(["-i", "-", "-frames:v", "1"])
                    .arg(&path)
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                png.stdin.take().unwrap().write_all(&raster.rgba).unwrap();
                assert!(png.wait().unwrap().success());
                cmd.arg("-i").arg(path);
            }
            let graph = format!("[0:v]format=rgba{}{tail}", f.chain);
            cmd.args(["-filter_complex", &graph, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
            let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&raw).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.stdout.len(), raw.len(), "{graph}");
            out.stdout
        };
        // Наибольшее и среднее расхождение цвета и прозрачности.
        let diff = |a: &[u8], b: &[u8]| {
            let worst = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            let mean = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64;
            (worst, mean)
        };
        // Кадр предпросмотра с цветом, не умноженным на непрозрачность, как его отдаёт ffmpeg.
        let straight = |recipe: &Recipe| {
            let mut out = raw.clone();
            recipe.look.apply_key(&mut out, w);
            if let Some(key) = &recipe.look.key {
                choke(&mut out, w, h, key.choke_passes(h as u32));
            }
            recipe.look.apply_grade(&mut out, w);
            let mut mattes = Mattes::default();
            for fx in recipe.vfx.stack.iter().filter(|fx| fx.active()) {
                apply(fx, &mut out, w, h, 0, &mut mattes);
            }
            if let Some(matte) = mattes.shape(&recipe.vfx, w, h) {
                cut(&mut out, w, &matte);
            }
            out
        };

        // Хромакей с подавлением отсвета и срезанным краем совпадает точка в точку.
        let key = ChromaKey { color: [40, 220, 60], similarity: 0.25, blend: 0.1, spill: 0.8, choke: 1.0 };
        let recipe = Recipe { look: Look { key: Some(key), ..Look::default() }, ..Recipe::default() };
        let f = filters(&recipe.look, &recipe.vfx, w as u32, h as u32, "t", 1);
        assert!(f.chain.contains("despill") && f.chain.contains("erosion"), "{}", f.chain);
        assert_eq!(diff(&straight(&recipe), &run(&f, "")).0, 0, "хромакей");

        // Эффекты: точные — точка в точку, размытие и шум — с допуском.
        for (kind, amount, extra, worst, mean) in [
            (FxKind::Invert, 0.7, 0.0, 0, 0.0),
            (FxKind::Posterize, 0.6, 0.0, 0, 0.0),
            (FxKind::RgbShift, 0.5, 0.0, 0, 0.0),
            (FxKind::Mirror, 1.0, 0.0, 0, 0.0),
            (FxKind::Mirror, 1.0, 3.0, 0, 0.0),
            (FxKind::Hue, 0.4, 0.0, 2, 1.0),
            (FxKind::Vignette, 0.8, 0.4, 1, 0.5),
            (FxKind::Scanlines, 0.5, 0.5, 1, 0.5),
            (FxKind::Sharpen, 0.5, 0.0, 255, 2.0),
            (FxKind::Pixelate, 0.5, 0.0, 255, 6.0),
            (FxKind::Blur, 0.6, 0.0, 40, 4.0),
            (FxKind::Glow, 0.8, 0.5, 40, 4.0),
            (FxKind::Grain, 0.4, 0.0, 30, 8.0),
        ] {
            let recipe = Recipe { vfx: one(kind, amount, extra), ..Recipe::default() };
            let f = filters(&recipe.look, &recipe.vfx, w as u32, h as u32, "t", 1);
            let (got_worst, got_mean) = diff(&straight(&recipe), &run(&f, ""));
            println!("{kind:?}: наибольшее расхождение {got_worst}, среднее {got_mean:.2}");
            assert!(got_worst <= worst && got_mean <= mean, "{kind:?}: {got_worst} и {got_mean:.2}");
        }

        // Маска с кадрированием режет прозрачность так же.
        let vfx = Vfx { crop: [0.1, 0.2, 0.0, 0.0], mask: Some(Mask::new(MaskShape::Ellipse)), ..Vfx::default() };
        let recipe = Recipe { vfx, ..Recipe::default() };
        let f = filters(&recipe.look, &recipe.vfx, w as u32, h as u32, "t", 1);
        assert_eq!(f.rasters.len(), 1);
        assert_eq!(diff(&straight(&recipe), &run(&f, "")).0, 0, "маска");

        // Режимы наложения: поток готовится без ошибок, чёрное становится прозрачным.
        let none = Filters { chain: String::new(), rasters: Vec::new() };
        let (prep, _) = blend_filters(Blend::Screen, "s");
        let screen = run(&none, &prep);
        let mut mine = raw.clone();
        premultiply(&mut mine, w, Blend::Screen);
        let alpha = |buf: &[u8]| buf.chunks_exact(4).map(|p| p[3]).collect::<Vec<u8>>();
        assert!(diff(&alpha(&mine), &alpha(&screen)).0 <= 1, "прозрачность в режиме «Экран»");
        let (prep, _) = blend_filters(Blend::Add, "s");
        assert!(alpha(&run(&none, &prep)).iter().all(|a| *a == 0), "при сложении клип ничего не закрывает");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
