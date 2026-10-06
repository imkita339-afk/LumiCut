//! Центральная панель: кадр предпросмотра и управление воспроизведением.

use std::f32::consts::TAU;

use eframe::egui::{
    emath::Rot2, pos2, vec2, Align, Align2, Color32, CursorIcon, Layout, Mesh, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, Ui, UiBuilder, Vec2,
};

use crate::app::{App, Page, StageDrag, VfxTool};
use crate::icons;
use crate::keys::Prop;
use crate::look::ChromaKey;
use crate::model::{fit_size, timecode, Id};
use crate::motion::Motion;
use crate::playback::Layer;
use crate::theme::{self, *};
use crate::ui::{fx, widgets};
use crate::vfx::{Mask, MaskShape};

/// Положение слоя на экране.
struct Placed {
    center: Pos2,
    size: Vec2,
    angle: f32,
}

impl Placed {
    fn new(l: &Layer, canvas: Rect, k: f32) -> Self {
        Placed {
            center: canvas.center() + vec2(l.pos_x, l.pos_y) * k,
            size: vec2(l.base_w, l.base_h) * l.scale * k,
            angle: l.rotation.to_radians(),
        }
    }

    fn corners(&self) -> [Pos2; 4] {
        let rot = Rot2::from_angle(self.angle);
        let h = self.size / 2.0;
        [vec2(-h.x, -h.y), vec2(h.x, -h.y), vec2(h.x, h.y), vec2(-h.x, h.y)].map(|v| self.center + rot * v)
    }

    fn contains(&self, p: Pos2) -> bool {
        let local = Rot2::from_angle(-self.angle) * (p - self.center);
        local.x.abs() <= self.size.x / 2.0 && local.y.abs() <= self.size.y / 2.0
    }

    /// Точка экрана для места (`u`, `v`) в кадре клипа: доли его ширины и высоты.
    fn at(&self, u: f32, v: f32, flip_h: bool, flip_v: bool) -> Pos2 {
        let (u, v) = (if flip_h { 1.0 - u } else { u }, if flip_v { 1.0 - v } else { v });
        self.center + Rot2::from_angle(self.angle) * vec2((u - 0.5) * self.size.x, (v - 0.5) * self.size.y)
    }

    /// Место точки экрана в кадре клипа: доли его ширины и высоты.
    fn uv(&self, p: Pos2, flip_h: bool, flip_v: bool) -> (f32, f32) {
        let local = Rot2::from_angle(-self.angle) * (p - self.center);
        let (u, v) = (local.x / self.size.x.max(1.0) + 0.5, local.y / self.size.y.max(1.0) + 0.5);
        (if flip_h { 1.0 - u } else { u }, if flip_v { 1.0 - v } else { v })
    }
}

/// Подсказка у верхнего края кадра.
fn stage_tip(ui: &Ui, stage: Rect, text: &str) {
    // Новая подсказка проявляется и опускается на место.
    let t = fx::fresh(ui.ctx(), eframe::egui::Id::new(("stage-tip", text)));
    let mut painter = ui.painter().with_clip_rect(stage);
    painter.multiply_opacity(t);
    let galley = painter.layout_no_wrap(text.to_owned(), theme::font(11.5), TEXT);
    let center = pos2(stage.center().x, stage.top() + 20.0 - 8.0 * (1.0 - t));
    let bar = Rect::from_center_size(center, galley.size() + vec2(20.0, 10.0));
    painter.rect_filled(bar, 6.0, PANEL.gamma_multiply(0.9));
    painter.galley(bar.min + vec2(10.0, 5.0), galley, TEXT);
}

/// Шахматная клетка: так в кадре выглядит прозрачность.
fn checkerboard(p: &Painter, rect: Rect) {
    const CELL: f32 = 14.0;
    p.rect_filled(rect, 0.0, Color32::from_gray(34));
    let (columns, rows) = ((rect.width() / CELL).ceil() as i32, (rect.height() / CELL).ceil() as i32);
    for y in 0..rows {
        for x in (y % 2..columns).step_by(2) {
            let cell = Rect::from_min_size(rect.min + vec2(x as f32, y as f32) * CELL, Vec2::splat(CELL));
            p.rect_filled(cell.intersect(rect), 0.0, Color32::from_gray(52));
        }
    }
}

