//! Текстовые клипы: надпись рисуется в картинку, которая идёт и в предпросмотр, и в экспорт.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui::epaint::text::{FontData, FontDefinitions, Fonts, LayoutJob};
use eframe::egui::{Align, Color32, FontFamily, FontId};
use serde::{Deserialize, Serialize};

pub const MIN_SIZE: f32 = 12.0;
pub const MAX_SIZE: f32 = 400.0;
/// Наибольшая толщина контура в долях высоты шрифта.
pub const MAX_OUTLINE: f32 = 0.12;
/// Длительность нового текстового клипа и нового субтитра, секунды.
pub const DURATION: f64 = 3.0;

/// Шрифты, которыми можно набрать текст: название и файл в папке шрифтов системы.
pub const FONTS: [(&str, &str); 10] = [
    ("Segoe UI", "segoeui.ttf"),
    ("Segoe UI Semibold", "seguisb.ttf"),
    ("Segoe UI Bold", "segoeuib.ttf"),
    ("Arial", "arial.ttf"),
    ("Arial Bold", "arialbd.ttf"),
    ("Impact", "impact.ttf"),
    ("Georgia", "georgia.ttf"),
    ("Times New Roman", "times.ttf"),
    ("Comic Sans", "comic.ttf"),
    ("Consolas", "consola.ttf"),
];

/// Сторона атласа, в который шрифтовой движок складывает знаки.
const ATLAS_SIDE: usize = 2048;
/// Показатель, с которым покрытие знака переводится в непрозрачность: так же текст рисует интерфейс.
const COVERAGE_GAMMA: f32 = 0.55;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub content: String,
    /// Название шрифта из списка `FONTS`.
    pub font: String,
    /// Высота шрифта в пикселях проекта.
    pub size: f32,
    pub color: [u8; 3],
    pub align: TextAlign,
    /// Толщина контура в долях высоты шрифта; 0 — без контура.
    pub outline: f32,
    pub outline_color: [u8; 3],
    pub shadow: bool,
    /// Непрозрачность плашки под текстом; 0 — без плашки.
    pub backdrop: f32,
    pub backdrop_color: [u8; 3],
    /// Субтитр: такие клипы попадают в файл субтитров и разделяют общий стиль.
    #[serde(default)]
    pub caption: bool,
}

/// Готовое оформление текста из библиотеки.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextPreset {
    Title,
    Subtitle,
    Plain,
    Outline,
    Badge,
    Accent,
}

impl TextPreset {
    pub const ALL: [TextPreset; 6] = [
        TextPreset::Title,
        TextPreset::Subtitle,
        TextPreset::Plain,
        TextPreset::Outline,
        TextPreset::Badge,
        TextPreset::Accent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TextPreset::Title => "Заголовок",
            TextPreset::Subtitle => "Подзаголовок",
            TextPreset::Plain => "Обычный текст",
            TextPreset::Outline => "С контуром",
            TextPreset::Badge => "На плашке",
            TextPreset::Accent => "Акцент",
        }
    }

    /// Текст с этим оформлением для кадра высотой `frame_h`.
    pub fn text(self, frame_h: u32) -> Text {
        let h = frame_h as f32;
        let base = Text {
            content: self.label().to_owned(),
            font: "Segoe UI".to_owned(),
            size: (h * 0.06).round(),
            color: [255, 255, 255],
            align: TextAlign::Center,
            outline: 0.0,
            outline_color: [0, 0, 0],
            shadow: false,
            backdrop: 0.0,
            backdrop_color: [0, 0, 0],
            caption: false,
        };
        match self {
            TextPreset::Title => Text { font: "Segoe UI Bold".to_owned(), size: (h * 0.13).round(), shadow: true, ..base },
            TextPreset::Subtitle => Text { font: "Segoe UI Semibold".to_owned(), size: (h * 0.08).round(), shadow: true, ..base },
            TextPreset::Plain => base,
            TextPreset::Outline => Text { font: "Impact".to_owned(), size: (h * 0.11).round(), outline: 0.06, ..base },
            TextPreset::Badge => Text { font: "Segoe UI Semibold".to_owned(), backdrop: 0.7, ..base },
            TextPreset::Accent => Text {
                font: "Arial Bold".to_owned(),
                size: (h * 0.1).round(),
                color: [0xFF, 0xC8, 0x3D],
                outline: 0.05,
                shadow: true,
                ..base
            },
        }
    }
}

