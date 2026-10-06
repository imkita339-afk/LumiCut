//! Процедурные эффекты: кадр целиком вычисляется из параметров эффекта и момента времени,
//! поэтому любой кадр можно получить сразу, без проигрывания с начала.

use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::motion::full_span;

pub const MIN_RES: u32 = 16;
pub const MAX_RES: u32 = 256;
pub const MAX_COUNT: u32 = 400;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum EffectKind {
    Explosion,
    Fire,
    Smoke,
    Sparks,
    Firework,
    Confetti,
    Shockwave,
    Lightning,
    Snow,
    Rain,
    Glitter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub kind: EffectKind,
    /// Зерно случайности: с другим зерном тот же эффект разлетается иначе.
    pub seed: u32,
    /// Число частиц (для ударной волны — колец).
    pub count: u32,
    /// Разлёт в долях кадра эффекта.
    pub spread: f32,
    /// Размер частицы в долях кадра эффекта.
    pub size: f32,
    /// Тяга вниз; отрицательная поднимает частицы.
    pub gravity: f32,
    /// Сторона кадра эффекта в точках: чем меньше, тем крупнее «пиксели».
    pub res: u32,
    /// Цвет в начале и в конце жизни частицы.
    pub colors: [[u8; 3]; 2],
    /// Какая часть эффекта приходится на этот клип: меняется, когда клип разрезают.
    #[serde(default = "full_span")]
    pub span: [f32; 2],
}

impl EffectKind {
    pub const ALL: [EffectKind; 11] = [
        EffectKind::Explosion,
        EffectKind::Fire,
        EffectKind::Smoke,
        EffectKind::Sparks,
        EffectKind::Firework,
        EffectKind::Confetti,
        EffectKind::Shockwave,
        EffectKind::Lightning,
        EffectKind::Snow,
        EffectKind::Rain,
        EffectKind::Glitter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EffectKind::Explosion => "Взрыв",
            EffectKind::Fire => "Огонь",
            EffectKind::Smoke => "Дым",
            EffectKind::Sparks => "Искры",
            EffectKind::Firework => "Фейерверк",
            EffectKind::Confetti => "Конфетти",
            EffectKind::Shockwave => "Ударная волна",
            EffectKind::Lightning => "Молния",
            EffectKind::Snow => "Снег",
            EffectKind::Rain => "Дождь",
            EffectKind::Glitter => "Блёстки",
        }
    }

    /// Что считает первый ползунок эффекта.
    pub fn count_label(self) -> &'static str {
        match self {
            EffectKind::Shockwave => "Колец",
            EffectKind::Lightning => "Ветвей",
            _ => "Частиц",
        }
    }

    /// Эффект идёт по всему кадру проекта, а не из одной точки.
    pub fn fills_frame(self) -> bool {
        matches!(self, EffectKind::Snow | EffectKind::Rain | EffectKind::Glitter)
    }

    /// Длительность клипа с эффектом по умолчанию, секунды.
    pub fn duration(self) -> f64 {
        match self {
            EffectKind::Explosion => 1.2,
            EffectKind::Fire => 1.6,
            EffectKind::Smoke => 2.5,
            EffectKind::Sparks => 1.0,
            EffectKind::Firework => 2.0,
            EffectKind::Confetti => 2.5,
            EffectKind::Shockwave => 0.8,
            EffectKind::Lightning => 0.8,
            EffectKind::Snow => 4.0,
            EffectKind::Rain => 3.0,
            EffectKind::Glitter => 2.5,
        }
    }

    /// Доля эффекта, на которой он выглядит нагляднее всего: кадр для плитки в библиотеке.
    pub fn poster(self) -> f32 {
        match self {
            EffectKind::Explosion | EffectKind::Sparks => 0.3,
            EffectKind::Confetti => 0.35,
            EffectKind::Shockwave => 0.45,
            EffectKind::Lightning => 0.12,
            _ => 0.5,
        }
    }

    /// Наибольшее число частиц, которое имеет смысл для эффекта.
    pub fn max_count(self) -> u32 {
        match self {
            EffectKind::Shockwave => 8,
            EffectKind::Lightning => 12,
            _ => MAX_COUNT,
        }
    }
}

