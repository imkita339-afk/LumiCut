//! Цвет клипа: готовые фильтры, ручная коррекция и хромакей. Одни и те же числа — таблица яркости
//! и матрица цвета — применяются к кадрам предпросмотра и уходят в фильтры ffmpeg при экспорте.

use serde::{Deserialize, Serialize};

use crate::vfx::bands;

/// Готовый фильтр из библиотеки.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum FilterKind {
    #[default]
    None,
    Vivid,
    Warm,
    Cool,
    Cinema,
    Vintage,
    Faded,
    Drama,
    Sepia,
    Mono,
    Noir,
}

/// Слагаемые цветокоррекции: каждое — отклонение от исходной картинки.
#[derive(Clone, Copy, Default)]
struct Grade {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    warmth: f32,
    /// Подъём чёрного: тени становятся серыми, как на выцветшей плёнке.
    lift: f32,
    /// Доля тонирования в сепию.
    sepia: f32,
}

impl FilterKind {
    pub const ALL: [FilterKind; 11] = [
        FilterKind::None,
        FilterKind::Vivid,
        FilterKind::Warm,
        FilterKind::Cool,
        FilterKind::Cinema,
        FilterKind::Vintage,
        FilterKind::Faded,
        FilterKind::Drama,
        FilterKind::Sepia,
        FilterKind::Mono,
        FilterKind::Noir,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterKind::None => "Без фильтра",
            FilterKind::Vivid => "Сочный",
            FilterKind::Warm => "Тёплый",
            FilterKind::Cool => "Холодный",
            FilterKind::Cinema => "Кино",
            FilterKind::Vintage => "Винтаж",
            FilterKind::Faded => "Выцветший",
            FilterKind::Drama => "Драма",
            FilterKind::Sepia => "Сепия",
            FilterKind::Mono => "Ч/Б",
            FilterKind::Noir => "Нуар",
        }
    }

    fn grade(self) -> Grade {
        let g = Grade::default();
        match self {
            FilterKind::None => g,
            FilterKind::Vivid => Grade { saturation: 0.4, contrast: 0.12, ..g },
            FilterKind::Warm => Grade { warmth: 0.45, saturation: 0.08, ..g },
            FilterKind::Cool => Grade { warmth: -0.45, ..g },
            FilterKind::Cinema => Grade { contrast: 0.22, saturation: -0.15, warmth: 0.15, lift: 0.03, ..g },
            FilterKind::Vintage => Grade { saturation: -0.3, warmth: 0.3, lift: 0.09, contrast: -0.08, ..g },
            FilterKind::Faded => Grade { saturation: -0.2, lift: 0.14, contrast: -0.15, ..g },
            FilterKind::Drama => Grade { contrast: 0.4, saturation: -0.25, brightness: -0.03, ..g },
            FilterKind::Sepia => Grade { sepia: 1.0, contrast: 0.05, ..g },
            FilterKind::Mono => Grade { saturation: -1.0, ..g },
            FilterKind::Noir => Grade { saturation: -1.0, contrast: 0.35, brightness: -0.04, ..g },
        }
    }
}

/// Цвет фона, который становится прозрачным.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChromaKey {
    pub color: [u8; 3],
    /// Насколько цвет точки может отличаться от заданного, 0..1.
    pub similarity: f32,
    /// Ширина плавного края, 0..1.
    pub blend: f32,
    /// Насколько гасится отсвет фона на самом предмете, 0..1.
    #[serde(default)]
    pub spill: f32,
    /// Насколько край предмета срезается внутрь, 0..1.
    #[serde(default)]
    pub choke: f32,
}

impl Default for ChromaKey {
    fn default() -> Self {
        Self { color: [0, 255, 0], similarity: 0.3, blend: 0.1, spill: 0.0, choke: 0.0 }
    }
}

/// Какую долю высоты кадра срезает с края наибольшее сжатие.
const CHOKE_REACH: f32 = 0.008;

impl ChromaKey {
    /// Непрозрачность точки: та же формула, что у фильтра `colorkey` в ffmpeg.
    fn alpha(&self, r: u8, g: u8, b: u8) -> u8 {
        let d = |a: u8, k: u8| (a as f32 - k as f32).powi(2);
        let diff = ((d(r, self.color[0]) + d(g, self.color[1]) + d(b, self.color[2])) / (255.0 * 255.0 * 3.0)).sqrt();
        if self.blend > 0.0001 {
            (((diff - self.similarity) / self.blend).clamp(0.0, 1.0) * 255.0) as u8
        } else if diff > self.similarity {
            255
        } else {
            0
        }
    }

