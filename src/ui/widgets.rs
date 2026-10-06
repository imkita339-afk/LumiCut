//! Общие элементы интерфейса.

use std::sync::Arc;

use eframe::egui::{
    self, epaint::Shadow, pos2, text::LayoutJob, text::TextWrapping, vec2, Align, Color32, FontId, Galley,
    Layout, Rect, Response, Rounding, Sense, Shape, Stroke, Ui, UiBuilder, Vec2,
};

use crate::icons;
use crate::theme::{self, *};
use crate::ui::fx;

/// Сторона кнопки закрепления значения.
pub const KEY_BUTTON: f32 = 18.0;

/// Скруглённая «карточка»-панель на фоне окна.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 10.0, PANEL);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let r = add(&mut child);
    ui.allocate_rect(rect, Sense::hover());
    r
}

/// Строка, обрезанная многоточием по ширине.
pub fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(1.0));
    ui.fonts(|f| f.layout_job(job))
}

pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, tip: &str, active: bool, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), sense);
    paint_icon_button(ui, rect, &resp, icon, active, enabled);
    resp.on_hover_text(tip)
}

/// То же, но в заданном прямоугольнике.
pub fn icon_button_at(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash, icon: &str, tip: &str, active: bool) -> Response {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    paint_icon_button(ui, rect, &resp, icon, active, true);
    resp.on_hover_text(tip)
}

fn paint_icon_button(ui: &Ui, rect: Rect, resp: &Response, icon: &str, active: bool, enabled: bool) {
    let ctx = ui.ctx();
    let hover = if enabled { fx::hover(ctx, resp) } else { 0.0 };
    let on = ctx.animate_bool_with_time(resp.id.with("on"), active, 0.14);
    let scale = if enabled { fx::press(ctx, resp, 0.12) } else { 1.0 };
    let bg = Color32::TRANSPARENT.lerp_to_gamma(ACTIVE, hover).lerp_to_gamma(HOVER, on);
    ui.painter().rect_filled(fx::zoom(rect, rect.center(), scale), 6.0, bg);
    let color = if enabled {
        TEXT2.lerp_to_gamma(TEXT, hover).lerp_to_gamma(ACCENT, on)
    } else {
        BORDER
    };
    // Значок подпрыгивает при переключении.
    let bump = 1.0 + 0.6 * on * (1.0 - on);
    let pad = (rect.width() * 0.22).max(3.0);
    icons::paint(ui.painter(), fx::zoom(rect.shrink(pad), rect.center(), scale * bump), icon, color);
    if resp.clicked() {
        fx::tap(ctx, rect, 6.0, TEXT);
    }
}

/// Закреплено ли значение свойства: от этого зависит вид кнопки-ромба.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    /// У свойства нет закреплённых точек.
    Free,
    /// Точки есть, но не в этом моменте.
    Elsewhere,
    /// Точка стоит в этом моменте.
    Here,
}

/// Кнопка-ромб, которая закрепляет значение в текущем моменте.
pub fn key_button(ui: &mut Ui, id: egui::Id, pin: Pin) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(KEY_BUTTON, KEY_BUTTON), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let ctx = ui.ctx();
    let hover = fx::hover(ctx, &resp);
    let on = ctx.animate_bool_with_time(resp.id.with("on"), pin == Pin::Here, 0.14);
    let scale = fx::press(ctx, &resp, 0.12);
    ui.painter().rect_filled(fx::zoom(rect, rect.center(), scale), 5.0, Color32::TRANSPARENT.lerp_to_gamma(ACTIVE, hover));
    let idle = if pin == Pin::Free { TEXT3 } else { ACCENT };
    let color = idle.lerp_to_gamma(if pin == Pin::Free { TEXT } else { ACCENT }, hover);
    // Ромб подпрыгивает, когда точка появляется или пропадает.
    let bump = 1.0 + 0.6 * on * (1.0 - on);
    let glyph = fx::zoom(rect.shrink(3.5), rect.center(), scale * bump);
    if on > 0.0 {
        icons::paint_filled(ui.painter(), glyph, icons::KEY, ACCENT.gamma_multiply(on));
    }
    icons::paint(ui.painter(), glyph, icons::KEY, color);
    if resp.clicked() {
        fx::tap(ctx, rect, 5.0, ACCENT);
    }
    resp
}

/// «Пилюля»-фильтр.
pub fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::font(11.5), TEXT2);
    let size = vec2(galley.size().x + 20.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let ctx = ui.ctx();
    let hover = fx::hover(ctx, &resp);
    let on = ctx.animate_bool_with_time(resp.id.with("on"), selected, 0.14);
    let scale = fx::press(ctx, &resp, 0.07);
    let bg = RAISED.lerp_to_gamma(HOVER, hover).lerp_to_gamma(TEXT, on);
    ui.painter().rect_filled(fx::zoom(rect, rect.center(), scale), 12.0, bg);
    let pos = pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
    fx::text(ui.painter(), pos, galley, TEXT2.lerp_to_gamma(PANEL, on), rect.center(), scale);
    if resp.clicked() {
        fx::tap(ctx, rect, 12.0, TEXT);
    }
    resp
}