impl Text {
    /// Оформление субтитра для кадра высотой `frame_h`.
    pub fn caption(content: &str, frame_h: u32) -> Text {
        Text {
            content: content.to_owned(),
            font: "Segoe UI Semibold".to_owned(),
            size: (frame_h as f32 * 0.052).round(),
            outline: 0.07,
            caption: true,
            ..TextPreset::Plain.text(frame_h)
        }
    }

    /// Первая строка текста: подпись клипа на дорожке.
    pub fn title(&self) -> &str {
        self.content.lines().find(|l| !l.trim().is_empty()).map_or("Текст", str::trim)
    }

    /// Переносит оформление с другого текста; содержание остаётся своё.
    pub fn restyle(&mut self, from: &Text) {
        let content = std::mem::take(&mut self.content);
        *self = Text { content, ..from.clone() };
    }
}

/// Картинка с надписью: пиксели RGBA с обычной (не умноженной) прозрачностью.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

fn fonts() -> &'static Mutex<Fonts> {
    static FONT_SET: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONT_SET.get_or_init(|| {
        let dir = std::env::var("WINDIR")
            .map(|w| PathBuf::from(w).join("Fonts"))
            .unwrap_or_else(|_| PathBuf::from("C:\\Windows\\Fonts"));
        let mut defs = FontDefinitions::default();
        // Встроенные шрифты остаются запасными: в них есть знаки, которых нет в системных.
        let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        for (name, file) in FONTS {
            let mut list = fallback.clone();
            if let Ok(bytes) = std::fs::read(dir.join(file)) {
                defs.font_data.insert(name.to_owned(), FontData::from_owned(bytes));
                list.insert(0, name.to_owned());
            }
            defs.families.insert(FontFamily::Name(name.into()), list);
        }
        Mutex::new(Fonts::new(1.0, ATLAS_SIDE, defs))
    })
}

