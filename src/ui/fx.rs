//! Отклик интерфейса: пружины нажатий, вспышки, волны и искры.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::Arc;

use eframe::egui::{
    emath::{easing, TSTransform},
    epaint::Shadow,
    lerp,
    mutex::Mutex,
    vec2, Color32, Context, Galley, Id, LayerId, Order, Painter, Pos2, Rect, Response, Shape, Stroke, Ui, UiBuilder, Vec2,
};

use crate::theme::*;

/// Жёсткость и затухание пружины: быстрый отклик с небольшим перелётом.
const STIFF: f32 = 900.0;
const DAMP: f32 = 26.0;
/// Толчок, от которого пружина отклоняется примерно на единицу.
pub const KICK: f32 = 51.0;
/// Сколько секунд сменившееся содержимое панели проявляется и подъезжает на место.
const ENTER: f32 = 0.24;

#[derive(Clone, Copy)]
struct Spring {
    x: f32,
    v: f32,
    /// Проход, в котором пружину считали последний раз.
    pass: u64,
}

struct Spark {
    pos: Pos2,
    vel: Vec2,
    gravity: f32,
    size: f32,
    color: Color32,
    clip: Rect,
    age: f32,
    life: f32,
}

enum Kind {
    /// Гаснущая заливка.
    Flash { rect: Rect, rounding: f32 },
    /// Контур, расходящийся от прямоугольника.
    Pulse { rect: Rect, rounding: f32 },
    /// Круговая волна.
    Ring { center: Pos2, from: f32, to: f32 },
    /// Росчерк, вырастающий из середины.
    Slash { a: Pos2, b: Pos2 },
    /// Прямоугольник, схлопывающийся в линию.
    Poof { rect: Rect, rounding: f32 },
}

struct Glow {
    kind: Kind,
    color: Color32,
    clip: Rect,
    age: f32,
    life: f32,
}

impl Glow {
    fn paint(&self, p: &Painter) {
        let t = (self.age / self.life).clamp(0.0, 1.0);
        let out = easing::cubic_out(t);
        let fade = 1.0 - t;
        let line = Stroke::new(0.5 + 1.5 * fade, self.color.gamma_multiply(fade));
        match self.kind {
            Kind::Flash { rect, rounding } => {
                p.rect_filled(rect, rounding, self.color.gamma_multiply(fade * fade));
            }
            Kind::Pulse { rect, rounding } => {
                let d = 6.0 * out;
                p.rect_stroke(rect.expand(d), rounding + d, line);
            }
            Kind::Ring { center, from, to } => {
                p.circle_stroke(center, lerp(from..=to, out), line);
            }
            Kind::Slash { a, b } => {
                let mid = a + (b - a) * 0.5;
                let half = (b - a) * 0.5 * easing::cubic_out((t * 4.0).min(1.0));
                let ends = [mid - half, mid + half];
                p.line_segment(ends, Stroke::new(9.0 * fade, additive(self.color.gamma_multiply(0.3 * fade))));
                let core = Color32::WHITE.lerp_to_gamma(self.color, t).gamma_multiply(fade);
                p.line_segment(ends, Stroke::new(0.5 + 2.5 * fade, core));
            }
            Kind::Poof { rect, rounding } => {
                let size = vec2(rect.width(), rect.height() * (1.0 - out));
                p.rect_filled(Rect::from_center_size(rect.center(), size), rounding, self.color.gamma_multiply(fade));
            }
        }
    }
}

#[derive(Default)]
struct State {
    springs: HashMap<Id, Spring>,
    sparks: Vec<Spark>,
    glows: Vec<Glow>,
    /// Что показано под каждым именем и с какого момента.
    enters: HashMap<Id, (u64, f64)>,
    /// Когда элемент появился на экране и в каком проходе его видели последний раз.
    births: HashMap<Id, (f64, u64)>,
    seed: u32,
}