/// Вкладка с подчёркиванием.
pub fn tab(ui: &mut Ui, label: &str, selected: bool, width: f32, height: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let ctx = ui.ctx();
    let lit = fx::hover(ctx, &resp).max(ctx.animate_bool_with_time(resp.id.with("on"), selected, 0.14));
    let color = TEXT2.lerp_to_gamma(TEXT, lit);
    let font = if selected { theme::font_semibold(12.5) } else { theme::font(12.5) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    let pos = rect.center() - galley.size() / 2.0;
    fx::text(ui.painter(), pos, galley, color, rect.center(), fx::press(ctx, &resp, 0.06));
    if selected {
        // Подчёркивание переезжает от прежней вкладки.
        let x = ctx.animate_value_with_time(ui.id().with("tab-line"), rect.left(), 0.16);
        let line = Rect::from_min_max(pos2(x + 6.0, rect.bottom() - 2.0), pos2(x + rect.width() - 6.0, rect.bottom()));
        ui.painter().rect_filled(line, 1.0, ACCENT);
    }
    resp
}

/// Несколько подписей в одной плашке; подложка выбранной переезжает к новой.
/// Возвращает номер подписи, по которой щёлкнули.
pub fn segmented(ui: &mut Ui, id: egui::Id, labels: &[&str], selected: usize) -> Option<usize> {
    let galleys: Vec<Arc<Galley>> =
        labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_owned(), theme::font_semibold(12.5), TEXT)).collect();
    let width: f32 = galleys.iter().map(|g| g.size().x + 26.0).sum();
    let (rect, _) = ui.allocate_exact_size(vec2(width + 4.0, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, RAISED);
    let ctx = ui.ctx().clone();
    let mut cells = Vec::with_capacity(galleys.len());
    let mut x = rect.left() + 2.0;
    for g in &galleys {
        cells.push(Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(g.size().x + 26.0, 24.0)));
        x += g.size().x + 26.0;
    }
    // Положение подложки считается от края плашки: при изменении размера окна она не отстаёт.
    let target = cells[selected.min(cells.len() - 1)];
    let left = ctx.animate_value_with_time(id.with("left"), target.left() - rect.left(), 0.18);
    let wide = ctx.animate_value_with_time(id.with("wide"), target.width(), 0.18);
    ui.painter().rect_filled(Rect::from_min_size(pos2(rect.left() + left, target.top()), vec2(wide, 24.0)), 6.0, TEXT);
    let mut picked = None;
    for (i, (cell, galley)) in cells.into_iter().zip(galleys).enumerate() {
        let resp = ui.interact(cell, id.with(i), Sense::click());
        let on = ctx.animate_bool_with_time(resp.id.with("on"), i == selected, 0.16);
        let color = TEXT2.lerp_to_gamma(TEXT, fx::hover(&ctx, &resp)).lerp_to_gamma(PANEL, on);
        let pos = cell.center() - galley.size() / 2.0;
        fx::text(ui.painter(), pos, galley, color, cell.center(), fx::press(&ctx, &resp, 0.06));
        if resp.clicked() && i != selected {
            fx::tap(&ctx, cell, 6.0, TEXT);
            picked = Some(i);
        }
    }
    picked
}

/// Заголовок раздела, который сворачивается щелчком. `lit` — в разделе что-то включено.
pub fn fold(ui: &mut Ui, title: &str, open: bool, lit: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let ctx = ui.ctx();
    let hover = fx::hover(ctx, &resp);
    ui.painter().rect_filled(rect, 6.0, Color32::TRANSPARENT.lerp_to_gamma(RAISED, hover));
    let color = TEXT2.lerp_to_gamma(TEXT, hover.max(if open { 1.0 } else { 0.0 }));
    // Стрелка смотрит вниз у раскрытого раздела и вправо у свёрнутого.
    let turn = ctx.animate_bool_with_time(resp.id.with("open"), open, 0.14);
    let (center, angle) = (pos2(rect.left() + 10.0, rect.center().y), (turn - 1.0) * std::f32::consts::FRAC_PI_2);
    let arrow = [vec2(-3.5, -2.0), vec2(0.0, 2.0), vec2(3.5, -2.0)].map(|v| center + egui::emath::Rot2::from_angle(angle) * v);
    ui.painter().add(Shape::line(arrow.to_vec(), Stroke::new(1.4, color)));
    let galley = ui.painter().layout_no_wrap(title.to_owned(), theme::font_semibold(12.5), color);
    let end = rect.left() + 22.0 + galley.size().x;
    ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, color);
    let mark = ctx.animate_bool_with_time(resp.id.with("lit"), lit, 0.16);
    if mark > 0.0 {
        ui.painter().circle_filled(pos2(end + 9.0, rect.center().y), 3.0 * mark, ACCENT);
    }
    resp
}

/// Переключатель.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = if *on { ACCENT } else { BORDER };
    ui.painter().rect_filled(rect, 10.0, bg);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    let knob = if *on { ON_ACCENT } else { TEXT2 };
    ui.painter().circle_filled(pos2(x, rect.center().y), 7.0, knob);
    resp
}