/// Расстояние от каждой точки до ближайшей закрашенной, в пикселях.
fn distance(solid: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1e12;
    // Одномерный проход: нижняя огибающая парабол, по столбцам и затем по строкам.
    fn pass(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
        let n = f.len();
        let (mut k, mut q) = (0usize, 1usize);
        v[0] = 0;
        z[0] = -FAR;
        z[1] = FAR;
        while q < n {
            let s = ((f[q] + (q * q) as f32) - (f[v[k]] + (v[k] * v[k]) as f32)) / (2.0 * (q as f32 - v[k] as f32));
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = FAR;
            q += 1;
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let d = q as f32 - v[k] as f32;
            *o = d * d + f[v[k]];
        }
    }
    let n = w.max(h);
    let (mut f, mut out, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    let mut d: Vec<f32> = solid.iter().map(|s| if *s { 0.0 } else { FAR }).collect();
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        pass(&f[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&d[y * w..(y + 1) * w]);
        pass(&f[..w], &mut out[..w], &mut v, &mut z);
        for x in 0..w {
            d[y * w + x] = out[x].sqrt();
        }
    }
    d
}

/// Размытие усреднением по квадрату со стороной `2r + 1`, два прохода подряд.
fn blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut a = src.to_vec();
    let mut b = vec![0.0; src.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    for _ in 0..2 {
        for y in 0..h {
            let row = &a[y * w..(y + 1) * w];
            let mut sum: f32 = row.iter().take(r + 1).sum();
            for x in 0..w {
                b[y * w + x] = sum * norm;
                if x + r + 1 < w {
                    sum += row[x + r + 1];
                }
                if x >= r {
                    sum -= row[x - r];
                }
            }
        }
        for x in 0..w {
            let mut sum: f32 = (0..=r.min(h - 1)).map(|y| b[y * w + x]).sum();
            for y in 0..h {
                a[y * w + x] = sum * norm;
                if y + r + 1 < h {
                    sum += b[(y + r + 1) * w + x];
                }
                if y >= r {
                    sum -= b[(y - r) * w + x];
                }
            }
        }
    }
    a
}

/// Рисует надпись. Размер картинки — размер надписи в пикселях проекта с полями под контур, тень и плашку.
pub fn render(text: &Text) -> Raster {
    let size = text.size.clamp(MIN_SIZE, MAX_SIZE);
    let outline = text.outline.clamp(0.0, MAX_OUTLINE) * size;
    let (shadow_drop, shadow_blur) = if text.shadow { (size * 0.05, (size * 0.04).max(1.0)) } else { (0.0, 0.0) };
    let backdrop = text.backdrop.clamp(0.0, 1.0);
    let (plate_x, plate_y) = if backdrop > 0.0 { (size * 0.38, size * 0.16) } else { (0.0, 0.0) };
    let margin = outline + shadow_drop + 2.0 * shadow_blur + 2.0;
    let (pad_x, pad_y) = ((margin.max(plate_x)).ceil() as i32, (margin.max(plate_y)).ceil() as i32);

    // Вёрстка и покрытие знаков.
    let family = if FONTS.iter().any(|f| f.0 == text.font) { text.font.as_str() } else { FONTS[0].0 };
    let font = FontId::new(size, FontFamily::Name(family.into()));
    let mut job = LayoutJob::simple(text.content.clone(), font, Color32::WHITE, f32::INFINITY);
    job.halign = match text.align {
        TextAlign::Left => Align::Min,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Max,
    };
    let fonts = fonts().lock().unwrap();
    fonts.begin_pass(1.0, ATLAS_SIDE);
    let galley = fonts.layout_job(job);
    let atlas = fonts.image();
    drop(fonts);

    // Знаки: место в картинке и место в атласе.
    let mut glyphs: Vec<([i32; 2], [usize; 2], [usize; 2])> = Vec::new();
    let (mut left, mut right) = (f32::MAX, f32::MIN);
    for row in &galley.rows {
        left = left.min(row.rect.left());
        right = right.max(row.rect.right());
        for g in &row.glyphs {
            let uv = g.uv_rect;
            if uv.is_nothing() {
                continue;
            }
            let at = g.pos + uv.offset;
            let (min, max) = ([uv.min[0] as usize, uv.min[1] as usize], [uv.max[0] as usize, uv.max[1] as usize]);
            glyphs.push(([at.x.round() as i32, at.y.round() as i32], min, [max[0] - min[0], max[1] - min[1]]));
        }
    }
    if left > right {
        (left, right) = (0.0, 0.0);
    }
    // Выступающие за строку знаки (курсив, хвосты букв) тоже должны поместиться.
    let (mut x0, mut x1) = (left.floor() as i32, right.ceil() as i32);
    let (mut y0, mut y1) = (0, galley.rect.height().ceil() as i32);
    for (at, _, span) in &glyphs {
        x0 = x0.min(at[0]);
        x1 = x1.max(at[0] + span[0] as i32);
        y0 = y0.min(at[1]);
        y1 = y1.max(at[1] + span[1] as i32);
    }
    let (w, h) = (((x1 - x0) + 2 * pad_x).max(2) as usize, ((y1 - y0) + 2 * pad_y).max(2) as usize);
    let (ox, oy) = (pad_x - x0, pad_y - y0);
    let mut cover = vec![0f32; w * h];
    for (at, min, span) in &glyphs {
        for gy in 0..span[1] {
            let y = (at[1] + oy) as usize + gy;
            let line = &atlas.pixels[(min[1] + gy) * atlas.size[0] + min[0]..][..span[0]];
            let dst = &mut cover[y * w + (at[0] + ox) as usize..][..span[0]];
            for (d, s) in dst.iter_mut().zip(line) {
                *d = d.max(s.powf(COVERAGE_GAMMA));
            }
        }
    }

    // Слои снизу вверх: плашка, тень, контур, заливка. Накапливается цвет, умноженный на непрозрачность.
    let mut rgb = vec![[0f32; 3]; w * h];
    let mut alpha = vec![0f32; w * h];
    let mut put = |i: usize, color: [u8; 3], a: f32| {
        if a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        for k in 0..3 {
            rgb[i][k] = color[k] as f32 * a + rgb[i][k] * (1.0 - a);
        }
        alpha[i] = a + alpha[i] * (1.0 - a);
    };
    if backdrop > 0.0 {
        // Плашка со скруглёнными углами вокруг строк текста.
        let half = [(x1 - x0) as f32 / 2.0 + plate_x, (y1 - y0) as f32 / 2.0 + plate_y];
        let center = [w as f32 / 2.0, h as f32 / 2.0];
        let round = (size * 0.22).min(half[0]).min(half[1]);
        for y in 0..h {
            for x in 0..w {
                let q = [
                    (x as f32 + 0.5 - center[0]).abs() - (half[0] - round),
                    (y as f32 + 0.5 - center[1]).abs() - (half[1] - round),
                ];
                let outside = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt() + q[0].max(q[1]).min(0.0) - round;
                put(y * w + x, text.backdrop_color, backdrop * (0.5 - outside).clamp(0.0, 1.0));
            }
        }
    }
    if text.shadow {
        let soft = blur(&cover, w, h, shadow_blur.round() as usize);
        let drop = shadow_drop.round() as usize;
        for y in drop..h {
            for x in 0..w {
                put(y * w + x, [0, 0, 0], 0.6 * soft[(y - drop) * w + x]);
            }
        }
    }
    if outline > 0.0 {
        let solid: Vec<bool> = cover.iter().map(|c| *c >= 0.5).collect();
        let far = distance(&solid, w, h);
        for (i, d) in far.iter().enumerate() {
            put(i, text.outline_color, outline + 0.5 - d);
        }
    }
    for (i, c) in cover.iter().enumerate() {
        put(i, text.color, *c);
    }

    let mut rgba = Vec::with_capacity(w * h * 4);
    for (c, a) in rgb.iter().zip(&alpha) {
        let k = if *a > 0.0 { 1.0 / a } else { 0.0 };
        rgba.extend(c.map(|v| (v * k + 0.5).clamp(0.0, 255.0) as u8));
        rgba.push((a * 255.0 + 0.5) as u8);
    }
    Raster { w: w as u32, h: h as u32, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(content: &str) -> Text {
        Text { content: content.to_owned(), size: 60.0, ..TextPreset::Plain.text(1080) }
    }

    /// Доля непрозрачных точек картинки.
    fn filled(r: &Raster) -> f32 {
        r.rgba.chunks_exact(4).filter(|p| p[3] > 128).count() as f32 / (r.w * r.h) as f32
    }

    #[test]
    fn text_is_drawn_and_longer_text_is_wider() {
        let (short, long) = (render(&plain("Привет")), render(&plain("Привет, мир")));
        assert_eq!(short.rgba.len(), (short.w * short.h * 4) as usize);
        assert!(short.h >= 60 && short.h < 110, "высота {}", short.h);
        assert!(long.w > short.w + 60, "{} и {}", short.w, long.w);
        // Буквы белые и занимают заметную часть картинки.
        assert!(filled(&short) > 0.08, "закрашено {}", filled(&short));
        assert!(short.rgba.chunks_exact(4).filter(|p| p[3] == 255).all(|p| p[..3] == [255, 255, 255]));
        // Второй раз получается то же самое.
        assert_eq!(render(&plain("Привет")), short);
    }

    #[test]
    fn lines_stack_and_size_scales_the_picture() {
        let (one, two) = (render(&plain("Раз")), render(&plain("Раз\nДва")));
        assert!(two.h > one.h + 50, "{} и {}", one.h, two.h);
        let big = render(&Text { size: 120.0, ..plain("Раз") });
        assert!((big.h as f32 / one.h as f32 - 2.0).abs() < 0.2, "{} и {}", one.h, big.h);
    }

    #[test]
    fn outline_shadow_and_backdrop_add_their_layers() {
        let base = render(&plain("Текст"));
        let outlined = render(&Text { outline: 0.08, outline_color: [255, 0, 0], ..plain("Текст") });
        assert!(outlined.w > base.w && filled(&outlined) > filled(&base));
        assert!(outlined.rgba.chunks_exact(4).any(|p| p[3] == 255 && p[..3] == [255, 0, 0]), "контур красный");

        let plate = render(&Text { backdrop: 1.0, backdrop_color: [0, 0, 255], ..plain("Текст") });
        assert!(filled(&plate) > 0.9, "плашка закрывает картинку: {}", filled(&plate));
        // Углы плашки скруглены.
        assert_eq!(plate.rgba[3], 0);

        let shadow = render(&Text { shadow: true, ..plain("Текст") });
        assert!(shadow.rgba.chunks_exact(4).any(|p| p[3] > 60 && p[3] < 200 && p[0] < 40), "тень тёмная и полупрозрачная");
    }

    #[test]
    fn empty_text_is_a_tiny_transparent_picture() {
        let r = render(&plain(""));
        assert!(r.w >= 2 && r.h >= 2);
        assert!(r.rgba.chunks_exact(4).all(|p| p[3] == 0));
    }

    #[test]
    fn caption_style_is_shared_but_content_stays() {
        let mut a = Text::caption("Первая строка", 1080);
        let styled = Text { color: [255, 255, 0], size: 80.0, ..Text::caption("Образец", 1080) };
        a.restyle(&styled);
        assert_eq!((a.content.as_str(), a.color, a.size, a.caption), ("Первая строка", [255, 255, 0], 80.0, true));
        assert_eq!(a.title(), "Первая строка");
    }
}