impl Effect {
    pub fn new(kind: EffectKind, seed: u32) -> Self {
        let (count, spread, size, gravity, res, colors) = match kind {
            EffectKind::Explosion => (28, 0.6, 0.2, -0.04, 64, [[255, 255, 255], [128, 128, 128]]),
            EffectKind::Fire => (90, 0.26, 0.13, -1.0, 64, [[255, 226, 96], [196, 42, 12]]),
            EffectKind::Smoke => (34, 0.3, 0.2, -0.7, 64, [[232, 232, 232], [92, 92, 98]]),
            EffectKind::Sparks => (70, 0.9, 0.012, 0.9, 128, [[255, 246, 205], [255, 120, 24]]),
            EffectKind::Firework => (110, 0.8, 0.012, 0.3, 128, [[255, 240, 180], [255, 64, 128]]),
            EffectKind::Confetti => (130, 1.0, 0.035, 1.3, 128, [[255, 88, 72], [120, 92, 255]]),
            EffectKind::Shockwave => (2, 0.9, 0.05, 0.0, 128, [[255, 255, 255], [120, 200, 255]]),
            EffectKind::Lightning => (3, 0.6, 0.014, 0.0, 128, [[255, 255, 255], [130, 170, 255]]),
            EffectKind::Snow => (120, 0.4, 0.022, 0.6, 128, [[255, 255, 255], [190, 215, 255]]),
            EffectKind::Rain => (140, 0.25, 0.07, 1.4, 128, [[205, 228, 255], [120, 160, 230]]),
            EffectKind::Glitter => (44, 1.0, 0.05, 0.0, 128, [[255, 255, 255], [255, 214, 110]]),
        };
        Self { kind, seed, count, spread, size, gravity, res, colors, span: full_span() }
    }

    /// Делит эффект между двумя частями разрезанного клипа; `k` — доля левой части.
    pub fn split(&mut self, right: &mut Effect, k: f32) {
        let mid = self.span[0] + (self.span[1] - self.span[0]) * k;
        self.span[1] = mid;
        right.span[0] = mid;
    }
}

// ---------- Случайность и цвет ----------

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

/// Число 0..1, определяемое зерном, номером частицы и номером величины.
fn rnd(seed: u32, particle: u32, slot: u32) -> f32 {
    let h = hash(seed ^ hash(particle.wrapping_mul(0x9E37_79B9) ^ hash(slot.wrapping_add(0x85EB_CA6B))));
    (h >> 8) as f32 / (1u32 << 24) as f32
}

type Rgb = [f32; 3];

fn rgb(c: [u8; 3]) -> Rgb {
    c.map(|v| v as f32 / 255.0)
}

fn mix(a: Rgb, b: Rgb, k: f32) -> Rgb {
    let k = k.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * k)
}

fn to_hsv(c: Rgb) -> Rgb {
    let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    [h / 6.0, if max > 0.0 { d / max } else { 0.0 }, max]
}

fn from_hsv(c: Rgb) -> Rgb {
    let h = c[0].rem_euclid(1.0) * 6.0;
    [5.0, 3.0, 1.0].map(|n: f32| {
        let k = (n + h) % 6.0;
        c[2] - c[2] * c[1] * k.min(4.0 - k).clamp(0.0, 1.0)
    })
}