impl State {
    /// Псевдослучайное число в 0..1.
    fn rnd(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn spark(&mut self, clip: Rect, pos: Pos2, vel: Vec2, color: Color32) {
        let (size, life) = (1.2 + self.rnd(), 0.25 + 0.3 * self.rnd());
        self.sparks.push(Spark { pos, vel, gravity: 520.0, size, color, clip, age: 0.0, life });
    }
}

#[derive(Clone, Default)]
struct Store(Arc<Mutex<State>>);

/// Состояние эффектов живёт в памяти egui, чтобы до него дотягивался любой виджет.
fn with<R>(ctx: &Context, f: impl FnOnce(&mut State) -> R) -> R {
    let store = ctx.data_mut(|d| d.get_temp_mut_or_default::<Store>(Id::NULL).clone());
    let mut state = store.0.lock();
    f(&mut state)
}

/// Цвет, который складывается с фоном, а не перекрывает его.
fn additive(c: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 0)
}

// ---------- Пружины и плавные переходы ----------

/// Значение, догоняющее `target` по пружине; `kick` — разовый толчок скорости.
pub fn spring(ctx: &Context, id: Id, target: f32, kick: f32) -> f32 {
    let (dt, pass) = (ctx.input(|i| i.stable_dt).min(0.05), ctx.cumulative_pass_nr());
    let (x, moving) = with(ctx, |s| {
        let sp = s.springs.entry(id).or_insert(Spring { x: target, v: 0.0, pass });
        sp.v += kick;
        if sp.pass != pass {
            sp.pass = pass;
            let steps = (dt / 0.004).ceil().max(1.0) as usize;
            let h = dt / steps as f32;
            for _ in 0..steps {
                sp.v += (STIFF * (target - sp.x) - DAMP * sp.v) * h;
                sp.x += sp.v * h;
            }
        }
        let moving = (target - sp.x).abs() > 1e-3 || sp.v.abs() > 1e-2;
        if !moving {
            (sp.x, sp.v) = (target, 0.0);
        }
        (sp.x, moving)
    });
    if moving {
        ctx.request_repaint();
    }
    x
}

/// Толкает пружину `id`; к своей цели она вернётся сама.
pub fn kick(ctx: &Context, id: Id, v: f32) {
    let pass = ctx.cumulative_pass_nr();
    with(ctx, |s| s.springs.entry(id).or_insert(Spring { x: 0.0, v: 0.0, pass }).v += v);
    ctx.request_repaint();
}

/// Масштаб кнопки: сжимается под указателем и отпружинивает после щелчка.
pub fn press(ctx: &Context, resp: &Response, depth: f32) -> f32 {
    let down = if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    let pop = if resp.clicked() { -0.4 * KICK } else { 0.0 };
    1.0 - depth * spring(ctx, resp.id.with("press"), down, pop)
}

/// Плавное 0..1 по наведению.
pub fn hover(ctx: &Context, resp: &Response) -> f32 {
    ctx.animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.1)
}

/// Истина в том кадре, когда значение под `id` сменилось.
pub fn changed<T: PartialEq + Clone + Send + Sync + 'static>(ctx: &Context, id: Id, value: T) -> bool {
    ctx.data_mut(|d| {
        let prev = d.get_temp::<T>(id);
        let diff = prev.as_ref().is_some_and(|p| *p != value);
        if diff || prev.is_none() {
            d.insert_temp(id, value);
        }
        diff
    })
}

/// Плавное 0..1 с того момента, как под `id` сменился `key`. При первом показе это сразу единица.
pub fn enter(ctx: &Context, id: Id, key: impl std::hash::Hash) -> f32 {
    let (hash, now) = (eframe::egui::util::hash(key), ctx.input(|i| i.time));
    let since = with(ctx, |s| {
        let shown = s.enters.entry(id).or_insert((hash, f64::NEG_INFINITY));
        if shown.0 != hash {
            *shown = (hash, now);
        }
        shown.1
    });
    let t = ((now - since) as f32 / ENTER).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    easing::cubic_out(t)
}