/// Углы рамки маски в долях кадра клипа.
fn mask_corners(mask: &Mask) -> [(f32, f32); 4] {
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sy)| (mask.x + sx * mask.w / 2.0, mask.y + sy * mask.h / 2.0))
}

/// Радиус точки пути на экране и расстояние, с которого она ловит указатель.
const POINT_R: f32 = 5.0;
const POINT_GRAB: f32 = 9.0;
/// Шаг между точками, которые ставит рисование от руки, пиксели экрана.
const DRAW_STEP: f32 = 14.0;

impl App {
    /// Путь выбранного клипа в кадре и его правка мышью. `origin` — положение клипа на экране,
    /// `k` — экранных пикселей на пиксель проекта.
    fn path_ui(&mut self, ui: &Ui, resp: &Response, stage: Rect, origin: Pos2, k: f32, id: Id) {
        let editing = self.path_edit == Some(id);
        let motion = self.project.clip(id).and_then(|c| c.motion.clone()).unwrap_or_default();
        let to_screen = |p: [f32; 2]| origin + vec2(p[0], p[1]) * k;
        let to_path = |pos: Pos2| {
            let v = (pos - origin) / k;
            [v.x.round(), v.y.round()]
        };
        let handle_at = |pos: Pos2| motion.points.iter().position(|p| (to_screen(*p) - pos).length() <= POINT_GRAB);

        // Линия пути: сплошная, пока его правят, иначе пунктир.
        let painter = ui.painter().with_clip_rect(stage);
        let line: Vec<Pos2> = motion.polyline().into_iter().map(to_screen).collect();
        if line.len() >= 2 {
            painter.add(Shape::line(line.clone(), Stroke::new(4.0, Color32::from_black_alpha(110))));
            if editing {
                painter.add(Shape::line(line, Stroke::new(1.5, ACCENT)));
            } else {
                painter.extend(Shape::dashed_line(&line, Stroke::new(1.5, ACCENT), 7.0, 5.0));
            }
        }
        if !editing {
            return;
        }

        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| stage.contains(*p));
        let hot = hover.and_then(handle_at);
        for (i, p) in motion.points.iter().enumerate() {
            // Первая точка — начало пути — залита акцентным цветом.
            let r = if hot == Some(i) { POINT_R + 2.0 } else { POINT_R };
            let fill = if i == 0 { ACCENT } else { Color32::WHITE };
            painter.circle_filled(to_screen(*p), r + 1.5, Color32::from_black_alpha(140));
            painter.circle(to_screen(*p), r, fill, Stroke::new(1.5, if i == 0 { ON_ACCENT } else { ACCENT }));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(if hot.is_some() { CursorIcon::Grab } else { CursorIcon::Crosshair });
        }
        stage_tip(ui, stage, "Щелчок — точка  ·  ведите с нажатой кнопкой — путь от руки  ·  правый щелчок по точке — удалить  ·  Esc — готово");