/// Цвет между `a` и `b` по цветовому кругу: между красным и фиолетовым лежит вся радуга.
fn mix_hue(a: Rgb, b: Rgb, k: f32) -> Rgb {
    let (a, b) = (to_hsv(a), to_hsv(b));
    from_hsv([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * k))
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Путь частицы с торможением о воздух: к концу эффекта проходит единицу.
fn reach(t: f32) -> f32 {
    const DRAG: f32 = 3.0;
    (1.0 - (-DRAG * t).exp()) / (1.0 - (-DRAG).exp())
}

// ---------- Холст ----------

/// Кадр эффекта: цвет хранится уже умноженным на непрозрачность.
struct Canvas {
    n: i32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(n: u32) -> Self {
        Self { n: n as i32, px: vec![[0.0; 4]; (n * n) as usize] }
    }

    /// Кладёт цвет поверх точки.
    fn put(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= self.n || y >= self.n || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let p = &mut self.px[(y * self.n + x) as usize];
        for i in 0..3 {
            p[i] = c[i] * a + p[i] * (1.0 - a);
        }
        p[3] = a + p[3] * (1.0 - a);
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb, a: f32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c, a);
            }
        }
    }

    /// Круг с центром в точках холста; при радиусе меньше точки закрашивается одна точка.
    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        if r < 0.75 {
            self.put(cx.floor() as i32, cy.floor() as i32, c, a);
            return;
        }
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.put(x, y, c, a);
                }
            }
        }
    }

    /// RGBA с обычной (не умноженной) прозрачностью.
    fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3];
            for c in &p[..3] {
                out.push(if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 });
            }
            out.push((a * 255.0).round() as u8);
        }
        out
    }
}

// ---------- Эффекты ----------

/// Клубы из крупных «пикселей»: взрыв, огонь и дым.
fn puffs(c: &mut Canvas, e: &Effect, t: f32) {
    /// Сторона клуба в его собственных «пикселях».
    const GRID: i32 = 8;
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));

    if e.kind == EffectKind::Explosion && t < 0.14 {
        let k = t / 0.14;
        let r = e.spread * 0.5 * (0.35 + 0.65 * k) * n;
        c.disc(n / 2.0, n / 2.0, r, mix(first, [1.0; 3], 0.7), 1.0 - k);
    }

    for i in 0..e.count.min(MAX_COUNT) {
        let r = |slot: u32| rnd(e.seed, i, slot);
        // Сколько клуб живёт и когда появляется, в долях эффекта: к концу эффекта исчезают все.
        let (life, births) = match e.kind {
            EffectKind::Explosion => (0.4 + 0.2 * r(1), 0.4),
            EffectKind::Fire => (0.3 + 0.18 * r(1), 1.0),
            _ => (0.45 + 0.2 * r(1), 0.8),
        };
        let born = r(0) * (1.0 - life) * births;
        let age = (t - born) / life;
        if !(0.0..1.0).contains(&age) {
            continue;
        }
        let since = t - born;

        let (mut x, mut y) = if e.kind == EffectKind::Explosion {
            let (angle, far) = (r(2) * TAU, e.spread * 0.5 * r(3).sqrt());
            (0.5 + angle.cos() * far * (1.0 + 0.3 * age), 0.5 + angle.sin() * far * (1.0 + 0.3 * age))
        } else {
            // Пламя сходится кверху, дым расползается.
            let widen = if e.kind == EffectKind::Fire { 1.0 - 0.5 * age } else { 1.0 + 0.8 * age };
            (0.5 + (r(2) - 0.5) * e.spread * widen, 0.8 + (r(3) - 0.5) * 0.06)
        };
        x += (r(4) - 0.5) * 0.08 * age;
        y += e.gravity * since;

        // Рост, выветривание и цвет зависят от возраста клуба.
        let (grow, peak, erode, shade, alpha) = match e.kind {
            EffectKind::Explosion => (0.45 + 0.55 * ease_out(age * 2.5), 1.0, smoothstep(0.4, 1.0, age), r(6), 1.0),
            EffectKind::Fire => (1.0 - 0.65 * age, 1.0, 0.9 * smoothstep(0.25, 1.0, age), age.powf(0.8) + 0.2 * (r(6) - 0.5), 1.0),
            _ => (0.5 + 0.7 * age, 1.2, smoothstep(0.3, 1.0, age), 0.75 * age + 0.25 * r(6), 0.85),
        };
        let color = mix(first, last, shade);

        // Клуб выровнен по точкам холста, чтобы его «пиксели» были одинаковыми.
        let largest = e.size * (0.6 + 0.4 * r(5)) * peak;
        let texel = ((2.0 * largest * n / GRID as f32).round() as i32).max(1);
        let side = texel * GRID;
        let (x0, y0) = ((x * n).round() as i32 - side / 2, (y * n).round() as i32 - side / 2);
        let radius = GRID as f32 / 2.0 * grow / peak;
        for v in 0..GRID {
            for u in 0..GRID {
                let cell = (v * GRID + u) as u32;
                let (du, dv) = (u as f32 + 0.5 - GRID as f32 / 2.0, v as f32 + 0.5 - GRID as f32 / 2.0);
                let d = (du * du + dv * dv).sqrt() / radius;
                let inside = d + 0.3 * (r(100 + cell) - 0.5) < 1.0;
                let alive = r(300 + cell) > erode * (0.6 + 0.6 * d);
                if !inside || !alive {
                    continue;
                }
                // Свет падает слева сверху, край темнее середины.
                let mut light = 1.0 - 0.1 * (du + dv) / GRID as f32;
                if d > 0.72 {
                    light *= 0.86;
                }
                c.rect(x0 + u * texel, y0 + v * texel, texel, texel, color.map(|ch| (ch * light).min(1.0)), alpha);
            }
        }
    }
}