/// Основная кнопка акцентного цвета с иконкой.
pub fn accent_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::font_semibold(13.0), ON_ACCENT);
    let icon_w = if icon.is_some() { 16.0 + 7.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 28.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let ctx = ui.ctx();
    let hover = fx::hover(ctx, &resp);
    let scale = fx::press(ctx, &resp, 0.05);
    let pivot = rect.center();
    let body = fx::zoom(rect, pivot, scale);
    fx::halo(ui.painter(), body, 7.0, ACCENT.gamma_multiply(0.1 + 0.22 * hover), 14.0);
    ui.painter().rect_filled(body, 7.0, ACCENT.lerp_to_gamma(Color32::WHITE, 0.18 * hover));

    // Блик пробегает по кнопке при наведении.
    let sweep = ctx.animate_bool_with_time(resp.id.with("sheen"), resp.hovered(), 0.4);
    if sweep > 0.0 && sweep < 1.0 {
        let x = egui::lerp((body.left() - 16.0)..=(body.right() + 16.0), sweep);
        let band = vec![
            pos2(x - 4.0, body.top()),
            pos2(x + 16.0, body.top()),
            pos2(x + 4.0, body.bottom()),
            pos2(x - 16.0, body.bottom()),
        ];
        let glint = Color32::WHITE.gamma_multiply(0.38 * (sweep * std::f32::consts::PI).sin());
        // Срез по бокам, чтобы блик не вылезал за скруглённые углы.
        let inner = body.shrink2(vec2(7.0, 0.0));
        ui.painter().with_clip_rect(inner).add(Shape::convex_polygon(band, glint, Stroke::NONE));
    }

    let mut x = rect.left() + 14.0;
    if let Some(icon) = icon {
        let r = Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0));
        icons::paint(ui.painter(), fx::zoom(r, pivot, scale), icon, ON_ACCENT);
        x += 16.0 + 7.0;
    }
    let pos = pos2(x, rect.center().y - galley.size().y / 2.0);
    fx::text(ui.painter(), pos, galley, ON_ACCENT, pivot, scale);
    if resp.clicked() {
        fx::tap(ctx, rect, 7.0, ACCENT);
    }
    resp
}

/// Обычная кнопка с иконкой.
pub fn soft_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::font(12.5), TEXT);
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 22.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let ctx = ui.ctx();
    let hover = fx::hover(ctx, &resp);
    let scale = fx::press(ctx, &resp, 0.05);
    let pivot = rect.center();
    let body = fx::zoom(rect, pivot, scale);
    ui.painter().rect_filled(body, 6.0, RAISED.lerp_to_gamma(HOVER, hover));
    ui.painter().rect_stroke(body, 6.0, Stroke::new(1.0, BORDER.lerp_to_gamma(TEXT3, hover)));
    let mut x = rect.left() + 11.0;
    if let Some(icon) = icon {
        let r = Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0));
        icons::paint(ui.painter(), fx::zoom(r, pivot, scale), icon, TEXT);
        x += 14.0 + 6.0;
    }
    let pos = pos2(x, rect.center().y - galley.size().y / 2.0);
    fx::text(ui.painter(), pos, galley, TEXT, pivot, scale);
    if resp.clicked() {
        fx::tap(ctx, rect, 6.0, TEXT);
    }
    resp
}

/// Полоса прогресса с бегущим бликом.
pub fn progress(ui: &mut Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let fraction = ui.ctx().animate_value_with_time(ui.id().with("progress"), fraction.clamp(0.0, 1.0), 0.2);
    ui.painter().rect_filled(rect, 5.0, RAISED);
    let fill = Rect::from_min_size(rect.min, vec2((rect.width() * fraction).max(rect.height()), rect.height()));
    fx::halo(ui.painter(), fill, 5.0, ACCENT.gamma_multiply(0.3), 10.0);
    ui.painter().rect_filled(fill, 5.0, ACCENT);
    let phase = (ui.input(|i| i.time) * 0.8).fract() as f32;
    let x = egui::lerp((fill.left() - 40.0)..=(fill.right() + 40.0), phase);
    let glint = Rect::from_min_max(pos2(x - 22.0, fill.top()), pos2(x + 22.0, fill.bottom()));
    let sheen = Shadow { offset: Vec2::ZERO, blur: 10.0, spread: 0.0, color: Color32::WHITE.gamma_multiply(0.5) };
    ui.painter().with_clip_rect(fill.shrink2(vec2(5.0, 0.0))).add(sheen.as_shape(glint, 5.0));
    ui.ctx().request_repaint();
}

/// Вертикальный разделитель в панели инструментов.
pub fn tool_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 18.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.center().x, rect.top()), pos2(rect.center().x, rect.bottom())],
        Stroke::new(1.0, BORDER),
    );
}

/// Скругление только левых углов.
pub fn round_left(r: f32) -> Rounding {
    Rounding { nw: r, sw: r, ne: 0.0, se: 0.0 }
}