    /// Хромакей для съёмки на зелёном или синем фоне с подавлением отсвета.
    pub fn studio(color: [u8; 3]) -> Self {
        Self { color, spill: 0.5, ..Self::default() }
    }

    /// Сколько раз край срезается на одну точку в кадре высотой `h`.
    pub fn choke_passes(&self, h: u32) -> u32 {
        (self.choke.clamp(0.0, 1.0) * CHOKE_REACH * h as f32).round() as u32
    }

    /// Чей отсвет гасится (синий вместо зелёного) и с какой силой; у фона другого цвета отсвет не гасится.
    fn despill(&self) -> Option<(bool, f32)> {
        let [r, g, b] = self.color;
        let strength = rounded(self.spill.clamp(0.0, 1.0));
        if strength <= 0.0 {
            None
        } else if g > r && g >= b {
            Some((false, strength))
        } else if b > r && b > g {
            Some((true, strength))
        } else {
            None
        }
    }

    /// Убирает с точки отсвет фона: те же действия и в том же порядке, что у фильтра `despill` в ffmpeg.
    fn clean(p: &mut [u8], blue: bool, strength: f32) {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let spill = if blue { (b - (r * 0.5 + g * 0.5)).max(0.0) } else { (g - (r * 0.5 + b * 0.5)).max(0.0) };
        let (gs, bs) = if blue { (0.0, -strength) } else { (-strength, 0.0) };
        let out = [(r + spill * 0.0).max(0.0), (g + spill * gs).max(0.0), (b + spill * bs).max(0.0)];
        for (c, v) in p.iter_mut().zip(out) {
            *c = ((v * 255.0) as i32).clamp(0, 255) as u8;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Look {
    #[serde(default)]
    pub filter: FilterKind,
    /// Сила готового фильтра, 0..1.
    pub strength: f32,
    /// Ручная коррекция, каждое значение −1..1.
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<ChromaKey>,
}

impl Default for Look {
    fn default() -> Self {
        Self { filter: FilterKind::None, strength: 1.0, brightness: 0.0, contrast: 0.0, saturation: 0.0, warmth: 0.0, key: None }
    }
}

/// Доли красного, зелёного и синего в яркости.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
const SEPIA: [[f32; 3]; 3] = [[0.393, 0.769, 0.189], [0.349, 0.686, 0.168], [0.272, 0.534, 0.131]];
/// Сколько знаков после запятой уходит в ffmpeg; предпросмотр считает с теми же округлёнными числами.
const DIGITS: f32 = 10_000.0;

fn rounded(x: f32) -> f32 {
    (x * DIGITS).round() / DIGITS
}

impl Look {
    /// Картинка остаётся как есть.
    pub fn is_neutral(&self) -> bool {
        self.key.is_none() && !self.graded()
    }

    /// Меняется ли цвет (а не только прозрачность).
    fn graded(&self) -> bool {
        (self.filter != FilterKind::None && self.strength > 0.0)
            || [self.brightness, self.contrast, self.saturation, self.warmth].iter().any(|v| v.abs() > 1e-4)
    }

    /// Готовый фильтр с его силой плюс ручная коррекция.
    fn grade(&self) -> Grade {
        let (f, k) = (self.filter.grade(), self.strength.clamp(0.0, 1.0));
        Grade {
            brightness: f.brightness * k + self.brightness * 0.5,
            contrast: f.contrast * k + self.contrast,
            saturation: (f.saturation * k + self.saturation).max(-1.0),
            warmth: f.warmth * k + self.warmth,
            lift: f.lift * k,
            sepia: f.sepia * k,
        }
    }

    /// Числа таблицы яркости: множитель контраста, сдвиг и подъём чёрного.
    fn curve_terms(&self) -> [f32; 3] {
        let g = self.grade();
        [rounded(1.0 + g.contrast), rounded(g.brightness), rounded(g.lift)]
    }

    /// Таблица, по которой меняется яркость каждого из трёх цветов.
    fn curve(&self) -> [u8; 256] {
        let [c, b, l] = self.curve_terms();
        std::array::from_fn(|i| {
            let x = i as f32 / 255.0;
            let y = (((x - 0.5) * c + 0.5 + b) * (1.0 - l) + l).clamp(0.0, 1.0);
            (y * 255.0 + 0.5) as u8
        })
    }

    /// Матрица цвета: строки — красный, зелёный и синий на выходе.
    fn matrix(&self) -> [[f32; 3]; 3] {
        let g = self.grade();
        let s = 1.0 + g.saturation;
        let gain = [1.0 + 0.22 * g.warmth, 1.0, 1.0 - 0.22 * g.warmth];
        let mut m = [[0f32; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                // Насыщенность: от серого той же яркости к исходному цвету и дальше.
                let sat = LUMA[j] * (1.0 - s) + if i == j { s } else { 0.0 };
                *cell = rounded((sat * (1.0 - g.sepia) + SEPIA[i][j] * g.sepia) * gain[i]);
            }
        }
        m
    }

    /// Применяет хромакей и цвет к кадру RGBA.
    pub fn apply(&self, rgba: &mut [u8]) {
        let w = rgba.len() / 4;
        self.apply_key(rgba, w);
        self.apply_grade(rgba, w);
    }

    /// Хромакей: прозрачность по цвету фона и подавление его отсвета. `w` — ширина кадра в точках.
    pub fn apply_key(&self, rgba: &mut [u8], w: usize) {
        let Some(key) = &self.key else { return };
        let despill = key.despill();
        bands(rgba, w, |_, rows| {
            for p in rows.chunks_exact_mut(4) {
                p[3] = key.alpha(p[0], p[1], p[2]);
                if let Some((blue, strength)) = despill {
                    ChromaKey::clean(p, blue, strength);
                }
            }
        });
    }

    /// Фильтр и ручная коррекция цвета. `w` — ширина кадра в точках.
    pub fn apply_grade(&self, rgba: &mut [u8], w: usize) {
        if !self.graded() {
            return;
        }
        let curve = self.curve();
        // Матрица в целых числах: 12 знаков после двоичной запятой.
        let m = self.matrix().map(|row| row.map(|v| (v * 4096.0).round() as i32));
        bands(rgba, w, |_, rows| {
            for p in rows.chunks_exact_mut(4) {
                let c = [curve[p[0] as usize] as i32, curve[p[1] as usize] as i32, curve[p[2] as usize] as i32];
                for (out, row) in p.iter_mut().zip(&m) {
                    *out = ((row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + 2048) >> 12).clamp(0, 255) as u8;
                }
            }
        });
    }

    /// Те же действия как фильтры ffmpeg для кадров RGBA высотой `h`; пустая строка, если картинка не меняется.
    pub fn filters(&self, h: u32) -> String {
        let mut out = String::new();
        if let Some(key) = &self.key {
            let [r, g, b] = key.color;
            out += &format!(",colorkey=0x{r:02X}{g:02X}{b:02X}:{:.4}:{:.4}", key.similarity, key.blend);
            if let Some((blue, strength)) = key.despill() {
                let scales = if blue { format!("green=0:blue=-{strength:.4}") } else { format!("green=-{strength:.4}:blue=0") };
                out += &format!(",despill=type={}:mix=0.5:expand=0:{scales}", if blue { "blue" } else { "green" });
            }
            // Сжатие края трогает только прозрачность: это четвёртая плоскость кадра.
            let passes = key.choke_passes(h);
            if passes > 0 {
                out += ",format=gbrap";
                out += &",erosion=threshold0=0:threshold1=0:threshold2=0".repeat(passes as usize);
                out += ",format=rgba";
            }
        }
        if self.graded() {
            let [c, b, l] = self.curve_terms();
            let curve = format!("clip(((val/255-0.5)*{c:.4}+0.5{b:+.4})*{:.4}{l:+.4},0,1)*255+0.5", 1.0 - l);
            out += &format!(",lutrgb=r='{curve}':g='{curve}':b='{curve}'");
            let m = self.matrix();
            out += &format!(
                ",colorchannelmixer=rr={:.4}:rg={:.4}:rb={:.4}:gr={:.4}:gg={:.4}:gb={:.4}:br={:.4}:bg={:.4}:bb={:.4}",
                m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(look: &Look, rgb: [u8; 3]) -> [u8; 4] {
        let mut p = [rgb[0], rgb[1], rgb[2], 255];
        look.apply(&mut p);
        p
    }

    #[test]
    fn neutral_look_changes_nothing() {
        let look = Look::default();
        assert!(look.is_neutral() && look.filters(1080).is_empty());
        assert_eq!(pixel(&look, [12, 130, 240]), [12, 130, 240, 255]);
        // Фильтр с нулевой силой тоже ничего не делает.
        assert!(Look { filter: FilterKind::Noir, strength: 0.0, ..Look::default() }.is_neutral());
    }

    #[test]
    fn mono_removes_colour_and_keeps_grey() {
        let look = Look { filter: FilterKind::Mono, ..Look::default() };
        let p = pixel(&look, [200, 40, 90]);
        assert!(p[0] == p[1] && p[1] == p[2], "{p:?}");
        assert_eq!(pixel(&look, [128, 128, 128]), [128, 128, 128, 255]);
    }

    #[test]
    fn manual_correction_moves_values_the_expected_way() {
        let grey = [100, 100, 100];
        assert!(pixel(&Look { brightness: 0.4, ..Look::default() }, grey)[0] > 140);
        assert!(pixel(&Look { contrast: 0.5, ..Look::default() }, grey)[0] < 95);
        assert!(pixel(&Look { contrast: 0.5, ..Look::default() }, [200, 200, 200])[0] > 215);
        let warm = pixel(&Look { warmth: 0.8, ..Look::default() }, grey);
        assert!(warm[0] > 110 && warm[2] < 90 && warm[1] == 100, "{warm:?}");
        let vivid = pixel(&Look { saturation: 0.8, ..Look::default() }, [150, 100, 100]);
        assert!(vivid[0] > 170 && vivid[1] < 95, "{vivid:?}");
    }

    #[test]
    fn chroma_key_makes_the_background_transparent() {
        let look = Look { key: Some(ChromaKey::default()), ..Look::default() };
        assert_eq!(pixel(&look, [0, 255, 0])[3], 0);
        assert_eq!(pixel(&look, [20, 235, 30])[3], 0);
        assert_eq!(pixel(&look, [200, 60, 50])[3], 255);
        // Цвет при этом не меняется.
        assert_eq!(pixel(&look, [200, 60, 50])[..3], [200, 60, 50]);
        assert_eq!(look.filters(1080), ",colorkey=0x00FF00:0.3000:0.1000");
    }

    #[test]
    fn spill_is_removed_from_the_subject_and_the_edge_is_eaten() {
        // Зеленоватая точка теряет лишний зелёный, остальные цвета не меняются.
        let key = ChromaKey { similarity: 0.1, blend: 0.0, ..ChromaKey::studio([0, 255, 0]) };
        let look = Look { key: Some(key), ..Look::default() };
        assert_eq!(pixel(&look, [120, 200, 100]), [120, 155, 100, 255]);
        assert_eq!(pixel(&look, [200, 60, 50]), [200, 60, 50, 255]);
        // У синего фона гасится синий, у фона другого цвета отсвет не трогают.
        let blue = Look { key: Some(ChromaKey { similarity: 0.1, blend: 0.0, ..ChromaKey::studio([0, 0, 255]) }), ..Look::default() };
        assert_eq!(pixel(&blue, [100, 120, 220]), [100, 120, 165, 255]);
        let red = Look { key: Some(ChromaKey { similarity: 0.1, blend: 0.0, ..ChromaKey::studio([255, 0, 0]) }), ..Look::default() };
        assert_eq!(pixel(&red, [120, 200, 100]), [120, 200, 100, 255]);

        let key = ChromaKey { choke: 0.5, ..ChromaKey::studio([0, 255, 0]) };
        assert_eq!((key.choke_passes(1080), key.choke_passes(540), ChromaKey::default().choke_passes(1080)), (4, 2, 0));
        let f = Look { key: Some(key), ..Look::default() }.filters(540);
        assert_eq!(
            f,
            ",colorkey=0x00FF00:0.3000:0.1000,despill=type=green:mix=0.5:expand=0:green=-0.5000:blue=0,format=gbrap\
             ,erosion=threshold0=0:threshold1=0:threshold2=0,erosion=threshold0=0:threshold1=0:threshold2=0,format=rgba"
        );
        assert!(blue.filters(540).contains("despill=type=blue:mix=0.5:expand=0:green=0:blue=-0.5000"));
        assert!(!red.filters(540).contains("despill"));

        // Проект, сохранённый до появления этих настроек, открывается с ними выключенными.
        let old: ChromaKey = serde_json::from_str(r#"{"color":[0,255,0],"similarity":0.3,"blend":0.1}"#).unwrap();
        assert_eq!(old, ChromaKey::default());
    }

    #[test]
    fn export_filters_carry_the_same_numbers() {
        let look = Look { filter: FilterKind::Sepia, contrast: 0.2, ..Look::default() };
        let f = look.filters(1080);
        assert!(f.starts_with(",lutrgb=r='clip(((val/255-0.5)*1.2500+0.5+0.0000)*1.0000+0.0000,0,1)*255+0.5':g='"), "{f}");
        assert!(f.ends_with(",colorchannelmixer=rr=0.3930:rg=0.7690:rb=0.1890:gr=0.3490:gg=0.6860:gb=0.1680:br=0.2720:bg=0.5340:bb=0.1310"), "{f}");
    }
}