/// Искры со следом: россыпь из точки или фейерверк с подлётом ракеты.
fn sparks(c: &mut Canvas, e: &Effect, t: f32) {
    const TRAIL: u32 = 7;
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));
    let thick = e.size * n / 2.0;
    let firework = e.kind == EffectKind::Firework;
    let (launch, origin) = if firework { (0.22, (0.5, 0.36)) } else { (0.0, (0.5, 0.5)) };

    if t < launch {
        // Ракета поднимается к точке разрыва.
        for j in (0..TRAIL).rev() {
            let k = (t - j as f32 * 0.012).max(0.0) / launch;
            let y = 0.98 + (origin.1 - 0.98) * ease_out(k);
            c.disc(origin.0 * n, y * n, thick, mix(first, last, j as f32 / TRAIL as f32), 1.0 - j as f32 / TRAIL as f32);
        }
        return;
    }
    let t = (t - launch) / (1.0 - launch);
    let blink = (t * 40.0) as u32;

    for i in 0..e.count.min(MAX_COUNT) {
        let r = |slot: u32| rnd(e.seed, i, slot);
        let life = if firework { 0.7 + 0.3 * r(2) } else { 0.45 + 0.55 * r(2) };
        let age = t / life;
        if age >= 1.0 {
            continue;
        }
        // Фейерверк раскрывается шаром, россыпь — неровным снопом.
        let speed = e.spread * 0.5 * if firework { 0.3 + 0.7 * r(1).sqrt() } else { 0.2 + 0.8 * r(1) };
        let (angle, tail) = (r(0) * TAU, if firework { 0.014 } else { 0.01 });
        // Догорающие искры мерцают.
        if age > 0.6 && rnd(e.seed ^ blink, i, 7) < 0.4 {
            continue;
        }
        for j in (0..TRAIL).rev() {
            let at = t - j as f32 * tail;
            if at < 0.0 {
                continue;
            }
            let x = origin.0 + angle.cos() * speed * reach(at);
            let y = origin.1 + angle.sin() * speed * reach(at) + 0.5 * e.gravity * at * at;
            let fade = (1.0 - age).powf(0.7) * (1.0 - j as f32 / TRAIL as f32);
            c.disc(x * n, y * n, thick, mix(first, last, age + j as f32 * 0.08), fade);
        }
    }
}

/// Бумажки, которые подлетают, кружатся и осыпаются.
fn confetti(c: &mut Canvas, e: &Effect, t: f32) {
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));
    let fade = ((1.0 - t) / 0.15).min(1.0);
    for i in 0..e.count.min(MAX_COUNT) {
        let r = |slot: u32| rnd(e.seed, i, slot);
        let angle = -TAU / 4.0 + (r(0) - 0.5) * 1.9;
        let speed = e.spread * (0.45 + 0.75 * r(1));
        let sway = 0.015 * (t * 9.0 + r(3) * TAU).sin();
        let x = 0.5 + angle.cos() * speed * 0.6 * reach(t) + sway;
        let y = 0.8 + angle.sin() * speed * 1.05 * reach(t) + 0.5 * e.gravity * t * t;
        // Бумажка поворачивается ребром к зрителю и обратно.
        let turn = (t * (6.0 + 10.0 * r(4)) + r(5) * TAU).cos().abs();
        let (w, h) = ((e.size * n * turn).round().max(1.0) as i32, (e.size * n * 0.6).round().max(1.0) as i32);
        let color = mix_hue(first, last, r(2));
        c.rect((x * n) as i32 - w / 2, (y * n) as i32 - h / 2, w, h, color.map(|v| v * (0.75 + 0.25 * turn)), fade);
    }
}

