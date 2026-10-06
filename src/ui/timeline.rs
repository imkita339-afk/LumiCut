//! Нижняя панель: инструменты, дорожки, клипы, курсор воспроизведения.

use std::sync::Arc;

use eframe::egui::{
    self, epaint::RectShape, pos2, vec2, Align, Align2, Color32, CursorIcon, Layout, Painter, Pos2, Rect,
    Rounding, Sense, Shape, Stroke, TextureHandle, Ui, UiBuilder, Vec2,
};

use crate::app::{App, Drag, EffectDrag, MediaDrag, TextDrag, TimelineView, Tool, TransitionDrag};
use crate::effects::EffectKind;
use crate::text::{self, TextPreset};
use crate::ffmpeg::WAVE_RATE;
use crate::icons;
use crate::model::{Clip, Id, MediaItem, MediaKind, TrackKind};
use crate::theme::{self, *};
use crate::ui::{fx, widgets};

const TOOLBAR_H: f32 = 42.0;
const HEADER_W: f32 = 140.0;
const RULER_H: f32 = 24.0;
const ROW_GAP: f32 = 4.0;
const EDGE: f32 = 7.0;
const MIN_PPS: f32 = 2.0;
const MAX_PPS: f32 = 400.0;
/// Зазор между соседними клипами и скругление их углов.
const CLIP_GAP: f32 = 2.0;
const CLIP_ROUND: f32 = 4.0;
/// Половина ромба, которым на клипе отмечено закреплённое значение.
const KEY_MARK: f32 = 4.0;
/// С какого расстояния закреплённая точка притягивает курсор воспроизведения, пиксели.
const KEY_SNAP: f32 = 6.0;

/// То, что тащат из библиотеки на дорожку.
#[derive(Clone, Copy)]
enum Incoming {
    Media(Id),
    Effect(EffectKind),
    Text(TextPreset),
}

/// Значок перехода на стыке клипов.
fn marker_rect(x: f32, row: Rect) -> Rect {
    Rect::from_center_size(pos2(x, row.center().y), vec2(24.0, 18.0))
}

fn paint_marker(p: &Painter, pill: Rect, color: Color32, width: f32) {
    p.rect_filled(pill, 5.0, PANEL.gamma_multiply(0.92));
    p.rect_stroke(pill, 5.0, Stroke::new(width, color));
    icons::paint(p, pill.shrink2(vec2(6.0, 3.0)), icons::BOW, color);
}

fn row_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => 52.0,
        TrackKind::Audio => 38.0,
    }
}

/// Пересчёт времени в координаты и обратно.
#[derive(Clone, Copy)]
struct Geo {
    left: f32,
    pps: f32,
    scroll: f32,
}

impl Geo {
    fn x(&self, t: f64) -> f32 {
        self.left + t as f32 * self.pps - self.scroll
    }

    fn t(&self, x: f32) -> f64 {
        ((x - self.left + self.scroll) / self.pps) as f64
    }
}

/// Пружина, которой дорожка вздрагивает от разреза, удаления или упавшего клипа.
fn thud_id(track: usize) -> egui::Id {
    egui::Id::new(("thud", track))
}