/// Плавное 0..1 с того момента, как элемент `id` появился на экране.
pub fn fresh(ctx: &Context, id: Id) -> f32 {
    let (now, pass) = (ctx.input(|i| i.time), ctx.cumulative_pass_nr());
    let since = with(ctx, |s| {
        let born = s.births.entry(id).or_insert((now, pass));
        born.1 = pass;
        born.0
    });
    let t = ((now - since) as f32 / ENTER).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    easing::cubic_out(t)
}

/// Содержимое, которое после смены `key` проявляется и подъезжает на место со сдвига `offset`.
pub fn appear<R>(ui: &mut Ui, id: Id, key: impl std::hash::Hash, offset: Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = enter(ui.ctx(), id, key);
    if t >= 1.0 {
        return add(ui);
    }
    let rect = ui.available_rect_before_wrap().translate(offset * (1.0 - t));
    let layout = *ui.layout();
    ui.allocate_new_ui(UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.multiply_opacity(t);
        add(ui)
    })
    .inner
}

/// Прямоугольник, растянутый в `scale` раз относительно `pivot`.
pub fn zoom(rect: Rect, pivot: Pos2, scale: f32) -> Rect {
    Rect::from_min_max(pivot + (rect.min - pivot) * scale, pivot + (rect.max - pivot) * scale)
}

/// Текст, масштабированный относительно `pivot` вместе со своей кнопкой.
pub fn text(p: &Painter, pos: Pos2, galley: Arc<Galley>, color: Color32, pivot: Pos2, scale: f32) {
    let mut shape = Shape::galley_with_override_text_color(pos, galley, color);
    if (scale - 1.0).abs() > 1e-3 {
        shape.transform(TSTransform::new(pivot.to_vec2() * (1.0 - scale), scale));
    }
    p.add(shape);
}

/// Мягкое свечение вокруг прямоугольника; рисуется до самого элемента.
pub fn halo(p: &Painter, rect: Rect, rounding: f32, color: Color32, blur: f32) {
    p.add(Shadow { offset: Vec2::ZERO, blur, spread: 0.0, color }.as_shape(rect, rounding));
}

// ---------- Разовые эффекты ----------

fn glow(ctx: &Context, clip: Rect, kind: Kind, color: Color32, life: f32) {
    with(ctx, |s| s.glows.push(Glow { kind, color, clip, age: 0.0, life }));
    ctx.request_repaint();
}

/// Гаснущая заливка.
pub fn flash(ctx: &Context, clip: Rect, rect: Rect, rounding: f32, color: Color32) {
    glow(ctx, clip, Kind::Flash { rect, rounding }, color, 0.32);
}

/// Контур, расходящийся от прямоугольника.
pub fn pulse(ctx: &Context, clip: Rect, rect: Rect, rounding: f32, color: Color32) {
    glow(ctx, clip, Kind::Pulse { rect, rounding }, color, 0.32);
}

/// Круговая волна от `from` до `to` пикселей.
pub fn ring(ctx: &Context, center: Pos2, from: f32, to: f32, color: Color32) {
    glow(ctx, Rect::EVERYTHING, Kind::Ring { center, from, to }, color, 0.36);
}

/// Росчерк разреза.
pub fn slash(ctx: &Context, clip: Rect, a: Pos2, b: Pos2, color: Color32) {
    glow(ctx, clip, Kind::Slash { a, b }, color, 0.34);
}

/// Прямоугольник, схлопывающийся в линию.
pub fn poof(ctx: &Context, clip: Rect, rect: Rect, rounding: f32, color: Color32) {
    glow(ctx, clip, Kind::Poof { rect, rounding }, color, 0.26);
}

/// Отклик кнопки на щелчок: вспышка и расходящийся контур.
pub fn tap(ctx: &Context, rect: Rect, rounding: f32, color: Color32) {
    flash(ctx, Rect::EVERYTHING, rect, rounding, color.gamma_multiply(0.16));
    pulse(ctx, Rect::EVERYTHING, rect, rounding, color.gamma_multiply(0.45));
}