/// Кольца, расходящиеся из середины.
fn shockwave(c: &mut Canvas, e: &Effect, t: f32) {
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));
    let rings = e.count.clamp(1, EffectKind::Shockwave.max_count());
    for i in 0..rings {
        let delay = 0.5 * i as f32 / rings as f32;
        let age = (t - delay) / (1.0 - delay);
        if !(0.0..1.0).contains(&age) {
            continue;
        }
        let radius = e.spread * 0.5 * ease_out(age) * n;
        let half = (e.size * n * (1.0 - 0.6 * age) / 2.0).max(0.5);
        let (color, alpha) = (mix(first, last, age), (1.0 - age).powf(1.5));
        let extent = (radius + half).ceil() as i32 + 1;
        let mid = c.n / 2;
        for y in mid - extent..=mid + extent {
            for x in mid - extent..=mid + extent {
                let (dx, dy) = (x as f32 + 0.5 - n / 2.0, y as f32 + 0.5 - n / 2.0);
                if ((dx * dx + dy * dy).sqrt() - radius).abs() <= half {
                    c.put(x, y, color, alpha);
                }
            }
        }
    }
}

/// Зигзаг молнии: случайное блуждание от точки `from` на `len` вниз; `lean` — уклон в сторону.
fn bolt(e: &Effect, salt: u32, from: (f32, f32), len: f32, lean: f32, steps: u32) -> Vec<(f32, f32)> {
    let mut points = vec![from];
    let (mut x, mut drift) = (from.0, 0.0f32);
    for k in 1..=steps {
        // Каждый шаг уводит в сторону, но тянет обратно к прямой, чтобы молния не уходила из кадра.
        drift = drift * 0.6 + (rnd(e.seed, salt.wrapping_add(k), 11) - 0.5) * e.spread * 0.22;
        x += drift + lean / steps as f32;
        points.push((x, from.1 + len * k as f32 / steps as f32));
    }
    points
}

/// Молния: ствол с ветвями, который вспыхивает несколько раз и гаснет.
fn lightning(c: &mut Canvas, e: &Effect, t: f32) {
    let n = c.n as f32;
    let (core, halo) = (rgb(e.colors[0]), rgb(e.colors[1]));
    // Удары один за другим, каждый слабее предыдущего.
    let pulse = |at: f32, power: f32| if t < at { 0.0 } else { power * (-(t - at) * 9.0).exp() };
    let glow = pulse(0.08, 1.0).max(pulse(0.38, 0.8)).max(pulse(0.62, 0.5));
    if glow < 0.02 {
        return;
    }
    let thick = (e.size * n / 2.0).max(0.5);
    // Ствол и ветви: ломаная, её толщина и яркость.
    let trunk = bolt(e, 0, (0.5 + (rnd(e.seed, 0, 12) - 0.5) * 0.2, 0.02), 0.94, 0.0, 26);
    let mut strokes = vec![(trunk.clone(), thick, glow)];
    for i in 0..e.count.min(EffectKind::Lightning.max_count()) {
        let from = trunk[2 + (rnd(e.seed, i, 13) * 18.0) as usize];
        let side = if rnd(e.seed, i, 14) < 0.5 { -1.0 } else { 1.0 };
        let len = 0.15 + 0.2 * rnd(e.seed, i, 15);
        strokes.push((bolt(e, 100 + i * 40, from, len, side * (0.12 + 0.2 * rnd(e.seed, i, 16)), 9), thick * 0.55, glow * 0.75));
    }
    // Сначала ореол всех ветвей, потом их яркая середина: ореол соседней ветви её не закрашивает.
    for bright in [false, true] {
        for (points, width, power) in &strokes {
            for pair in points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let steps = (((b.0 - a.0).hypot(b.1 - a.1) * n).ceil() as u32).max(1);
                for s in 0..steps {
                    let k = s as f32 / steps as f32;
                    let (x, y) = ((a.0 + (b.0 - a.0) * k) * n, (a.1 + (b.1 - a.1) * k) * n);
                    if bright {
                        c.disc(x, y, *width, mix(core, halo, 1.0 - power), *power);
                    } else {
                        c.disc(x, y, width * 3.0, halo, 0.12 * power);
                    }
                }
            }
        }
    }
}