        let pointer = resp.interact_pointer_pos();
        if resp.drag_started() {
            self.stage_drag = None;
            if let Some(press) = ui.input(|i| i.pointer.press_origin()) {
                self.commit();
                self.stage_drag = Some(match handle_at(press) {
                    Some(i) => StageDrag::PathPoint(i),
                    None => {
                        self.edit_path(id, |m| m.points.push(to_path(press)));
                        StageDrag::PathDraw { from: motion.points.len() }
                    }
                });
            }
        }
        if let (true, Some(pos)) = (resp.dragged(), pointer) {
            match self.stage_drag {
                Some(StageDrag::PathPoint(i)) => self.edit_path(id, |m| {
                    if let Some(p) = m.points.get_mut(i) {
                        *p = to_path(pos);
                    }
                }),
                Some(StageDrag::PathDraw { .. }) => self.edit_path(id, |m| {
                    let far = m.points.last().map_or(true, |last| (to_screen(*last) - pos).length() >= DRAW_STEP);
                    if far {
                        m.points.push(to_path(pos));
                    }
                }),
                _ => {}
            }
        }
        if resp.drag_stopped() {
            if let Some(StageDrag::PathDraw { from }) = self.stage_drag {
                // Росчерк от руки прореживается: остаются точки, которые задают его форму.
                self.edit_path(id, |m| m.simplify(from, 3.0 / k));
            }
            self.stage_drag = None;
            self.touch();
        }
        if let (true, Some(pos)) = (resp.clicked(), pointer) {
            if handle_at(pos).is_none() {
                self.commit();
                self.edit_path(id, |m| m.points.push(to_path(pos)));
                self.touch();
            }
        }
        if let (true, Some(pos)) = (resp.secondary_clicked(), pointer) {
            if let Some(i) = handle_at(pos) {
                self.commit();
                self.edit_path(id, |m| {
                    m.points.remove(i);
                });
                self.touch();
            }
        }
    }

    /// Правит путь клипа; у клипа без пути он создаётся, опустевший путь убирается.
    fn edit_path(&mut self, id: Id, edit: impl FnOnce(&mut Motion)) {
        let Some(clip) = self.project.clip_mut(id) else { return };
        let motion = clip.motion.get_or_insert_with(Motion::default);
        edit(motion);
        if motion.points.is_empty() {
            clip.motion = None;
        }
        self.dirty = true;
    }

    /// Инструменты кадра на странице VFX: пипетка, маска и режимы просмотра.
    fn stage_tools(&mut self, ui: &mut Ui, header: Rect, opacity: f32) {
        let clip = self.selected.and_then(|id| self.project.clip(id));
        let (any, stylable) = (clip.is_some(), clip.map_or(false, |c| self.stylable(c)));
        let has_mask = clip.map_or(false, |c| c.vfx.mask.is_some());
        let area = Rect::from_min_max(pos2(header.center().x - 80.0, header.top() + 4.0), pos2(header.right() - 8.0, header.bottom() - 4.0));
        let mut mask_clicked = false;
        ui.allocate_new_ui(UiBuilder::new().max_rect(area).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.multiply_opacity(opacity);
            ui.spacing_mut().item_spacing.x = 2.0;
            let v = &mut self.vfx_ui;
            if widgets::icon_button(ui, icons::COMPARE, 28.0, "До и после: показать слой без оформления", v.original && any, any).clicked() {
                v.original = !v.original;
            }
            if widgets::icon_button(ui, icons::LAYERS, 28.0, "Только выбранный слой на прозрачном фоне", v.solo && any, any).clicked() {
                v.solo = !v.solo;
            }
            if widgets::icon_button(ui, icons::MATTE, 28.0, "Вырез: белое остаётся в кадре, чёрное вырезано", v.matte && stylable, stylable).clicked() {
                v.matte = !v.matte;
            }
            widgets::tool_separator(ui);
            let tip = "Маска: двигать и растягивать прямо в кадре";
            mask_clicked = widgets::icon_button(ui, icons::SHAPE, 28.0, tip, v.tool == VfxTool::Mask, stylable).clicked();
            let picking = v.tool == VfxTool::Pipette;
            if widgets::icon_button(ui, icons::PIPETTE, 28.0, "Пипетка: взять цвет фона для хромакея из кадра", picking, stylable).clicked() {
                v.tool = if picking { VfxTool::Move } else { VfxTool::Pipette };
            }
        });
        if !mask_clicked {
            return;
        }
        if self.vfx_ui.tool == VfxTool::Mask {
            self.vfx_ui.tool = VfxTool::Move;
        } else if let Some(id) = self.selected {
            // У клипа без маски она появляется сразу: овал посередине кадра.
            if !has_mask {
                self.commit();
                if let Some(clip) = self.project.clip_mut(id) {
                    clip.vfx.mask = Some(Mask::new(MaskShape::Ellipse));
                }
                self.touch();
                self.vfx_ui.open[2] = true;
            }
            self.vfx_ui.tool = VfxTool::Mask;
        }
    }

    /// Контуры кадрирования и маски выбранного клипа поверх кадра.
    fn shape_overlay(&self, ui: &Ui, stage: Rect, p: &Placed, l: &Layer) {
        let Some(clip) = self.project.clip(l.clip) else { return };
        let painter = ui.painter().with_clip_rect(stage);
        let at = |u: f32, v: f32| p.at(u, v, l.flip_h, l.flip_v);
        let [left, top, right, bottom] = clip.vfx.crop;
        if clip.vfx.crop.iter().any(|c| *c > 1e-4) {
            let frame = [at(left, top), at(1.0 - right, top), at(1.0 - right, 1.0 - bottom), at(left, 1.0 - bottom), at(left, top)];
            painter.extend(Shape::dashed_line(&frame, Stroke::new(1.0, Color32::WHITE), 5.0, 4.0));
        }
        let Some(mask) = &clip.vfx.mask else { return };
        let outline: Vec<Pos2> = match mask.shape {
            MaskShape::Rect => mask_corners(mask).map(|(u, v)| at(u, v)).to_vec(),
            MaskShape::Ellipse => (0..48)
                .map(|i| {
                    let a = TAU * i as f32 / 48.0;
                    at(mask.x + a.cos() * mask.w / 2.0, mask.y + a.sin() * mask.h / 2.0)
                })
                .collect(),
        };
        let editing = self.vfx_ui.tool == VfxTool::Mask;
        painter.add(Shape::closed_line(outline.clone(), Stroke::new(3.5, Color32::from_black_alpha(110))));
        painter.add(Shape::closed_line(outline, Stroke::new(1.5, if editing { ACCENT } else { Color32::WHITE })));
        if !editing {
            return;
        }
        // Рамка фигуры с ручками по углам и точка в середине.
        let corners = mask_corners(mask).map(|(u, v)| at(u, v));
        let mut frame = corners.to_vec();
        frame.push(corners[0]);
        painter.extend(Shape::dashed_line(&frame, Stroke::new(1.0, ACCENT.gamma_multiply(0.7)), 4.0, 4.0));
        for c in corners {
            let h = Rect::from_center_size(c, vec2(9.0, 9.0));
            painter.rect_filled(h, 2.0, Color32::WHITE);
            painter.rect_stroke(h, 2.0, Stroke::new(1.5, ACCENT));
        }
        painter.circle(at(mask.x, mask.y), 4.0, ACCENT, Stroke::new(1.5, ON_ACCENT));
    }

    /// Инструмент страницы VFX в кадре; `true` — мышь в кадре занята им.
    fn stage_tool(&mut self, ui: &Ui, resp: &Response, stage: Rect, p: &Placed, l: &Layer) -> bool {
        let id = l.clip;
        let uv = |pos: Pos2| p.uv(pos, l.flip_h, l.flip_v);
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|pos| stage.contains(*pos));
        match self.vfx_ui.tool {
            VfxTool::Move => return false,
            VfxTool::Pipette => {
                stage_tip(ui, stage, "Щёлкните по фону в кадре: этот цвет станет прозрачным  ·  Esc — отмена");
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                }
                let picked = hover.and_then(|pos| {
                    let (u, v) = uv(pos);
                    self.preview.pick(id, u, v).map(|color| (pos, color))
                });
                if let Some((pos, color)) = picked {
                    // Образец цвета под указателем.
                    let painter = ui.painter().with_clip_rect(stage);
                    let swatch = pos + vec2(24.0, -24.0);
                    painter.circle_filled(swatch, 16.0, Color32::from_black_alpha(150));
                    painter.circle(swatch, 13.0, Color32::from_rgb(color[0], color[1], color[2]), Stroke::new(1.5, Color32::WHITE));
                    if resp.clicked() {
                        self.commit();
                        if let Some(clip) = self.project.clip_mut(id) {
                            match &mut clip.look.key {
                                Some(key) => key.color = color,
                                None => clip.look.key = Some(ChromaKey::studio(color)),
                            }
                        }
                        self.touch();
                        self.vfx_ui.tool = VfxTool::Move;
                        self.vfx_ui.open[0] = true;
                        fx::ring(ui.ctx(), pos, 4.0, 28.0, ACCENT);
                    }
                } else if resp.clicked() {
                    self.toast("Пипетка берёт цвет из кадра выбранного видео или картинки");
                }
            }
            VfxTool::Mask => {
                let Some(mask) = self.project.clip(id).and_then(|c| c.vfx.mask.clone()) else {
                    self.vfx_ui.tool = VfxTool::Move;
                    return false;
                };
                stage_tip(ui, stage, "Тяните маску, чтобы подвинуть её, за угол — чтобы изменить размер  ·  Esc — готово");
                let on_corner = |pos: Pos2| mask_corners(&mask).iter().any(|(u, v)| (p.at(*u, *v, l.flip_h, l.flip_v) - pos).length() <= 10.0);
                if let Some(pos) = hover.filter(|_| resp.hovered()) {
                    ui.ctx().set_cursor_icon(if on_corner(pos) { CursorIcon::ResizeNwSe } else { CursorIcon::Grab });
                }
                if resp.drag_started() {
                    self.stage_drag = None;
                    if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                        self.commit();
                        self.stage_drag = Some(if on_corner(origin) {
                            StageDrag::MaskSize
                        } else {
                            StageDrag::MaskMove { grab: uv(origin), x: mask.x, y: mask.y }
                        });
                    }
                }
                if let (true, Some(pos)) = (resp.dragged(), resp.interact_pointer_pos()) {
                    let (u, v) = uv(pos);
                    match (&self.stage_drag, self.project.clip_mut(id).and_then(|c| c.vfx.mask.as_mut())) {
                        (Some(StageDrag::MaskMove { grab, x, y }), Some(m)) => {
                            m.x = (x + u - grab.0).clamp(-0.5, 1.5);
                            m.y = (y + v - grab.1).clamp(-0.5, 1.5);
                            self.dirty = true;
                        }
                        (Some(StageDrag::MaskSize), Some(m)) => {
                            // Угол следует за указателем, середина остаётся на месте.
                            m.w = (2.0 * (u - m.x).abs()).clamp(0.02, 2.0);
                            m.h = (2.0 * (v - m.y).abs()).clamp(0.02, 2.0);
                            self.dirty = true;
                        }
                        _ => {}
                    }
                }
                if resp.drag_stopped() {
                    self.stage_drag = None;
                    self.touch();
                }
            }
        }
        true
    }

    pub fn player_ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 38.0));
        let transport = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 46.0), rect.max);
        let stage = Rect::from_min_max(header.left_bottom(), transport.right_top());
        let vfx_page = self.page == Page::Vfx;

        ui.painter().text(
            pos2(header.left() + 14.0, header.center().y),
            Align2::LEFT_CENTER,
            "Плеер",
            theme::font_semibold(13.0),
            TEXT,
        );
        // На странице VFX место сведений о кадре занимают инструменты.
        let tools = ui.ctx().animate_bool_with_time(ui.id().with("vfx-tools"), vfx_page, 0.18);
        if tools < 1.0 {
            ui.painter().text(
                pos2(header.right() - 14.0, header.center().y),
                Align2::RIGHT_CENTER,
                format!("{}×{} · {} к/с", self.project.width, self.project.height, self.project.fps),
                theme::font_mono(11.5),
                TEXT3.gamma_multiply(1.0 - tools),
            );
        }
        if vfx_page {
            self.stage_tools(ui, header, tools);
        }
        // Режимы просмотра действуют на выбранный клип и только на странице VFX.
        let focus = self.selected.filter(|_| vfx_page);
        self.preview.matte = focus.filter(|_| self.vfx_ui.matte);
        self.preview.original = focus.filter(|_| self.vfx_ui.original);

        ui.painter().rect_filled(stage, 0.0, BG);
        let area = stage.shrink(12.0);
        let (cw, ch) = fit_size(self.project.width as f32, self.project.height as f32, area.width(), area.height());
        let canvas = Rect::from_center_size(area.center(), vec2(cw, ch));
        let k = cw / self.project.width as f32;
        self.canvas = Some(canvas);

        let layers = self.preview.update(ui.ctx(), &self.project, self.playhead, self.playing);
        let painter = ui.painter().with_clip_rect(canvas);
        painter.rect_filled(canvas, 0.0, Color32::BLACK);
        // «Только слой»: остальные слои скрыты, а прозрачное видно шахматной клеткой.
        let solo = focus.filter(|id| self.vfx_ui.solo && layers.iter().any(|l| l.clip == *id));
        if solo.is_some() {
            checkerboard(&painter, canvas);
        }
        for l in layers.iter().filter(|l| solo.map_or(true, |id| l.clip == id)) {
            let p = Placed::new(l, canvas, k);
            let mut mesh = Mesh::with_texture(l.texture);
            // Отражённый клип: текстура читается в обратную сторону.
            let (u, v) = (if l.flip_h { 1.0 } else { 0.0 }, if l.flip_v { 1.0 } else { 0.0 });
            mesh.add_rect_with_uv(
                Rect::from_center_size(p.center, p.size),
                Rect::from_min_max(pos2(u, v), pos2(1.0 - u, 1.0 - v)),
                Color32::WHITE.gamma_multiply(l.opacity.clamp(0.0, 1.0)),
            );
            if p.angle != 0.0 {
                mesh.rotate(Rot2::from_angle(p.angle), p.center);
            }
            painter.add(Shape::mesh(mesh));
            if l.flash > 0.0 {
                painter.rect_filled(canvas, 0.0, Color32::WHITE.gamma_multiply(l.flash.min(1.0)));
            }
        }
        if self.project.is_empty() {
            ui.painter().text(
                canvas.center(),
                Align2::CENTER_CENTER,
                "Добавьте клипы на таймлайн",
                theme::font(13.0),
                TEXT3,
            );
        }

        // Выделение и перетаскивание клипа прямо в кадре.
        let resp = ui.interact(stage, ui.id().with("stage"), Sense::click_and_drag());
        let selected = self.selected.and_then(|id| layers.iter().find(|l| l.clip == id));
        if let Some(l) = selected {
            let p = Placed::new(l, canvas, k);
            let corners = p.corners();
            let overlay = ui.painter().with_clip_rect(stage);
            overlay.add(Shape::closed_line(corners.to_vec(), Stroke::new(1.5, ACCENT)));
            for c in corners {
                let h = Rect::from_center_size(c, vec2(8.0, 8.0));
                overlay.rect_filled(h, 1.0, Color32::WHITE);
                overlay.rect_stroke(h, 1.0, Stroke::new(1.0, ACCENT));
            }
        }

        // Путь движения выбранного клипа; пока его рисуют, мышь в кадре работает только на него.
        let path_of = self.selected.and_then(|id| self.project.clip(id)).filter(|c| {
            self.path_edit == Some(c.id) || c.motion.is_some()
        });
        let (now, reach) = (self.playhead, self.project.key_reach());
        if let Some((id, x, y)) = path_of.map(|c| (c.id, c.value(Prop::PosX, now), c.value(Prop::PosY, now))) {
            self.path_ui(ui, &resp, stage, canvas.center() + vec2(x, y) * k, k, id);
        }
        if self.path_edit.is_some() {
            return self.transport_ui(ui, transport);
        }
        // Страница VFX: контуры маски и инструменты кадра.
        if vfx_page {
            if let Some(l) = selected {
                let placed = Placed::new(l, canvas, k);
                self.shape_overlay(ui, stage, &placed, l);
                if self.stage_tool(ui, &resp, stage, &placed, l) {
                    return self.transport_ui(ui, transport);
                }
            } else if self.vfx_ui.tool != VfxTool::Move {
                stage_tip(ui, stage, "Поставьте курсор времени на клип и выберите его: инструмент работает в его кадре");
            }
        }

        if resp.drag_started() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                self.stage_drag = None;
                // Перетаскивание меняет собственные положение и масштаб клипа, а не те,
                // что получились из-за пути и перехода.
                let own = self.selected.and_then(|id| self.project.clip(id)).map(|c| {
                    (c.value(Prop::PosX, now), c.value(Prop::PosY, now), c.value(Prop::Scale, now), !c.keys.is_empty())
                });
                if let (Some(l), Some((x, y, scale, pinned))) = (selected, own) {
                    let p = Placed::new(l, canvas, k);
                    let on_corner = p.corners().iter().any(|c| (*c - origin).length() <= 9.0);
                    if on_corner {
                        self.commit();
                        self.stage_drag = Some(StageDrag::Scale { dist: (origin - p.center).length().max(1.0), scale });
                    } else if p.contains(origin) {
                        self.commit();
                        self.stage_drag = Some(StageDrag::Move { origin, x, y });
                    }
                    // Закреплённое значение правят в одном моменте: воспроизведение при этом останавливается.
                    if pinned && self.stage_drag.is_some() && self.playing {
                        self.pause();
                    }
                }
            }
        }
        if resp.dragged() {
            if let (Some(pos), Some(id)) = (resp.interact_pointer_pos(), self.selected) {
                let center = selected.map(|l| Placed::new(l, canvas, k).center);
                match (&self.stage_drag, self.project.clip_mut(id)) {
                    (Some(StageDrag::Move { origin, x, y }), Some(clip)) => {
                        let d = (pos - *origin) / k;
                        clip.set_value(Prop::PosX, now, (x + d.x).round(), reach);
                        clip.set_value(Prop::PosY, now, (y + d.y).round(), reach);
                        self.dirty = true;
                    }
                    (Some(StageDrag::Scale { dist, scale }), Some(clip)) => {
                        if let Some(c) = center {
                            clip.set_value(Prop::Scale, now, (scale * (pos - c).length() / dist).clamp(0.05, 8.0), reach);
                            self.dirty = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() {
            self.stage_drag = None;
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.sel_transition = None;
                self.selected = layers
                    .iter()
                    .rev()
                    .find(|l| Placed::new(l, canvas, k).contains(pos))
                    .map(|l| l.clip);
            }
        }

        self.transport_ui(ui, transport);
    }

    /// Управление воспроизведением под кадром.
    fn transport_ui(&mut self, ui: &mut Ui, transport: Rect) {
        let fps = self.project.fps;
        let tc = timecode(self.playhead, fps);
        let total = timecode(self.project.duration(), fps);
        let tc_galley = ui.painter().layout_no_wrap(tc, theme::font_mono(12.0), TEXT);
        let tc_w = tc_galley.size().x;
        let y = transport.center().y;
        ui.painter().galley(pos2(transport.left() + 14.0, y - tc_galley.size().y / 2.0), tc_galley, TEXT);
        ui.painter().text(
            pos2(transport.left() + 14.0 + tc_w, y),
            Align2::LEFT_CENTER,
            format!(" / {total}"),
            theme::font_mono(12.0),
            TEXT3,
        );

        let cx = transport.center().x;
        let prev = Rect::from_center_size(pos2(cx - 44.0, y), vec2(32.0, 32.0));
        let next = Rect::from_center_size(pos2(cx + 44.0, y), vec2(32.0, 32.0));
        let play = Rect::from_center_size(pos2(cx, y), vec2(36.0, 36.0));
        if widgets::icon_button_at(ui, prev, "prev-frame", icons::PREV_FRAME, "Кадр назад (←)", false).clicked() {
            self.step_frames(-1);
        }
        if widgets::icon_button_at(ui, next, "next-frame", icons::NEXT_FRAME, "Кадр вперёд (→)", false).clicked() {
            self.step_frames(1);
        }
        let play_resp = ui
            .interact(play, ui.id().with("play"), Sense::click())
            .on_hover_text(if self.playing { "Пауза (пробел)" } else { "Воспроизвести (пробел)" });
        // Кнопка отзывается на запуск и остановку, откуда бы они ни пришли: щелчок или пробел.
        let ctx = ui.ctx();
        let toggled = fx::changed(ctx, play_resp.id.with("playing"), self.playing);
        if toggled {
            fx::ring(ctx, play.center(), 18.0, 36.0, if self.playing { ACCENT } else { TEXT });
        }
        let pop = fx::spring(ctx, play_resp.id.with("pop"), 0.0, if toggled { fx::KICK } else { 0.0 });
        let scale = fx::press(ctx, &play_resp, 0.1) + 0.14 * pop;
        let on = ctx.animate_bool_with_time(play_resp.id.with("on"), self.playing, 0.16);
        let bg = TEXT.lerp_to_gamma(Color32::WHITE, fx::hover(ctx, &play_resp)).lerp_to_gamma(ACCENT, on);
        fx::halo(ui.painter(), fx::zoom(play, play.center(), scale), 18.0, ACCENT.gamma_multiply(0.35 * on), 18.0);
        ui.painter().circle_filled(play.center(), 18.0 * scale, bg);
        let glyph = Rect::from_center_size(play.center() + vec2(if self.playing { 0.0 } else { 1.0 }, 0.0), vec2(16.0, 16.0));
        let icon = if self.playing { icons::PAUSE } else { icons::PLAY };
        icons::paint_filled(ui.painter(), fx::zoom(glyph, play.center(), scale), icon, PANEL);
        if play_resp.clicked() {
            self.toggle_play();
        }

        if !self.audio.available() {
            ui.painter().text(
                pos2(transport.right() - 14.0, y),
                Align2::RIGHT_CENTER,
                "нет устройства вывода звука",
                theme::font(11.5),
                TEXT3,
            );
        }
    }
}