/// Сноп искр из точки: `dir` задаёт направление и скорость (px/с), `spread` — разброс в радианах.
pub fn sparks(ctx: &Context, clip: Rect, origin: Pos2, dir: Vec2, spread: f32, count: usize, color: Color32) {
    with(ctx, |s| {
        for _ in 0..count {
            let angle = dir.angle() + (s.rnd() - 0.5) * spread;
            let speed = dir.length() * (0.35 + 0.65 * s.rnd());
            s.spark(clip, origin, Vec2::angled(angle) * speed, color);
        }
    });
    ctx.request_repaint();
}

/// Осколки, разлетающиеся из прямоугольника.
pub fn burst(ctx: &Context, clip: Rect, rect: Rect, count: usize, color: Color32) {
    with(ctx, |s| {
        for _ in 0..count {
            let pos = rect.lerp_inside(vec2(s.rnd(), s.rnd()));
            let vel = Vec2::angled(TAU * s.rnd()) * (60.0 + 180.0 * s.rnd()) - vec2(0.0, 70.0);
            s.spark(clip, pos, vel, color);
        }
    });
    ctx.request_repaint();
}

/// Конфетти: залп вверх из точки.
pub fn confetti(ctx: &Context, origin: Pos2) {
    const COLORS: [Color32; 4] = [ACCENT, WAVE, DANGER, TEXT];
    with(ctx, |s| {
        for i in 0..72 {
            let angle = -TAU / 4.0 + (s.rnd() - 0.5) * 2.4;
            let vel = Vec2::angled(angle) * (380.0 + 640.0 * s.rnd());
            let (size, life) = (2.5 + 2.0 * s.rnd(), 0.9 + 0.8 * s.rnd());
            s.sparks.push(Spark {
                pos: origin,
                vel,
                gravity: 900.0,
                size,
                color: COLORS[i % COLORS.len()],
                clip: Rect::EVERYTHING,
                age: 0.0,
                life,
            });
        }
    });
    ctx.request_repaint();
}

/// Играет ли сейчас хоть один эффект или пружина.
#[cfg(test)]
pub fn busy(ctx: &Context) -> bool {
    with(ctx, |s| !s.glows.is_empty() || !s.sparks.is_empty() || s.springs.values().any(|sp| sp.v != 0.0))
}

/// Продвигает и рисует эффекты поверх интерфейса; вызывается в конце кадра.
pub fn paint(ctx: &Context) {
    let (dt, pass) = (ctx.input(|i| i.stable_dt).min(0.05), ctx.cumulative_pass_nr());
    let pressed = ctx.input(|i| if i.pointer.primary_pressed() { i.pointer.interact_pos() } else { None });
    if let Some(pos) = pressed {
        ring(ctx, pos, 2.0, 15.0, TEXT.gamma_multiply(0.3));
    }

    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("fx")));
    let alive = with(ctx, |s| {
        // Пружины виджетов, пропавших с экрана, забываются: недоигранное движение не возобновится.
        s.springs.retain(|_, sp| pass.saturating_sub(sp.pass) <= 2);
        // Элемент, пропавший с экрана, при следующем показе появится заново.
        s.births.retain(|_, born| pass.saturating_sub(born.1) <= 2);

        for g in &mut s.glows {
            g.age += dt;
        }
        s.glows.retain(|g| g.age < g.life);
        for g in &s.glows {
            g.paint(&painter.with_clip_rect(g.clip));
        }

        for p in &mut s.sparks {
            p.age += dt;
            p.vel.y += p.gravity * dt;
            p.vel *= 1.0 - 2.5 * dt;
            p.pos += p.vel * dt;
        }
        s.sparks.retain(|p| p.age < p.life);
        for p in &s.sparks {
            let fade = 1.0 - (p.age / p.life).powi(2);
            let color = additive(p.color.gamma_multiply(fade));
            let painter = painter.with_clip_rect(p.clip);
            painter.line_segment([p.pos, p.pos - p.vel * 0.025], Stroke::new(p.size * 0.8, color));
            painter.circle_filled(p.pos, p.size * 0.5, color);
        }

        !s.glows.is_empty() || !s.sparks.is_empty()
    });
    if alive {
        ctx.request_repaint();
    }
}