/// Снег и дождь: частицы идут сверху вниз по всему кадру и возвращаются наверх.
fn weather(c: &mut Canvas, e: &Effect, t: f32) {
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));
    let rain = e.kind == EffectKind::Rain;
    for i in 0..e.count.min(MAX_COUNT) {
        let r = |slot: u32| rnd(e.seed, i, slot);
        // Ближние частицы крупнее, ярче и падают быстрее.
        let near = r(1);
        let speed = e.gravity * if rain { 4.0 } else { 2.5 } * (0.6 + 0.8 * near);
        let y = (r(2) + t * speed).rem_euclid(1.0) * 1.2 - 0.1;
        let color = mix(first, last, r(5));
        if rain {
            // Струя: короткий наклонный штрих, яркий у головы.
            let len = (e.size * n * (0.6 + 0.8 * near)).max(2.0);
            let x = (r(0) * (1.0 + e.spread) - e.spread * 0.5 + (y + 0.1) * e.spread * 0.5) * n;
            for j in 0..len as i32 {
                let k = j as f32 / len;
                c.put((x - e.spread * 0.5 * j as f32) as i32, (y * n) as i32 - j, color, (0.35 + 0.5 * near) * (1.0 - k));
            }
        } else {
            let sway = e.spread * 0.06 * (t * TAU * (1.0 + 2.0 * r(3)) + r(4) * TAU).sin();
            c.disc((r(0) + sway) * n, y * n, e.size * n * (0.4 + 0.6 * near) / 2.0, color, 0.45 + 0.55 * near);
        }
    }
}

/// Блёстки: звёздочки вспыхивают и гаснут в случайных местах.
fn glitter(c: &mut Canvas, e: &Effect, t: f32) {
    let n = c.n as f32;
    let (first, last) = (rgb(e.colors[0]), rgb(e.colors[1]));
    for i in 0..e.count.min(MAX_COUNT) {
        let r = |slot: u32| rnd(e.seed, i, slot);
        let life = 0.25 + 0.3 * r(3);
        let born = r(2) * (1.0 - life);
        let age = (t - born) / life;
        if !(0.0..1.0).contains(&age) {
            continue;
        }
        let shine = (age * TAU / 2.0).sin();
        let (x, y) = (0.5 + (r(0) - 0.5) * e.spread, 0.5 + (r(1) - 0.5) * e.spread + e.gravity * (t - born));
        let (cx, cy) = ((x * n) as i32, (y * n) as i32);
        let color = mix(first, last, r(4));
        // Четыре луча: чем дальше от середины, тем слабее.
        let arm = (e.size * n * (0.5 + 0.5 * r(5)) * shine).round() as i32;
        c.put(cx, cy, [1.0; 3], shine);
        for d in 1..=arm {
            let a = shine * (1.0 - d as f32 / (arm + 1) as f32);
            for (dx, dy) in [(d, 0), (-d, 0), (0, d), (0, -d)] {
                c.put(cx + dx, cy + dy, color, a);
            }
        }
    }
}

/// Сторона кадра эффекта в точках.
pub fn side(e: &Effect) -> u32 {
    e.res.clamp(MIN_RES, MAX_RES)
}

/// Сколько кадров эффекта в клипе длиной `duration` секунд; кадр `n` показывает долю `n / число`.
pub fn frame_count(duration: f64, fps: f64) -> u32 {
    (duration * fps - 1e-6).ceil().max(1.0) as u32
}