fn ruler_label(t: f64, step: f64) -> String {
    let total = t.max(0.0);
    let m = (total / 60.0).floor() as u64;
    let s = total - m as f64 * 60.0;
    if step < 1.0 {
        format!("{m:02}:{s:04.1}")
    } else {
        format!("{m:02}:{:02}", s.round() as u64)
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_clip(
    ui: &Ui,
    p: &Painter,
    slot: Rect,
    vis: Rect,
    clip: &Clip,
    media: Option<&MediaItem>,
    thumb: Option<&TextureHandle>,
    wave: Option<&Arc<Vec<f32>>>,
    selected: bool,
    hot: f32,
    pps: f32,
) {
    let kind = media.map_or(MediaKind::Video, |m| m.kind);
    let (fill, rim) = match (&clip.effect, kind) {
        _ if clip.text.is_some() => (CLIP_TEXT, CLIP_TEXT_RIM),
        (Some(_), _) => (CLIP_EFFECT, CLIP_EFFECT_RIM),
        (None, MediaKind::Video) => (CLIP_VIDEO, CLIP_VIDEO_RIM),
        (None, MediaKind::Image) => (CLIP_IMAGE, CLIP_IMAGE_RIM),
        (None, MediaKind::Audio) => (CLIP_AUDIO, CLIP_AUDIO_RIM),
    };
    // У каждого клипа своя рамка, а между соседями зазор: стык виден и без наведения.
    let r = if slot.width() > 3.0 * CLIP_GAP { slot.shrink2(vec2(CLIP_GAP / 2.0, 0.0)) } else { slot };
    p.rect_filled(r, CLIP_ROUND, fill);
    let cp = p.with_clip_rect(r.intersect(vis));

    if clip.generated() {
        // У эффекта и надписи нет кадров исходника: хватает заливки и подписи.
    } else if kind != MediaKind::Audio {
        if let Some(tex) = thumb {
            let size = tex.size_vec2();
            let tile_w = (slot.height() * size.x / size.y).max(8.0);
            let first = ((vis.left() - slot.left()) / tile_w).floor().max(0.0) as usize;
            let mut x = slot.left() + first as f32 * tile_w;
            while x < r.right() && x < vis.right() {
                // Плитки идут сплошной лентой; крайние обрезаны и скруглены по рамке клипа.
                // Каждая на пиксель заходит на предыдущую, иначе сглаженные края дают полоски на швах.
                let (left, right) = ((x - 1.0).max(r.left()), (x + tile_w).min(r.right()));
                let round = |edge: bool| if edge { CLIP_ROUND } else { 0.0 };
                let (west, east) = (round(left <= r.left()), round(right >= r.right()));
                let mut tile = RectShape::filled(
                    Rect::from_min_max(pos2(left, r.top()), pos2(right, r.bottom())),
                    Rounding { nw: west, sw: west, ne: east, se: east },
                    Color32::from_gray(200),
                );
                tile.fill_texture_id = tex.id();
                tile.uv = Rect::from_min_max(pos2((left - x) / tile_w, 0.0), pos2((right - x) / tile_w, 1.0));
                cp.add(tile);
                x += tile_w;
            }
        }
    } else if let Some(peaks) = wave {
        let h = (r.height() - 16.0).max(6.0);
        let cy = r.bottom() - h / 2.0 - 1.0;
        let x0 = r.left().max(vis.left());
        let x1 = r.right().min(vis.right());
        let per_px = clip.speed * WAVE_RATE / pps as f64;
        let span = ((2.0 * per_px).ceil() as usize).clamp(1, 256);
        let mut x = x0;
        while x < x1 {
            let t_src = clip.offset + ((x - slot.left()) / pps) as f64 * clip.speed;
            let idx = (t_src * WAVE_RATE) as usize;
            let peak = peaks.iter().skip(idx).take(span).fold(0.0f32, |a, b| a.max(*b));
            let a = (peak.sqrt() * h / 2.0).max(0.5);
            cp.line_segment([pos2(x, cy - a), pos2(x, cy + a)], Stroke::new(1.0, WAVE));
            x += 2.0;
        }
    }

    // Подпись.
    let name = match (&clip.effect, media) {
        _ if clip.text.is_some() => clip.text.as_ref().map_or("", |t| t.title()),
        (Some(effect), _) => effect.kind.label(),
        (None, Some(m)) => m.name.as_str(),
        (None, None) => "файл не найден",
    };
    let label = if (clip.speed - 1.0).abs() > 1e-3 {
        format!("{name} · {}×", (clip.speed * 100.0).round() / 100.0)
    } else {
        name.to_owned()
    };
    let pad = if selected { 14.0 } else { 6.0 };
    let max_w = r.width() - pad * 2.0;
    if max_w > 24.0 {
        let color = if kind == MediaKind::Audio { Color32::from_rgb(0xC9, 0xF2, 0xE6) } else { Color32::WHITE };
        let galley = widgets::elided(ui, &label, theme::font(10.5), color, max_w - 10.0);
        let left = (r.left() + pad).max(vis.left() + 2.0).min(r.right() - pad - galley.size().x - 10.0);
        let bg = Rect::from_min_size(pos2(left, r.top() + 4.0), galley.size() + vec2(10.0, 3.0));
        if kind != MediaKind::Audio {
            cp.rect_filled(bg, 3.0, CLIP_LABEL_BG);
        }
        cp.galley(bg.min + vec2(5.0, 1.5), galley, color);
    }
    // Значки в углу клипа: движение по пути и оформление (цвет, хромакей, маска, эффекты).
    let moving = clip.motion.as_ref().map_or(false, |m| m.is_active());
    let mut badge_x = r.right() - pad - 8.0;
    for (shown, icon) in [(moving, icons::PATH), (clip.styled(), icons::WAND)] {
        if shown && badge_x - 8.0 > r.left() + pad && max_w > 44.0 {
            let badge = Rect::from_center_size(pos2(badge_x, r.bottom() - 11.0), vec2(16.0, 16.0));
            cp.rect_filled(badge, 4.0, CLIP_LABEL_BG);
            icons::paint(&cp, badge.shrink(2.5), icon, Color32::WHITE);
            badge_x -= 19.0;
        }
    }

    if hot > 0.0 {
        p.rect_filled(r, CLIP_ROUND, Color32::WHITE.gamma_multiply(0.07 * hot));
    }
    p.rect_stroke(r.shrink(0.5), CLIP_ROUND, Stroke::new(1.0, rim));
    if selected {
        p.rect_stroke(r.shrink(1.0), CLIP_ROUND, Stroke::new(2.0, ACCENT));
        for x in [r.left(), r.right() - 9.0] {
            let h = Rect::from_min_size(pos2(x, r.top()), vec2(9.0, r.height()));
            cp.rect_filled(h, 0.0, ACCENT);
            cp.line_segment(
                [pos2(h.center().x, h.center().y - 7.0), pos2(h.center().x, h.center().y + 7.0)],
                Stroke::new(2.0, ON_ACCENT),
            );
        }
    }

    // Закреплённые значения: ромбы вдоль нижнего края. Точки разных свойств в одном моменте — один ромб.
    if r.width() > 2.0 * KEY_MARK + 2.0 {
        let mut last = f32::NEG_INFINITY;
        for t in clip.keys.times() {
            if t < 0.0 || t > clip.duration {
                continue;
            }
            // Крайние точки сдвинуты внутрь, чтобы ромб не обрезался краем клипа.
            let x = (slot.left() + t as f32 * pps).clamp(r.left() + KEY_MARK + 1.0, r.right() - KEY_MARK - 1.0);
            if x - last < 1.0 {
                continue;
            }
            last = x;
            let c = pos2(x, r.bottom() - KEY_MARK - 4.0);
            let diamond = [vec2(0.0, -KEY_MARK), vec2(KEY_MARK, 0.0), vec2(0.0, KEY_MARK), vec2(-KEY_MARK, 0.0)];
            cp.add(Shape::convex_polygon(diamond.map(|v| c + v).to_vec(), Color32::WHITE, Stroke::new(1.0, ON_ACCENT)));
        }
    }
}

impl App {
    fn zoom_timeline(&mut self, factor: f32) {
        let old = self.tl.pps;
        let new = (old * factor).clamp(MIN_PPS, MAX_PPS);
        let px = self.playhead as f32 * old - self.tl.scroll;
        self.tl.pps = new;
        self.tl.scroll = (self.playhead as f32 * new - px).max(0.0);
    }

    fn snap_points(&self, ignore: Option<Id>) -> Vec<f64> {
        let mut v = vec![0.0, self.playhead];
        for t in &self.project.tracks {
            for c in &t.clips {
                if Some(c.id) != ignore {
                    v.push(c.start);
                    v.push(c.end());
                }
            }
        }
        v
    }

    /// Привязка одной точки времени; второе значение — точка, к которой притянуло.
    fn snap_time(&self, t: f64, ignore: Option<Id>) -> (f64, Option<f64>) {
        if !self.tl.snap {
            return (t, None);
        }
        let mut best = (8.0 / self.tl.pps as f64, t, None);
        for p in self.snap_points(ignore) {
            let d = (p - t).abs();
            if d < best.0 {
                best = (d, p, Some(p));
            }
        }
        (best.1, best.2)
    }

    /// Привязка клипа началом или концом; второе значение — точка, к которой притянуло.
    fn snap_clip(&self, start: f64, dur: f64, ignore: Option<Id>) -> (f64, Option<f64>) {
        if !self.tl.snap {
            return (start, None);
        }
        let mut best = (8.0 / self.tl.pps as f64, start, None);
        for p in self.snap_points(ignore) {
            for (edge, cand) in [(start, p), (start + dur, p - dur)] {
                let d = (edge - p).abs();
                if d < best.0 && cand >= 0.0 {
                    best = (d, cand, Some(p));
                }
            }
        }
        (best.1, best.2)
    }

    /// Закреплённая точка выбранного клипа рядом с моментом `t`: к ней притягивается курсор
    /// воспроизведения, чтобы на точку было легко вернуться и поправить её.
    fn snap_to_key(&self, t: f64) -> Option<f64> {
        if !self.tl.snap {
            return None;
        }
        let clip = self.project.clip(self.selected?)?;
        let reach = (KEY_SNAP / self.tl.pps) as f64;
        clip.keys
            .times()
            .into_iter()
            .filter(|k| *k >= 0.0 && *k <= clip.duration)
            .map(|k| clip.start + k)
            .filter(|k| (k - t).abs() <= reach)
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
    }

    /// Запоминает точку привязки; искры — в момент, когда край «прилип».
    fn snap_feedback(&mut self, hit: Option<f64>, geo: Geo, y: f32, area: Rect, spark: bool) {
        if let Some(t) = hit.filter(|_| spark && hit != self.tl.snapped) {
            let at = pos2(geo.x(t), y);
            fx::sparks(&self.ctx, area, at, vec2(0.0, -170.0), 0.7, 5, ACCENT);
            fx::sparks(&self.ctx, area, at, vec2(0.0, 170.0), 0.7, 5, ACCENT);
        }
        self.tl.snapped = hit;
    }

    /// Отрезок дорожки на экране по геометрии последнего кадра (и область, которой он обрезан).
    fn span_rect(&self, track: usize, from: f64, to: f64) -> Option<(Rect, Rect)> {
        let view = self.tl.view.as_ref()?;
        let row = view.rows.iter().find(|(i, _)| *i == track)?.1;
        let r = Rect::from_min_max(pos2(view.x(from), row.top()), pos2(view.x(to), row.bottom()));
        Some((view.area, r))
    }

    /// Клип лёг на дорожку: вспышка и толчок.
    pub fn fx_land(&self, id: Id, force: f32) {
        let Some((ti, ci)) = self.project.find_clip(id) else { return };
        let c = &self.project.tracks[ti].clips[ci];
        if let Some((area, r)) = self.span_rect(ti, c.start, c.end()) {
            fx::flash(&self.ctx, area, r, 4.0, Color32::WHITE.gamma_multiply(0.4 * force));
            fx::kick(&self.ctx, thud_id(ti), 140.0 * force);
        }
    }

    /// Разрез клипа: росчерк, искры и толчок дорожки.
    pub fn fx_cut(&self, id: Id, t: f64) {
        let Some((ti, _)) = self.project.find_clip(id) else { return };
        let Some((area, r)) = self.span_rect(ti, t, t) else { return };
        let (top, bottom) = (r.center_top(), r.center_bottom());
        fx::slash(&self.ctx, area, top - vec2(0.0, 9.0), bottom + vec2(0.0, 9.0), ACCENT);
        fx::sparks(&self.ctx, area, top, vec2(0.0, -320.0), 1.2, 12, ACCENT);
        fx::sparks(&self.ctx, area, bottom, vec2(0.0, 260.0), 1.2, 12, ACCENT);
        fx::kick(&self.ctx, thud_id(ti), 110.0);
    }

    /// Кусок дорожки исчезает: схлопывание и осколки.
    pub fn fx_vanish(&self, track: usize, from: f64, to: f64) {
        let Some((area, r)) = self.span_rect(track, from, to) else { return };
        let fill = match self.project.tracks[track].kind {
            TrackKind::Video => CLIP_VIDEO,
            TrackKind::Audio => CLIP_AUDIO,
        };
        fx::poof(&self.ctx, area, r, 4.0, fill.lerp_to_gamma(Color32::WHITE, 0.45));
        let seen = r.intersect(area);
        if seen.is_positive() {
            let count = ((seen.width() / 12.0) as usize).clamp(6, 30);
            fx::burst(&self.ctx, area, seen, count, DANGER);
        }
        fx::kick(&self.ctx, thud_id(track), -90.0);
    }

    /// Отмена и повтор: лёгкая вспышка по дорожкам.
    pub fn fx_sweep(&self) {
        let Some(view) = &self.tl.view else { return };
        for (_, row) in &view.rows {
            fx::flash(&self.ctx, view.area, *row, 4.0, Color32::WHITE.gamma_multiply(0.07));
        }
    }

    fn hit_clip(&self, rows: &[(usize, Rect)], geo: Geo, pos: Pos2) -> Option<(Id, usize, Rect)> {
        for (ti, row) in rows {
            if pos.y < row.top() || pos.y > row.bottom() {
                continue;
            }
            for c in self.project.tracks[*ti].clips.iter().rev() {
                let r = Rect::from_min_max(pos2(geo.x(c.start), row.top()), pos2(geo.x(c.end()), row.bottom()));
                if pos.x >= r.left() && pos.x <= r.right() {
                    return Some((c.id, *ti, r));
                }
            }
        }
        None
    }

    fn timeline_toolbar(&mut self, ui: &mut Ui, rect: Rect) {
        let inner = rect.shrink2(vec2(8.0, 0.0));
        ui.allocate_new_ui(
            UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let select = self.tl.tool == Tool::Select;
                if widgets::icon_button(ui, icons::CURSOR, 30.0, "Выделение (V)", select, true).clicked() {
                    self.tl.tool = Tool::Select;
                }
                if widgets::icon_button(ui, icons::BLADE, 30.0, "Лезвие (B)", !select, true).clicked() {
                    self.tl.tool = Tool::Blade;
                }
                widgets::tool_separator(ui);
                let can_undo = self.history.can_undo();
                if widgets::icon_button(ui, icons::UNDO, 30.0, "Отменить (Ctrl+Z)", false, can_undo).clicked() {
                    self.undo();
                }
                let can_redo = self.history.can_redo();
                if widgets::icon_button(ui, icons::REDO, 30.0, "Повторить (Ctrl+Shift+Z)", false, can_redo).clicked() {
                    self.redo();
                }
                widgets::tool_separator(ui);
                let has_clips = !self.project.is_empty();
                if widgets::icon_button(ui, icons::SPLIT, 30.0, "Разделить по курсору (Ctrl+B)", false, has_clips).clicked() {
                    self.split_at_playhead();
                }
                if widgets::icon_button(ui, icons::TRIM_LEFT, 30.0, "Обрезать слева от курсора (Q)", false, has_clips).clicked() {
                    self.trim_at_playhead(true);
                }
                if widgets::icon_button(ui, icons::TRIM_RIGHT, 30.0, "Обрезать справа от курсора (W)", false, has_clips).clicked() {
                    self.trim_at_playhead(false);
                }
                let (what, has_sel) = self.deletable();
                if widgets::icon_button(ui, icons::TRASH, 30.0, &format!("{what} (Del)"), false, has_sel).clicked() {
                    self.delete_selected();
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::icon_button(ui, icons::FIT, 30.0, "Показать весь проект", false, true).clicked() {
                        self.tl.fit = true;
                    }
                    if widgets::icon_button(ui, icons::ZOOM_IN, 30.0, "Крупнее", false, true).clicked() {
                        self.zoom_timeline(1.3);
                    }
                    ui.spacing_mut().slider_width = 96.0;
                    let mut pps = self.tl.pps;
                    let slider = egui::Slider::new(&mut pps, MIN_PPS..=MAX_PPS).logarithmic(true).show_value(false);
                    if ui.add(slider).on_hover_text("Масштаб таймлайна").changed() {
                        self.zoom_timeline(pps / self.tl.pps);
                    }
                    if widgets::icon_button(ui, icons::ZOOM_OUT, 30.0, "Мельче", false, true).clicked() {
                        self.zoom_timeline(1.0 / 1.3);
                    }
                    widgets::tool_separator(ui);
                    if widgets::icon_button(ui, icons::MAGNET, 30.0, "Привязка к краям клипов и курсору", self.tl.snap, true).clicked() {
                        self.tl.snap = !self.tl.snap;
                    }
                });
            },
        );
    }

    pub fn timeline_ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let toolbar = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
        self.timeline_toolbar(ui, toolbar);
        ui.painter().line_segment([toolbar.left_bottom(), toolbar.right_bottom()], Stroke::new(1.0, LINE));

        let body = Rect::from_min_max(pos2(rect.left(), toolbar.bottom() + 1.0), rect.max);
        let headers = Rect::from_min_size(body.min, vec2(HEADER_W, body.height()));
        ui.painter().line_segment([headers.right_top(), headers.right_bottom()], Stroke::new(1.0, LINE));
        let lanes = Rect::from_min_max(pos2(headers.right() + 8.0, body.top()), pos2(body.right() - 8.0, body.bottom() - 6.0));
        let ruler = Rect::from_min_size(lanes.min, vec2(lanes.width(), RULER_H));

        let mut rows: Vec<(usize, Rect)> = Vec::new();
        let mut y = ruler.bottom() + ROW_GAP;
        for (i, t) in self.project.tracks.iter().enumerate() {
            // Дорожке субтитров хватает высоты подписи: так все дорожки помещаются в панель.
            let h = if t.captions { row_height(TrackKind::Audio) } else { row_height(t.kind) };
            rows.push((i, Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), h))));
            y += h + ROW_GAP;
        }

        // Масштаб и прокрутка.
        if self.tl.fit {
            let d = self.project.duration().max(5.0) as f32;
            self.tl.pps = (lanes.width() * 0.92 / d).clamp(MIN_PPS, MAX_PPS);
            self.tl.scroll = 0.0;
            self.tl.fit = false;
        }
        if ui.rect_contains_pointer(body) {
            let (scroll, zoom, ptr) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            if zoom != 1.0 {
                let px = ptr.map_or(0.0, |p| p.x - lanes.left());
                let t = (px + self.tl.scroll) / self.tl.pps;
                self.tl.pps = (self.tl.pps * zoom).clamp(MIN_PPS, MAX_PPS);
                self.tl.scroll = t * self.tl.pps - px;
            } else if scroll != Vec2::ZERO {
                self.tl.scroll -= scroll.x + scroll.y;
            }
        }
        let content_w = self.project.duration() as f32 * self.tl.pps;
        let max_scroll = (content_w - lanes.width() * 0.5).max(0.0);
        if self.playing {
            let x = self.playhead as f32 * self.tl.pps - self.tl.scroll;
            if x > lanes.width() - 24.0 || x < 0.0 {
                self.tl.scroll = self.playhead as f32 * self.tl.pps - lanes.width() * 0.1;
            }
        }
        self.tl.scroll = self.tl.scroll.clamp(0.0, max_scroll);
        let geo = Geo { left: lanes.left(), pps: self.tl.pps, scroll: self.tl.scroll };
        let area = Rect::from_min_max(lanes.min, pos2(lanes.right(), body.bottom()));
        self.tl.view = Some(TimelineView {
            left: geo.left,
            pps: geo.pps,
            scroll: geo.scroll,
            ruler,
            area,
            rows: rows.clone(),
        });

        // Значки переходов на стыках: клип справа от стыка, дорожка и место значка.
        let mut markers: Vec<(Id, usize, Rect)> = Vec::new();
        for (ti, row) in &rows {
            for (owner, t) in self.project.junctions(*ti) {
                if self.project.clip(owner).map_or(false, |c| c.transition.is_some()) {
                    markers.push((owner, *ti, marker_rect(geo.x(t), *row)));
                }
            }
        }

        // Заголовки дорожек.
        ui.painter().text(
            pos2(headers.left() + 12.0, ruler.center().y),
            Align2::LEFT_CENTER,
            "Дорожки",
            theme::font(11.0),
            TEXT3,
        );
        for (i, row) in &rows {
            let (name, kind, locked, enabled) = {
                let t = &self.project.tracks[*i];
                (t.name.clone(), t.kind, t.locked, t.enabled)
            };
            let icon = if kind == TrackKind::Video { icons::FILM } else { icons::NOTE };
            let icon_rect = Rect::from_center_size(pos2(headers.left() + 18.0, row.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), icon_rect, icon, TEXT3);
            ui.painter().text(
                pos2(headers.left() + 32.0, row.center().y),
                Align2::LEFT_CENTER,
                &name,
                theme::font(11.5),
                if enabled { TEXT2 } else { TEXT3 },
            );
            let toggle = Rect::from_center_size(pos2(headers.right() - 16.0, row.center().y), vec2(22.0, 22.0));
            let lock = Rect::from_center_size(pos2(headers.right() - 40.0, row.center().y), vec2(22.0, 22.0));
            let (lock_icon, lock_tip) = if locked {
                (icons::LOCK, "Разблокировать дорожку")
            } else {
                (icons::UNLOCK, "Заблокировать дорожку")
            };
            if widgets::icon_button_at(ui, lock, ("lock", *i), lock_icon, lock_tip, locked).clicked() {
                self.commit();
                self.project.tracks[*i].locked = !locked;
                self.touch();
            }
            let (t_icon, t_tip) = match (kind, enabled) {
                (TrackKind::Video, true) => (icons::EYE, "Скрыть дорожку"),
                (TrackKind::Video, false) => (icons::EYE_OFF, "Показать дорожку"),
                (TrackKind::Audio, true) => (icons::VOLUME, "Выключить звук дорожки"),
                (TrackKind::Audio, false) => (icons::VOLUME_OFF, "Включить звук дорожки"),
            };
            if widgets::icon_button_at(ui, toggle, ("enabled", *i), t_icon, t_tip, !enabled).clicked() {
                self.commit();
                self.project.tracks[*i].enabled = !enabled;
                self.touch();
            }
        }

        // Взаимодействие с областью дорожек.
        let resp = ui.interact(area, ui.id().with("lanes"), Sense::click_and_drag());
        let (pointer, press, primary_down) =
            ui.input(|i| (i.pointer.interact_pos(), i.pointer.press_origin(), i.pointer.primary_down()));
        let fps = self.project.fps.max(1.0);
        let to_frame = |t: f64| (t.max(0.0) * fps).round() / fps;

        if resp.is_pointer_button_down_on() && primary_down && self.tl.drag.is_none() {
            if let Some(o) = press {
                if o.y <= ruler.bottom() {
                    self.tl.drag = Some(Drag::Scrub);
                }
            }
        }
        if resp.drag_started() && self.tl.drag.is_none() && primary_down {
            if let Some(o) = press {
                match self.hit_clip(&rows, geo, o) {
                    Some((id, ti, r)) if self.tl.tool == Tool::Select && !self.project.tracks[ti].locked => {
                        self.selected = Some(id);
                        self.commit();
                        let wide = r.width() > EDGE * 3.0;
                        let (start, end) = self.project.clip(id).map_or((0.0, 0.0), |c| (c.start, c.end()));
                        let at = geo.t(o.x);
                        self.tl.drag = Some(if wide && o.x - r.left() <= EDGE {
                            Drag::TrimStart { clip: id, grab: at - start }
                        } else if wide && r.right() - o.x <= EDGE {
                            Drag::TrimEnd { clip: id, grab: at - end }
                        } else {
                            Drag::Move { clip: id, grab: at - start }
                        });
                    }
                    Some(_) => {}
                    None => self.tl.drag = Some(Drag::Scrub),
                }
            }
        }

        let drag = self.tl.drag;
        // В кадре захвата клип ещё не двигали — искры привязки ни к чему.
        let spark = !resp.drag_started();
        if let (Some(drag), Some(pos), true) = (drag, pointer, primary_down) {
            match drag {
                Drag::Scrub => {
                    let t = geo.t(pos.x);
                    self.seek(self.snap_to_key(t).unwrap_or_else(|| to_frame(t)));
                }
                Drag::Move { clip, grab } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    if let Some((ti, ci)) = self.project.find_clip(clip) {
                        let dur = self.project.tracks[ti].clips[ci].duration;
                        let (t, hit) = self.snap_clip((geo.t(pos.x) - grab).max(0.0), dur, Some(clip));
                        self.snap_feedback(hit, geo, pos.y, area, spark);
                        let target = rows
                            .iter()
                            .find(|(_, r)| pos.y >= r.top() - ROW_GAP / 2.0 && pos.y <= r.bottom() + ROW_GAP / 2.0)
                            .map(|(i, _)| *i)
                            .filter(|i| {
                                let fits = self.project.clip_fits(&self.project.tracks[ti].clips[ci], *i);
                                fits && !self.project.tracks[*i].locked
                            })
                            .unwrap_or(ti);
                        if target != ti {
                            let c = self.project.tracks[ti].clips.remove(ci);
                            self.project.tracks[target].clips.push(c);
                        }
                        if let Some(c) = self.project.clip_mut(clip) {
                            c.start = t;
                        }
                        self.dirty = true;
                    }
                }
                Drag::TrimStart { clip, grab } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let (t, hit) = self.snap_time(geo.t(pos.x) - grab, Some(clip));
                    self.project.trim_start(clip, t);
                    // Край мог упереться в соседа или в начало исходника и не дойти до точки.
                    let reached = self.project.clip(clip).map_or(false, |c| (c.start - t).abs() < 1e-6);
                    self.snap_feedback(hit.filter(|_| reached), geo, pos.y, area, spark);
                    self.dirty = true;
                }
                Drag::TrimEnd { clip, grab } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let (t, hit) = self.snap_time(geo.t(pos.x) - grab, Some(clip));
                    self.project.trim_end(clip, t);
                    let reached = self.project.clip(clip).map_or(false, |c| (c.end() - t).abs() < 1e-6);
                    self.snap_feedback(hit.filter(|_| reached), geo, pos.y, area, spark);
                    self.dirty = true;
                }
            }
        }
        if !primary_down {
            self.tl.snapped = None;
            if let Some(drag) = self.tl.drag.take() {
                match drag {
                    Drag::Move { clip, .. } => {
                        if let Some((ti, ci)) = self.project.find_clip(clip) {
                            let start = self.project.tracks[ti].clips[ci].start;
                            self.project.move_clip(clip, ti, start);
                        }
                        self.touch();
                        self.fx_land(clip, 0.6);
                    }
                    Drag::TrimStart { .. } | Drag::TrimEnd { .. } => self.touch(),
                    Drag::Scrub => {}
                }
            }
        }

        // Значок перехода под указателем (лезвие режет клипы и сквозь значок).
        let marker_at = |pos: Pos2| markers.iter().find(|m| m.2.contains(pos)).map(|m| m.0);
        let pick = self.tl.tool == Tool::Select;
        if resp.secondary_clicked() {
            if let Some(pos) = pointer {
                self.sel_transition = marker_at(pos);
                self.selected = match self.sel_transition {
                    Some(_) => None,
                    None => self.hit_clip(&rows, geo, pos).map(|h| h.0),
                };
            }
        }
        if resp.clicked() {
            if let Some(pos) = pointer {
                if pos.y > ruler.bottom() {
                    self.sel_transition = marker_at(pos).filter(|_| pick);
                    match self.hit_clip(&rows, geo, pos) {
                        Some(_) if self.sel_transition.is_some() => self.selected = None,
                        Some((id, ti, _)) => {
                            if self.tl.tool == Tool::Blade && !self.project.tracks[ti].locked {
                                self.split_clip(id, to_frame(geo.t(pos.x)));
                            } else {
                                self.selected = Some(id);
                            }
                        }
                        None => {
                            self.selected = None;
                            self.seek(to_frame(geo.t(pos.x)));
                        }
                    }
                }
            }
        }
        resp.context_menu(|ui| {
            ui.set_min_width(210.0);
            if ui.add(egui::Button::new("Разделить по курсору").shortcut_text("Ctrl+B")).clicked() {
                ui.close_menu();
                self.split_at_playhead();
            }
            let (what, any) = self.deletable();
            let del = egui::Button::new(what).shortcut_text("Del");
            if ui.add_enabled(any, del).clicked() {
                ui.close_menu();
                self.delete_selected();
            }
            self.clip_commands(ui);
            ui.separator();
            if ui.button("Импорт медиа…").clicked() {
                ui.close_menu();
                self.import_dialog();
            }
        });

        // Курсор мыши над краями клипа и для лезвия; клип под указателем подсвечивается.
        let mut hot: Option<Id> = None;
        let mut blade: Option<(f32, Rect)> = None;
        if self.tl.drag.is_none() {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()).filter(|p| area.contains(*p) && p.y > ruler.bottom()) {
                if pick && resp.hovered() && marker_at(pos).is_some() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                } else if let Some((id, ti, r)) = self.hit_clip(&rows, geo, pos) {
                    if !self.project.tracks[ti].locked {
                        let over = resp.hovered();
                        hot = over.then_some(id);
                        if self.tl.tool == Tool::Blade {
                            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                            blade = over.then_some((geo.x(to_frame(geo.t(pos.x))), r));
                        } else if r.width() > EDGE * 3.0 && (pos.x - r.left() <= EDGE || r.right() - pos.x <= EDGE) {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                        }
                    }
                }
            }
        }

        // Отрисовка.
        let p = ui.painter().with_clip_rect(area);

        // Линейка.
        const STEPS: [f64; 15] = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0];
        let step = STEPS.iter().copied().find(|s| *s as f32 * geo.pps >= 84.0).unwrap_or(3600.0);
        let mut n = (geo.t(lanes.left()) / step).floor().max(0.0) as i64;
        loop {
            let t = n as f64 * step;
            let x = geo.x(t);
            if x > lanes.right() {
                break;
            }
            p.line_segment([pos2(x, ruler.top() + 4.0), pos2(x, ruler.bottom())], Stroke::new(1.0, BORDER));
            p.text(pos2(x + 4.0, ruler.top() + 12.0), Align2::LEFT_CENTER, ruler_label(t, step), theme::font_mono(10.5), TEXT3);
            for k in 1..5 {
                let xm = geo.x(t + step * k as f64 / 5.0);
                p.line_segment([pos2(xm, ruler.bottom() - 5.0), pos2(xm, ruler.bottom())], Stroke::new(1.0, LINE));
            }
            n += 1;
        }

        // Дорожки и клипы.
        let dragged = match drag {
            Some(Drag::Move { clip, .. }) => Some(clip),
            _ => None,
        };
        let picked = fx::changed(ui.ctx(), ui.id().with("picked"), self.selected);
        let mut on_top: Option<(Rect, Clip)> = None;
        for (ti, row) in &rows {
            let track = &self.project.tracks[*ti];
            p.rect_filled(*row, 4.0, RAIL);
            let thud = vec2(0.0, fx::spring(ui.ctx(), thud_id(*ti), 0.0, 0.0));
            for clip in &track.clips {
                let r = Rect::from_min_max(pos2(geo.x(clip.start), row.top()), pos2(geo.x(clip.end()), row.bottom()));
                if r.right() < lanes.left() || r.left() > lanes.right() {
                    continue;
                }
                if dragged == Some(clip.id) {
                    on_top = Some((r, clip.clone()));
                    continue;
                }
                let selected = self.selected == Some(clip.id);
                if selected && picked {
                    fx::pulse(ui.ctx(), area, r, 4.0, ACCENT);
                }
                let lit = ui.ctx().animate_bool_with_time(egui::Id::new(("clip-hot", clip.id)), hot == Some(clip.id), 0.1);
                paint_clip(
                    ui,
                    &p,
                    r.translate(thud),
                    area,
                    clip,
                    self.project.media(clip.media),
                    self.lib.thumbs.get(&clip.media),
                    self.lib.waves.get(&clip.media),
                    selected,
                    lit,
                    geo.pps,
                );
            }
            if !track.enabled || track.locked {
                p.rect_filled(*row, 4.0, Color32::from_black_alpha(if track.enabled { 60 } else { 120 }));
            }
        }
        // Клип в руке приподнят над дорожкой и отбрасывает тень.
        let lift = ui.ctx().animate_bool_with_time(ui.id().with("lift"), on_top.is_some(), 0.12);
        if let Some((r, clip)) = &on_top {
            let r = r.translate(vec2(0.0, -3.0 * lift));
            fx::halo(&p, r.translate(vec2(0.0, 6.0)), 4.0, Color32::from_black_alpha((170.0 * lift) as u8), 16.0);
            paint_clip(
                ui,
                &p,
                r,
                area,
                clip,
                self.project.media(clip.media),
                self.lib.thumbs.get(&clip.media),
                self.lib.waves.get(&clip.media),
                true,
                0.0,
                geo.pps,
            );
        }

        // Переходы: полоса на всю длительность и значок на стыке.
        for (owner, ti, pill) in &markers {
            let Some(clip) = self.project.clip(*owner) else { continue };
            let Some((_, half)) = self.project.transition_in(*ti, clip) else { continue };
            let reach = half as f32 * geo.pps;
            let band = Rect::from_min_max(
                pos2(pill.center().x - reach, pill.center().y - row_height(TrackKind::Video) / 2.0),
                pos2(pill.center().x + reach, pill.center().y + row_height(TrackKind::Video) / 2.0),
            );
            p.rect_filled(band, CLIP_ROUND, Color32::WHITE.gamma_multiply(0.09));
            let chosen = self.sel_transition == Some(*owner);
            paint_marker(&p, *pill, if chosen { ACCENT } else { TEXT }, if chosen { 1.5 } else { 1.0 });
        }

        // Переход из библиотеки: все стыки подсвечены, ближайший к указателю примет его.
        if let Some(kind) = egui::DragAndDrop::payload::<TransitionDrag>(ui.ctx()).map(|d| d.0) {
            let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|pos| body.contains(*pos));
            let mut nearest: Option<(Id, f32, Rect)> = None;
            for (ti, row) in &rows {
                if self.project.tracks[*ti].locked {
                    continue;
                }
                for (owner, t) in self.project.junctions(*ti) {
                    let pill = marker_rect(geo.x(t), *row);
                    paint_marker(&p, pill, ACCENT.gamma_multiply(0.75), 1.0);
                    let Some(pos) = pointer else { continue };
                    let d = (pos.x - pill.center().x).abs();
                    let on_row = pos.y >= row.top() - ROW_GAP / 2.0 && pos.y <= row.bottom() + ROW_GAP / 2.0;
                    if on_row && d <= 60.0 && nearest.map_or(true, |n| d < n.1) {
                        nearest = Some((owner, d, pill));
                    }
                }
            }
            let released = ui.input(|i| i.pointer.any_released());
            if let Some((owner, _, pill)) = nearest {
                let beat = 0.5 + 0.5 * (ui.input(|i| i.time) * 7.0).sin() as f32;
                fx::halo(&p, pill, 5.0, ACCENT.gamma_multiply(0.35 + 0.25 * beat), 12.0);
                paint_marker(&p, pill.expand(2.0), ACCENT, 1.5);
                ui.ctx().request_repaint();
                if released {
                    self.apply_transition(owner, kind);
                    egui::DragAndDrop::clear_payload(ui.ctx());
                }
            } else if released && pointer.is_some() {
                self.toast("Переход ставится на стык клипов, стоящих вплотную");
            }
        }

        // Линия привязки и намеченный разрез.
        let lanes_bottom = rows.last().map_or(ruler.bottom(), |(_, r)| r.bottom());
        if let Some(t) = self.tl.snapped {
            let ends = [pos2(geo.x(t), ruler.bottom()), pos2(geo.x(t), lanes_bottom)];
            p.line_segment(ends, Stroke::new(5.0, ACCENT.gamma_multiply(0.16)));
            p.line_segment(ends, Stroke::new(1.0, ACCENT));
        }
        if let Some((x, r)) = blade {
            let ends = [pos2(x, r.top() - 3.0), pos2(x, r.bottom() + 3.0)];
            p.line_segment(ends, Stroke::new(5.0, Color32::WHITE.gamma_multiply(0.14)));
            p.extend(Shape::dashed_line(&ends, Stroke::new(1.0, Color32::WHITE), 4.0, 3.0));
        }

        // Перетаскивание файла, эффекта или надписи из библиотеки.
        let incoming = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx())
            .map(|d| Incoming::Media(d.0))
            .or_else(|| egui::DragAndDrop::payload::<EffectDrag>(ui.ctx()).map(|d| Incoming::Effect(d.0)))
            .or_else(|| egui::DragAndDrop::payload::<TextDrag>(ui.ctx()).map(|d| Incoming::Text(d.0)));
        if let Some(incoming) = incoming {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()).filter(|p| body.contains(*p)) {
                let usable = |i: usize| {
                    let fits = match incoming {
                        Incoming::Media(media) => self.project.compatible(media, i),
                        Incoming::Effect(_) | Incoming::Text(_) => {
                            self.project.tracks[i].kind == TrackKind::Video && !self.project.tracks[i].captions
                        }
                    };
                    fits && !self.project.tracks[i].locked
                };
                let hovered = rows
                    .iter()
                    .find(|(i, r)| pos.y >= r.top() - ROW_GAP / 2.0 && pos.y <= r.bottom() + ROW_GAP / 2.0 && usable(*i))
                    .copied();
                // Видео по умолчанию ложится на нижнюю видеодорожку, звук и эффекты — на верхнюю из своих.
                let to_main = match incoming {
                    Incoming::Media(media) => self.project.media(media).map_or(false, |m| m.kind != MediaKind::Audio),
                    Incoming::Effect(_) | Incoming::Text(_) => false,
                };
                let fallback = if to_main {
                    rows.iter().rev().find(|(i, _)| usable(*i)).copied()
                } else {
                    rows.iter().find(|(i, _)| usable(*i)).copied()
                };
                if let Some((ti, row)) = hovered.or(fallback) {
                    let dur = match incoming {
                        Incoming::Media(media) => self.project.default_duration(media),
                        Incoming::Effect(kind) => kind.duration(),
                        Incoming::Text(_) => text::DURATION,
                    };
                    let (t, _) = self.snap_clip(geo.t(pos.x.max(lanes.left())).max(0.0), dur, None);
                    let start = self.project.free_start(ti, None, t, dur);
                    let ghost = Rect::from_min_max(pos2(geo.x(start), row.top()), pos2(geo.x(start + dur), row.bottom()));
                    // Место посадки «дышит», пока файл держат над дорожкой.
                    let beat = 0.5 + 0.5 * (ui.input(|i| i.time) * 7.0).sin() as f32;
                    p.rect_filled(ghost, 4.0, ACCENT.gamma_multiply(0.18 + 0.12 * beat));
                    p.rect_stroke(ghost, 4.0, Stroke::new(1.5, ACCENT));
                    ui.ctx().request_repaint();
                    if ui.input(|i| i.pointer.any_released()) {
                        self.commit();
                        let added = match incoming {
                            Incoming::Media(media) => self.project.add_clip(ti, media, start),
                            Incoming::Effect(kind) => self.project.add_effect(ti, kind, start),
                            Incoming::Text(preset) => {
                                let text = preset.text(self.project.height);
                                self.project.add_text(ti, text, start, text::DURATION)
                            }
                        };
                        match (added, incoming) {
                            (Some(id), Incoming::Effect(_)) => self.effect_added(id),
                            (Some(id), Incoming::Text(_)) => self.text_added(id),
                            (Some(id), Incoming::Media(_)) => {
                                self.selected = Some(id);
                                self.touch();
                                self.fx_land(id, 1.0);
                            }
                            (None, _) => {
                                self.history.undo(&mut self.project);
                            }
                        }
                        egui::DragAndDrop::clear_payload(ui.ctx());
                    }
                }
            }
        }

        // Подсказка для пустого таймлайна.
        if self.project.is_empty() && !egui::DragAndDrop::has_any_payload(ui.ctx()) {
            let main_row = rows.iter().rev().find(|(i, _)| self.project.tracks[*i].kind == TrackKind::Video);
            if let Some((_, row)) = main_row {
                p.text(
                    row.center(),
                    Align2::CENTER_CENTER,
                    "Перетащите сюда файлы из библиотеки или дважды щёлкните по файлу",
                    theme::font(12.0),
                    TEXT3,
                );
            }
        }

        // Курсор воспроизведения.
        let x = geo.x(self.playhead);
        if x >= lanes.left() - 6.0 && x <= lanes.right() + 6.0 {
            let top = pos2(x, ruler.top() + 2.0);
            let ends = [top, pos2(x, lanes_bottom)];
            p.line_segment(ends, Stroke::new(5.0, ACCENT.gamma_multiply(0.12)));
            p.line_segment(ends, Stroke::new(1.0, ACCENT));
            // Головка укрупняется, пока её тащат.
            let scrub = matches!(drag, Some(Drag::Scrub)) && primary_down;
            let k = 1.0 + 0.3 * ui.ctx().animate_bool_with_time(ui.id().with("scrub"), scrub, 0.1);
            let head = [vec2(-5.5, 0.0), vec2(5.5, 0.0), vec2(5.5, 8.0), vec2(0.0, 13.0), vec2(-5.5, 8.0)];
            p.add(Shape::convex_polygon(head.iter().map(|v| top + *v * k).collect(), ACCENT, Stroke::NONE));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use eframe::egui::{self, pos2, vec2, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    use crate::app::{App, Category, InspectorTab, Page, TextDrag, Tool, TransitionDrag, VfxShelf, VfxTool};
    use crate::effects::EffectKind;
    use crate::keys::{Key, Prop};
    use crate::look::FilterKind;
    use crate::model::{Clip, MediaItem, MediaKind};
    use crate::text::TextPreset;
    use crate::transition::TransitionKind;
    use crate::ui::fx;
    use crate::ui::inspector::pin_id;
    use crate::vfx::{Blend, FxKind, MaskShape, VfxPreset};

    /// Приложение без окна: кадры и мышь подаются вручную.
    struct Rig {
        ctx: egui::Context,
        app: App,
        time: f64,
    }

    impl Rig {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let mut app = App::new(&ctx, vec![]);
            app.preview.enabled = false;
            let id = app.project.alloc_id();
            app.project.media.push(MediaItem {
                id,
                path: PathBuf::from("no-such-file.mp4"),
                name: "clip.mp4".into(),
                kind: MediaKind::Video,
                duration: 10.0,
                width: 1920,
                height: 1080,
                fps: 30.0,
                has_audio: false,
            });
            app.project.append_clip(id).unwrap();
            app.tl.pps = 50.0;
            let mut rig = Rig { ctx, app, time: 0.0 };
            rig.step(vec![]);
            rig.step(vec![]);
            rig
        }

        fn step(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let app = &mut self.app;
            let _ = self.ctx.run(input, |ctx| app.frame(ctx));
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            self.step(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.step(vec![Event::PointerMoved(from)]);
            self.button(from, true);
            for i in 1..=8 {
                let p = from + (to - from) * (i as f32 / 8.0);
                self.step(vec![Event::PointerMoved(p)]);
            }
            self.button(to, false);
            self.step(vec![]);
        }

        fn click(&mut self, pos: Pos2) {
            self.step(vec![Event::PointerMoved(pos)]);
            self.button(pos, true);
            self.button(pos, false);
            self.step(vec![]);
        }

        /// Точка на дорожке `track` в момент `t`.
        fn at(&self, t: f64, track: usize) -> Pos2 {
            let v = self.app.tl.view.as_ref().unwrap();
            let row = v.rows.iter().find(|(i, _)| *i == track).unwrap().1;
            pos2(v.x(t), row.center().y)
        }

        fn clips(&self, track: usize) -> &[Clip] {
            &self.app.project.tracks[track].clips
        }
    }

    #[test]
    fn drag_moves_clip_and_undo_restores() {
        let mut r = Rig::new();
        let from = r.at(5.0, 1);
        r.drag(from, from + vec2(100.0, 0.0));
        let start = r.clips(1)[0].start;
        assert!((start - 2.0).abs() < 0.05, "начало {start}");
        r.app.undo();
        assert_eq!(r.clips(1)[0].start, 0.0);
    }

    #[test]
    fn drag_to_upper_video_track() {
        let mut r = Rig::new();
        let (from, to) = (r.at(5.0, 1), r.at(5.0, 0));
        r.drag(from, to);
        assert_eq!(r.clips(0).len(), 1);
        assert!(r.clips(1).is_empty());
    }

    #[test]
    fn video_cannot_be_dropped_on_audio_track() {
        let mut r = Rig::new();
        let (from, to) = (r.at(5.0, 1), r.at(5.0, 2));
        r.drag(from, to);
        assert_eq!(r.clips(1).len(), 1);
        assert!(r.clips(2).is_empty());
    }

    #[test]
    fn edges_trim_clip() {
        let mut r = Rig::new();
        let right = r.at(10.0, 1) - vec2(3.0, 0.0);
        r.drag(right, right - vec2(100.0, 0.0));
        let c = r.clips(1)[0].clone();
        assert!((c.duration - 8.0).abs() < 0.05, "длительность {}", c.duration);

        let left = r.at(0.0, 1) + vec2(3.0, 0.0);
        r.drag(left, left + vec2(50.0, 0.0));
        let c = r.clips(1)[0].clone();
        assert!((c.start - 1.0).abs() < 0.05, "начало {}", c.start);
        assert!((c.offset - 1.0).abs() < 0.05, "смещение {}", c.offset);
        assert!((c.end() - 8.0).abs() < 0.05, "конец {}", c.end());
    }

    #[test]
    fn blade_click_splits_clip() {
        let mut r = Rig::new();
        r.app.tl.tool = Tool::Blade;
        let p = r.at(4.0, 1);
        r.click(p);
        assert_eq!(r.clips(1).len(), 2);
        assert!((r.clips(1)[1].start - 4.0).abs() < 0.05);
    }

    #[test]
    fn clicks_select_and_scrub() {
        let mut r = Rig::new();
        let on_clip = r.at(5.0, 1);
        r.click(on_clip);
        assert_eq!(r.app.selected, Some(r.clips(1)[0].id));

        let empty = r.at(12.0, 0);
        r.click(empty);
        assert_eq!(r.app.selected, None);

        let v = r.app.tl.view.clone().unwrap();
        r.click(pos2(v.x(3.0), v.ruler.center().y));
        assert!((r.app.playhead - 3.0).abs() < 0.05, "курсор {}", r.app.playhead);
    }

    /// Клип, разрезанный лезвием на четвёртой секунде.
    fn cut_rig() -> Rig {
        let mut r = Rig::new();
        r.app.tl.tool = Tool::Blade;
        let p = r.at(4.0, 1);
        r.click(p);
        r.app.tl.tool = Tool::Select;
        r
    }

    #[test]
    fn transition_is_applied_selected_and_deleted() {
        let mut r = cut_rig();
        let (left, right) = (r.clips(1)[0].id, r.clips(1)[1].id);
        // Двойной щелчок по плитке: переход встаёт на ближайший к курсору стык.
        r.app.apply_transition_auto(TransitionKind::Dissolve);
        assert!(r.clips(1)[1].transition.is_some());
        assert_eq!((r.app.selected, r.app.sel_transition), (None, Some(right)));

        // Щелчок по клипу выбирает клип, щелчок по значку на стыке — переход.
        let on_clip = r.at(1.0, 1);
        r.click(on_clip);
        assert_eq!((r.app.selected, r.app.sel_transition), (Some(left), None));
        let on_marker = r.at(4.0, 1);
        r.click(on_marker);
        assert_eq!((r.app.selected, r.app.sel_transition), (None, Some(right)));

        r.app.delete_selected();
        assert_eq!(r.clips(1).len(), 2, "удаляется переход, а не клип");
        assert!(r.clips(1)[1].transition.is_none());
        r.app.undo();
        assert!(r.clips(1)[1].transition.is_some());
    }

    #[test]
    fn transition_is_dropped_on_a_junction() {
        let mut r = cut_rig();
        let junction = r.at(4.0, 1);
        egui::DragAndDrop::set_payload(&r.ctx, TransitionDrag(TransitionKind::SlideLeft));
        r.step(vec![Event::PointerMoved(junction + vec2(20.0, 0.0))]);
        r.button(junction + vec2(20.0, 0.0), true);
        r.button(junction + vec2(20.0, 0.0), false);
        let kind = r.clips(1)[1].transition.as_ref().map(|t| t.kind);
        assert_eq!(kind, Some(TransitionKind::SlideLeft));
    }

    #[test]
    fn transition_disappears_when_clips_are_pulled_apart() {
        let mut r = cut_rig();
        r.app.apply_transition_auto(TransitionKind::Fade);
        let from = r.at(7.0, 1);
        r.drag(from, from + vec2(150.0, 0.0));
        assert!(r.clips(1)[1].start > 4.5);
        assert!(r.clips(1)[1].transition.is_none());
    }

    #[test]
    fn effect_lands_on_top_track_and_moves_between_video_tracks() {
        let mut r = Rig::new();
        r.app.playhead = 2.0;
        r.app.add_effect(EffectKind::Explosion);
        let id = r.clips(0)[0].id;
        assert_eq!(r.clips(0)[0].start, 2.0);
        assert_eq!(r.app.selected, Some(id));
        assert!(r.app.inspector_tab == InspectorTab::Effect);
        r.step(vec![]);

        // По пути к звуковой дорожке эффект остаётся на последней видеодорожке, где место нашлось
        // только после лежащего там клипа.
        let (from, audio) = (r.at(2.5, 0), r.at(2.5, 2));
        r.drag(from, audio);
        assert_eq!((r.clips(0).len(), r.clips(1).len(), r.clips(2).len()), (0, 2, 0));
        assert_eq!(r.clips(1)[1].start, 10.0);
        let (from, upper) = (r.at(10.5, 1), r.at(5.0, 0));
        r.drag(from, upper);
        assert_eq!((r.clips(0).len(), r.clips(1).len()), (1, 1));
    }

    #[test]
    fn path_is_drawn_in_the_frame() {
        let mut r = Rig::new();
        let id = r.clips(1)[0].id;
        r.app.selected = Some(id);
        r.app.path_edit = Some(id);
        r.step(vec![]);
        let canvas = r.app.canvas.unwrap();
        let (c, k) = (canvas.center(), canvas.width() / 1920.0);
        let points = |r: &Rig| r.clips(1)[0].motion.as_ref().map_or(Vec::new(), |m| m.points.clone());

        // Щелчки ставят точки.
        r.click(c + vec2(-80.0, 0.0));
        r.click(c + vec2(80.0, 40.0));
        let p = points(&r);
        assert_eq!(p.len(), 2);
        assert!((p[0][0] + 80.0 / k).abs() <= 1.0 && (p[1][1] - 40.0 / k).abs() <= 1.0, "{p:?}");

        // Точку можно перетащить.
        r.drag(c + vec2(-80.0, 0.0), c + vec2(-80.0, -60.0));
        let p = points(&r);
        assert_eq!(p.len(), 2);
        assert!((p[0][1] + 60.0 / k).abs() <= 1.0, "{p:?}");

        // Росчерк по прямой оставляет только свои концы.
        r.drag(c + vec2(0.0, 100.0), c + vec2(120.0, 100.0));
        assert_eq!(points(&r).len(), 4);
        r.app.undo();
        assert_eq!(points(&r).len(), 2);

        // Клип идёт по пути: в начале он в первой точке, в конце — в последней.
        let clip = r.clips(1)[0].clone();
        let (start, end) = (clip.position(clip.start), clip.position(clip.end()));
        assert!((start.0 + 80.0 / k).abs() <= 1.0 && (end.0 - 80.0 / k).abs() <= 1.0);
    }

    /// Клип выбран, курсор времени стоит на второй секунде, непрозрачность — 20%.
    fn pin_rig() -> (Rig, Pos2) {
        let mut r = Rig::new();
        let id = r.clips(1)[0].id;
        r.app.selected = Some(id);
        r.app.project.clip_mut(id).unwrap().opacity = 0.2;
        r.app.playhead = 2.0;
        r.step(vec![]);
        r.step(vec![]);
        let button = r.ctx.read_response(pin_id(Prop::Opacity)).expect("кнопка закрепления").rect;
        (r, button.center())
    }

    #[test]
    fn value_is_pinned_at_two_moments_and_changes_between_them() {
        let (mut r, button) = pin_rig();
        // Кнопка-ромб закрепляет то, что сейчас стоит в поле.
        r.click(button);
        assert_eq!(r.clips(1)[0].keys.opacity.0, vec![Key { t: 2.0, v: 0.2 }]);

        // Курсор времени передвинули и поменяли значение ползунком — закрепилась вторая точка.
        r.app.playhead = 6.0;
        r.step(vec![]);
        r.drag(button - vec2(150.0, 0.0), button - vec2(100.0, 0.0));
        let clip = r.clips(1)[0].clone();
        let keys = clip.keys.opacity.0.clone();
        assert_eq!((keys.len(), keys[1].t), (2, 6.0));
        assert!(keys[1].v > 0.3, "значение {}", keys[1].v);
        assert_eq!(clip.value(Prop::Opacity, 2.0), 0.2);
        assert!((clip.value(Prop::Opacity, 4.0) - (0.2 + keys[1].v) / 2.0).abs() < 1e-6);
        // Остальные свойства остались обычными.
        assert!(clip.keys.scale.is_empty() && clip.keys.pos_x.is_empty());

        // Ползунок был одной правкой: отмена возвращает к одной точке.
        r.app.undo();
        assert_eq!(r.clips(1)[0].keys.opacity.0.len(), 1);
        r.app.redo();

        // Повторное нажатие на закреплённой точке убирает её; значение остаётся тем, что было видно.
        r.step(vec![]);
        r.click(button);
        assert_eq!(r.clips(1)[0].keys.opacity.0.len(), 1);
        r.app.playhead = 2.0;
        r.step(vec![]);
        r.click(button);
        let clip = r.clips(1)[0].clone();
        assert!(clip.keys.is_empty());
        assert_eq!(clip.opacity, 0.2);
    }

    #[test]
    fn looking_at_a_changing_value_does_not_pin_it() {
        let (mut r, _) = pin_rig();
        let id = r.clips(1)[0].id;
        let reach = r.app.project.key_reach();
        let clip = r.app.project.clip_mut(id).unwrap();
        for prop in [Prop::Scale, Prop::PosX, Prop::Rotation, Prop::Opacity] {
            let to = clip.value(prop, 6.0) + 0.337;
            clip.toggle_key(prop, 2.0, reach);
            clip.set_value(prop, 6.0, to, reach);
        }
        let before = r.clips(1)[0].keys.clone();
        // Между точками значения дробные: поля показывают их округлёнными, но сами ничего не правят.
        for t in [2.7, 3.3, 4.9] {
            r.app.playhead = t;
            r.step(vec![]);
            r.step(vec![]);
        }
        assert_eq!(r.clips(1)[0].keys, before);
        assert!(!r.app.history.can_undo(), "показ значения не должен попадать в историю правок");
    }

    #[test]
    fn pin_button_fits_into_the_row() {
        let (r, _) = pin_rig();
        let rects: Vec<Rect> = [Prop::Scale, Prop::PosX, Prop::PosY, Prop::Rotation, Prop::Opacity]
            .map(|prop| r.ctx.read_response(pin_id(prop)).expect("кнопка закрепления").rect)
            .to_vec();
        // Кнопки стоят одной колонкой у правого края панели и не вылезают за него.
        let panel_right = 1440.0 - 6.0 - 12.0;
        for rect in &rects {
            assert_eq!(rect.center().x, rects[0].center().x);
            assert!(rect.right() <= panel_right + 0.5, "кнопка {rect:?} выходит за панель");
            assert!(rect.width() >= 17.0 && rect.height() >= 17.0, "кнопка {rect:?} сжата");
        }
    }

    #[test]
    fn playhead_snaps_to_pinned_points_of_the_selected_clip() {
        let mut r = Rig::new();
        let id = r.clips(1)[0].id;
        // Точка стоит не на сетке кадров: без привязки курсор на неё не попал бы.
        r.app.project.clip_mut(id).unwrap().toggle_key(Prop::Scale, 3.01, 0.01);
        r.app.selected = Some(id);
        r.step(vec![]);
        let v = r.app.tl.view.clone().unwrap();
        let ruler = |t: f64| pos2(v.x(t), v.ruler.center().y);
        r.drag(ruler(1.0), ruler(3.01) + vec2(4.0, 0.0));
        assert_eq!(r.app.playhead, 3.01);
        // Вдали от точки курсор встаёт по кадрам.
        r.drag(ruler(1.0), ruler(5.0));
        assert!((r.app.playhead - 5.0).abs() < 0.02, "курсор {}", r.app.playhead);
    }

    #[test]
    fn text_is_added_at_the_playhead_and_dropped_from_the_library() {
        let mut r = Rig::new();
        r.app.playhead = 2.0;
        r.app.add_text(TextPreset::Title);
        let id = r.clips(0)[0].id;
        assert_eq!((r.clips(0)[0].start, r.app.selected), (2.0, Some(id)));
        assert!(r.app.inspector_tab == InspectorTab::Text);
        // Раздел библиотеки и вкладка с настройками надписи рисуются.
        r.app.lib.category = Category::Text;
        r.step(vec![]);
        r.step(vec![]);
        for tab in [InspectorTab::Video, InspectorTab::Motion] {
            r.app.inspector_tab = tab;
            r.step(vec![]);
            assert!(r.app.inspector_tab == tab, "у надписи есть эта вкладка");
        }

        // Плитку тащат на верхнюю видеодорожку.
        let drop = r.at(7.0, 0);
        egui::DragAndDrop::set_payload(&r.ctx, TextDrag(TextPreset::Outline));
        r.step(vec![Event::PointerMoved(drop)]);
        r.button(drop, true);
        r.button(drop, false);
        assert_eq!(r.clips(0).len(), 2);
        let dropped = r.clips(0)[1].clone();
        assert_eq!(dropped.text.as_ref().unwrap().font, "Impact");
        assert!((dropped.start - 7.0).abs() < 0.05 && r.app.selected == Some(dropped.id));
        r.app.undo();
        assert_eq!(r.clips(0).len(), 1);
    }

    #[test]
    fn captions_are_added_imported_and_listed() {
        let mut r = Rig::new();
        r.app.playhead = 1.0;
        r.app.add_caption();
        assert!(r.app.project.tracks[0].captions && r.clips(0).len() == 1);
        assert!(r.app.inspector_tab == InspectorTab::Text);

        let dir = std::env::temp_dir().join(format!("lumicut-srt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("in.srt");
        std::fs::write(&path, "1\n00:00:05,000 --> 00:00:06,500\nПривет\n\n2\n00:00:07,000 --> 00:00:08,000\nПока\n").unwrap();
        r.app.import_srt(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(r.clips(0).len(), 3);
        let cues = r.app.caption_cues();
        assert_eq!((cues[1].start, cues[1].end, cues[1].text.as_str()), (5.0, 6.5, "Привет"));
        assert_eq!(cues[2].text, "Пока");

        // Список субтитров и пять дорожек на таймлайне рисуются.
        r.app.lib.category = Category::Subtitles;
        r.step(vec![]);
        r.step(vec![]);
        assert_eq!(r.app.tl.view.as_ref().unwrap().rows.len(), 5);
        // Импорт — одна правка.
        r.app.undo();
        assert_eq!(r.clips(0).len(), 1);
    }

    #[test]
    fn filter_goes_to_the_selected_clip_or_the_one_under_the_playhead() {
        let mut r = Rig::new();
        r.app.playhead = 3.0;
        r.app.apply_filter(FilterKind::Sepia);
        let id = r.clips(1)[0].id;
        assert_eq!((r.clips(1)[0].look.filter, r.app.selected), (FilterKind::Sepia, Some(id)));
        assert!(r.app.inspector_tab == InspectorTab::Color);
        // Плитки фильтров и вкладка «Цвет» рисуются; от одного показа ничего не меняется.
        r.app.lib.category = Category::Filters;
        r.app.project.clip_mut(id).unwrap().look.contrast = 0.337;
        let before = r.clips(1)[0].look.clone();
        for _ in 0..3 {
            r.step(vec![]);
        }
        assert_eq!(r.clips(1)[0].look, before);

        // Над пустым местом и без выбора фильтру лечь некуда.
        r.app.selected = None;
        r.app.playhead = 10.0;
        r.app.apply_filter(FilterKind::Mono);
        assert_eq!(r.clips(1)[0].look.filter, FilterKind::Sepia);
        r.app.undo();
        assert_eq!(r.clips(1)[0].look.filter, FilterKind::None);
    }

    #[test]
    fn clips_are_copied_pasted_duplicated_and_deleted_with_a_shift() {
        let mut r = cut_rig();
        let (a, b) = (r.clips(1)[0].id, r.clips(1)[1].id);
        r.app.selected = Some(a);
        r.app.playhead = 12.0;
        r.step(vec![Event::Copy]);
        r.step(vec![Event::Paste("что-то из буфера".into())]);
        assert_eq!(r.clips(1).len(), 3);
        let pasted = r.clips(1)[2].clone();
        assert_eq!((pasted.start, pasted.duration, r.app.selected), (12.0, 4.0, Some(pasted.id)));

        r.app.selected = Some(b);
        r.step(vec![Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        assert_eq!(r.clips(1).len(), 4);
        assert_eq!(r.clips(1)[3].start, 16.0, "копия встаёт в ближайшее свободное место после клипа");

        r.app.selected = Some(a);
        r.app.ripple_delete_selected();
        let starts: Vec<f64> = r.clips(1).iter().map(|c| c.start).collect();
        assert_eq!(starts, [0.0, 8.0, 12.0]);
        r.app.undo();
        assert_eq!(r.clips(1).len(), 4);
    }

    #[test]
    fn page_switch_opens_the_vfx_editor_and_back() {
        let mut r = Rig::new();
        let id = r.clips(1)[0].id;
        r.app.playhead = 3.0;
        r.step(vec![]);
        // Переключатель страниц в верхней панели: подпись «VFX» — вторая.
        let vfx = r.ctx.read_response(egui::Id::new("page").with(1usize)).expect("переключатель страниц").rect;
        r.click(vfx.center());
        assert!(r.app.page == Page::Vfx);
        // Без выбора страница сразу берёт клип под курсором времени.
        assert_eq!(r.app.selected, Some(id));
        // Инструмент в кадре и режимы просмотра живут только на этой странице: Esc убирает инструмент,
        // а выделение снимает лишь следующим нажатием.
        r.app.vfx_ui.tool = VfxTool::Pipette;
        r.app.vfx_ui.solo = true;
        let esc = || vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }];
        r.step(esc());
        assert!(r.app.vfx_ui.tool == VfxTool::Move && r.app.selected == Some(id));
        r.step(vec![Event::Key { key: egui::Key::Num1, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]);
        assert!(r.app.page == Page::Edit && !r.app.vfx_ui.solo);
        for _ in 0..40 {
            r.step(vec![]);
        }
        assert!(r.app.preview.matte.is_none() && r.app.preview.original.is_none());
    }

    #[test]
    fn vfx_page_stacks_effects_and_carries_style_between_clips() {
        let mut r = cut_rig();
        let (a, b) = (r.clips(1)[0].id, r.clips(1)[1].id);
        r.app.selected = Some(a);
        r.app.set_page(Page::Vfx);
        let kinds = |r: &Rig, i: usize| r.clips(1)[i].vfx.stack.iter().map(|fx| fx.kind).collect::<Vec<FxKind>>();

        // Эффекты встают в стопку по порядку; готовый набор заменяет её целиком, отмена возвращает.
        r.app.add_fx(FxKind::Blur);
        r.app.add_fx(FxKind::Glow);
        assert_eq!(kinds(&r, 0), [FxKind::Blur, FxKind::Glow]);
        r.app.apply_vfx_preset(VfxPreset::Comic);
        assert_eq!(kinds(&r, 0), [FxKind::Posterize, FxKind::Sharpen]);
        r.app.undo();
        assert_eq!(kinds(&r, 0), [FxKind::Blur, FxKind::Glow]);

        // Все полки библиотеки и разделы оформления рисуются; от одного показа ничего не меняется.
        let clip = r.app.project.clip_mut(a).unwrap();
        clip.vfx.blend = Blend::Screen;
        clip.vfx.crop = [0.137, 0.0, 0.0, 0.0];
        clip.vfx.mask = Some(crate::vfx::Mask::new(MaskShape::Rect));
        clip.vfx.stack[0].amount = 0.337;
        clip.look.key = Some(crate::look::ChromaKey { similarity: 0.337, ..crate::look::ChromaKey::studio([0, 255, 0]) });
        let before = r.clips(1)[0].clone();
        r.app.history.clear();
        for shelf in [VfxShelf::Looks, VfxShelf::Generators, VfxShelf::Effects] {
            r.app.vfx_ui.shelf = shelf;
            for _ in 0..3 {
                r.step(vec![]);
            }
        }
        r.app.vfx_ui.tool = VfxTool::Mask;
        (r.app.vfx_ui.matte, r.app.vfx_ui.solo, r.app.vfx_ui.original) = (true, true, true);
        for _ in 0..3 {
            r.step(vec![]);
        }
        assert_eq!((&r.clips(1)[0].look, &r.clips(1)[0].vfx), (&before.look, &before.vfx));
        assert!(!r.app.history.can_undo(), "показ оформления не должен попадать в историю правок");
        // Режимы просмотра относятся к выбранному клипу.
        assert_eq!((r.app.preview.matte, r.app.preview.original), (Some(a), Some(a)));

        // Оформление переносится на другой клип одной правкой.
        r.app.copy_style();
        r.app.selected = Some(b);
        r.app.paste_style();
        assert_eq!((&r.clips(1)[1].look, &r.clips(1)[1].vfx), (&before.look, &before.vfx));
        r.app.undo();
        assert!(!r.clips(1)[1].styled());

        // У надписи есть только наложение: панель рисуется и для неё, эффекты на неё не ложатся.
        r.app.playhead = 1.0;
        r.app.add_text(TextPreset::Title);
        let text = r.app.selected.unwrap();
        for _ in 0..3 {
            r.step(vec![]);
        }
        r.app.add_fx(FxKind::Invert);
        assert!(r.app.project.clip(text).unwrap().vfx.stack.is_empty());
        assert_eq!(kinds(&r, 0).len(), 3, "эффект лёг на видео под курсором");
        // Анимации появления панелей не держат перерисовку вечно.
        for _ in 0..120 {
            r.step(vec![]);
        }
        assert!(!fx::busy(&r.ctx));
    }

    /// Пипетка и маска в кадре на настоящем видео: зелёный фон с красным квадратом посередине.
    /// Запуск: `cargo test -- --ignored vfx_tools_work_in_the_frame`.
    #[test]
    #[ignore = "нужен ffmpeg"]
    fn vfx_tools_work_in_the_frame() {
        let dir = std::env::temp_dir().join(format!("lumicut-stage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("green.mp4");
        let source = "color=c=0x10D030:s=320x180:r=30:d=2,drawbox=x=120:y=50:w=80:h=80:color=red:t=fill";
        let made = crate::ffmpeg::ffmpeg()
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", source, "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status();
        assert!(made.map_or(false, |s| s.success()));

        let mut r = Rig::new();
        r.app.preview.enabled = true;
        let media = r.app.project.alloc_id();
        r.app.project.media.push(MediaItem {
            id: media,
            path,
            name: "green.mp4".into(),
            kind: MediaKind::Video,
            duration: 2.0,
            width: 320,
            height: 180,
            fps: 30.0,
            has_audio: false,
        });
        let id = r.app.project.add_clip(0, media, 0.0).unwrap();
        r.app.selected = Some(id);
        r.app.playhead = 0.5;
        r.app.set_page(Page::Vfx);
        // Ждём кадр от декодера.
        for _ in 0..300 {
            r.step(vec![]);
            if !r.app.preview.pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        r.step(vec![]);
        let canvas = r.app.canvas.unwrap();
        let clip = |r: &Rig| r.app.project.clip(id).unwrap().clone();

        // Пипетка: щелчок по фону берёт его цвет, включает хромакей и убирает инструмент.
        r.app.vfx_ui.tool = VfxTool::Pipette;
        r.step(vec![]);
        r.click(canvas.left_top() + vec2(canvas.width() * 0.1, canvas.height() * 0.5));
        let key = clip(&r).look.key.expect("хромакей включён пипеткой");
        assert!(key.color[1] > 170 && key.color[0] < 70 && key.color[2] < 90, "{:?}", key.color);
        assert!(key.spill > 0.0 && r.app.vfx_ui.tool == VfxTool::Move);
        // Щелчок по квадрату меняет цвет у того же хромакея.
        r.app.vfx_ui.tool = VfxTool::Pipette;
        r.step(vec![]);
        r.click(canvas.center());
        let red = clip(&r).look.key.unwrap().color;
        assert!(red[0] > 170 && red[1] < 70, "{red:?}");
        r.app.undo();
        r.app.undo();
        assert!(clip(&r).look.key.is_none());

        // Маска: за середину её двигают, за угол — растягивают; середина при этом стоит на месте.
        r.app.project.clip_mut(id).unwrap().vfx.mask = Some(crate::vfx::Mask::new(MaskShape::Rect));
        r.app.vfx_ui.tool = VfxTool::Mask;
        r.step(vec![]);
        let at = |u: f32, v: f32| canvas.left_top() + vec2(canvas.width() * u, canvas.height() * v);
        r.drag(at(0.5, 0.5), at(0.6, 0.4));
        let mask = clip(&r).vfx.mask.unwrap();
        assert!((mask.x - 0.6).abs() < 0.01 && (mask.y - 0.4).abs() < 0.01, "{mask:?}");
        r.drag(at(0.9, 0.7), at(0.8, 0.6));
        let mask = clip(&r).vfx.mask.unwrap();
        assert!((mask.w - 0.4).abs() < 0.02 && (mask.h - 0.4).abs() < 0.02, "{mask:?}");
        assert!((mask.x - 0.6).abs() < 0.01);
        // Каждое перетаскивание — одна правка.
        r.app.undo();
        assert!((clip(&r).vfx.mask.unwrap().w - 0.6).abs() < 1e-6);

        drop(r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn effects_fire_and_settle() {
        let mut r = Rig::new();
        r.app.tl.tool = Tool::Blade;
        let p = r.at(4.0, 1);
        r.click(p);
        assert!(fx::busy(&r.ctx), "разрез должен дать искры");
        r.app.selected = Some(r.clips(1)[0].id);
        r.app.delete_selected();
        r.app.undo();
        // Эффекты не должны держать перерисовку вечно.
        for _ in 0..120 {
            r.step(vec![]);
        }
        assert!(!fx::busy(&r.ctx), "эффекты должны затихнуть");
    }
}