/// Квадратный кадр эффекта в RGBA (прозрачность не умножена), когда пройдена доля `progress` клипа.
pub fn render(e: &Effect, progress: f32) -> Vec<u8> {
    let t = e.span[0] + (e.span[1] - e.span[0]) * progress.clamp(0.0, 1.0);
    let mut canvas = Canvas::new(side(e));
    match e.kind {
        EffectKind::Explosion | EffectKind::Fire | EffectKind::Smoke => puffs(&mut canvas, e, t),
        EffectKind::Sparks | EffectKind::Firework => sparks(&mut canvas, e, t),
        EffectKind::Confetti => confetti(&mut canvas, e, t),
        EffectKind::Shockwave => shockwave(&mut canvas, e, t),
        EffectKind::Lightning => lightning(&mut canvas, e, t),
        EffectKind::Snow | EffectKind::Rain => weather(&mut canvas, e, t),
        EffectKind::Glitter => glitter(&mut canvas, e, t),
    }
    canvas.rgba()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_has_expected_size_and_is_repeatable() {
        for kind in EffectKind::ALL {
            let e = Effect::new(kind, 7);
            let a = render(&e, 0.4);
            assert_eq!(a.len(), (e.res * e.res * 4) as usize, "{kind:?}");
            assert_eq!(a, render(&e, 0.4), "{kind:?}: кадр должен зависеть только от параметров");
            assert!(a.chunks(4).any(|p| p[3] > 0), "{kind:?}: в середине эффекта кадр не пустой");
        }
    }

    #[test]
    fn weather_keeps_falling_and_lightning_strikes_in_flashes() {
        // Снег и дождь идут всё время клипа: и в начале, и в самом конце кадр не пустой.
        for kind in [EffectKind::Snow, EffectKind::Rain] {
            let e = Effect::new(kind, 5);
            for t in [0.0, 0.5, 1.0] {
                assert!(render(&e, t).chunks(4).filter(|p| p[3] > 0).count() > 30, "{kind:?} в момент {t}");
            }
            assert_ne!(render(&e, 0.2), render(&e, 0.3), "{kind:?}: частицы движутся");
            assert!(kind.fills_frame());
        }
        // Молния: до первого удара пусто, в момент удара ярко, к концу почти погасла.
        let e = Effect::new(EffectKind::Lightning, 5);
        let lit = |t: f32| render(&e, t).chunks(4).map(|p| p[3] as u32).sum::<u32>();
        assert_eq!(lit(0.02), 0);
        assert!(lit(0.1) > lit(0.3) && lit(0.3) > 0 && lit(0.4) > lit(0.36));
        assert!(!EffectKind::Lightning.fills_frame());
        // Больше ветвей — больше закрашено.
        assert!(render(&Effect { count: 10, ..e.clone() }, 0.1).chunks(4).filter(|p| p[3] > 0).count()
            > render(&Effect { count: 0, ..e }, 0.1).chunks(4).filter(|p| p[3] > 0).count());
    }

    #[test]
    fn seed_changes_the_picture() {
        let (a, b) = (Effect::new(EffectKind::Explosion, 1), Effect::new(EffectKind::Explosion, 2));
        assert_ne!(render(&a, 0.4), render(&b, 0.4));
    }

    #[test]
    fn split_halves_continue_each_other() {
        let mut left = Effect::new(EffectKind::Sparks, 3);
        let whole = left.clone();
        let mut right = left.clone();
        left.split(&mut right, 0.5);
        assert_eq!(render(&left, 1.0), render(&whole, 0.5));
        assert_eq!(render(&right, 0.0), render(&whole, 0.5));
    }

    #[test]
    fn hue_mix_keeps_end_colours() {
        let (a, b) = (rgb([255, 72, 88]), rgb([120, 92, 255]));
        for (got, want) in [(mix_hue(a, b, 0.0), a), (mix_hue(a, b, 1.0), b)] {
            for i in 0..3 {
                assert!((got[i] - want[i]).abs() < 0.01, "{got:?} и {want:?}");
            }
        }
    }
}
