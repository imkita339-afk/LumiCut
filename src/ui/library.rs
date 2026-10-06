//! Левая панель: разделы, библиотека медиа, эффекты и переходы.

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, ColorImage, Layout, Painter, Rect, Response, ScrollArea, Sense,
    Shape, Stroke, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};

use crate::app::{App, Category, EffectDrag, InspectorTab, MediaDrag, MediaFilter, TextDrag, TransitionDrag};
use crate::effects::{self, Effect, EffectKind};
use crate::icons;
use crate::look::{FilterKind, Look};
use crate::model::{fit_size, short_time, Id, MediaKind};
use crate::text::{self, Text, TextPreset};
use crate::theme::{self, *};
use crate::transition::TransitionKind;
use crate::ui::{fx, widgets};

const RAIL_W: f32 = 72.0;
/// Плиток в ряду, зазор между ними и высота подписи под плиткой.
pub(super) const COLUMNS: usize = 3;
const GAP: f32 = 8.0;
const CAPTION_H: f32 = 22.0;
/// Сторона кадра эффекта на плитке, в точках эффекта.
const EFFECT_THUMB: u32 = 48;
/// Высота кадра, под которую рисуется образец надписи для плитки.
const TEXT_THUMB_FRAME: u32 = 360;
/// Размер картинки-образца на плитках фильтров.
const SAMPLE: [usize; 2] = [96, 54];

/// Картинка-образец для плиток фильтров и эффектов: небо, солнце, холмы и дом с красной крышей.
pub(super) fn sample(size: [usize; 2]) -> Vec<u8> {
    let [w, h] = size;
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let sun = ((u - 0.76) * w as f32 / h as f32).hypot(v - 0.27) < 0.12;
            let (near, far) = (0.66 + 0.07 * (u * 7.0).sin(), 0.54 + 0.06 * (u * 4.0 + 1.0).cos());
            let wall = (u - 0.3).abs() < 0.085 && v > 0.47 && v < 0.69;
            let roof = v <= 0.47 && v > 0.33 + (u - 0.3).abs() * 1.25;
            let c: [f32; 3] = if roof {
                [203.0, 60.0, 46.0]
            } else if wall {
                [238.0, 226.0, 204.0]
            } else if v > near {
                [52.0 + 30.0 * (1.0 - v), 138.0 - 60.0 * (v - near), 62.0]
            } else if v > far {
                [70.0, 124.0, 104.0]
            } else if sun {
                [255.0, 216.0, 112.0]
            } else {
                [78.0 + 120.0 * v / 0.6, 138.0 + 80.0 * v / 0.6, 224.0 + 16.0 * v / 0.6]
            };
            out.extend(c.map(|k| k.clamp(0.0, 255.0) as u8));
            out.push(255);
        }
    }
    out
}

/// Время реплики: минуты, секунды и десятые.
fn clock(t: f64) -> String {
    format!("{:02}:{:04.1}", (t / 60.0).floor() as u64, t % 60.0)
}

/// Файл библиотеки, как он показан на плитке.
struct Item {
    id: Id,
    name: String,
    kind: MediaKind,
    duration: f64,
    /// Расширение прописными буквами: по нему файлы собраны в группы.
    ext: String,
}

/// Плитка библиотеки: место в сетке и то же место с учётом наведения и нажатия.
pub(super) struct Tile {
    pub(super) resp: Response,
    pub(super) slot: Rect,
    pub(super) rect: Rect,
    pub(super) hover: f32,
}

pub(super) fn tile_size(width: f32) -> Vec2 {
    let w = ((width - (COLUMNS - 1) as f32 * GAP) / COLUMNS as f32).floor();
    vec2(w, (w * 9.0 / 16.0).round())
}

/// Ряд плиток во всю ширину панели.
pub(super) fn tile_row(ui: &mut Ui, size: Vec2) -> Rect {
    ui.allocate_exact_size(vec2(ui.available_width(), size.y + CAPTION_H + 10.0), Sense::hover()).0
}

pub(super) fn tile(ui: &mut Ui, row: Rect, column: usize, size: Vec2, id: impl std::hash::Hash) -> Tile {
    let slot = Rect::from_min_size(pos2(row.left() + column as f32 * (size.x + GAP), row.top()), size);
    let hit = Rect::from_min_max(slot.min, pos2(slot.right(), slot.bottom() + CAPTION_H));
    let resp = ui.interact(hit, ui.id().with(id), Sense::click_and_drag());
    // Плитка приподнимается под указателем и проседает под нажатием.
    let hover = fx::hover(ui.ctx(), &resp);
    let rect = fx::zoom(slot, slot.center(), fx::press(ui.ctx(), &resp, 0.06) * (1.0 + 0.025 * hover));
    if resp.double_clicked() {
        fx::tap(ui.ctx(), slot, 6.0, ACCENT);
    }
    ui.painter().rect_filled(rect, 6.0, RAISED);
    Tile { resp, slot, rect, hover }
}

/// Рамка наведения и подпись под плиткой.
pub(super) fn tile_caption(ui: &Ui, tile: &Tile, name: &str) {
    if tile.hover > 0.0 {
        ui.painter().rect_stroke(tile.rect, 6.0, Stroke::new(1.5, TEXT2.gamma_multiply(tile.hover)));
    }
    let galley = widgets::elided(ui, name, theme::font(11.5), TEXT2, tile.slot.width());
    ui.painter().galley(pos2(tile.slot.left(), tile.slot.bottom() + 5.0), galley, TEXT2);
}

pub(super) fn panel_title(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(title).font(theme::font_semibold(14.0)).color(TEXT));
    });
}

pub(super) fn panel_hint(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::font(11.5)).color(TEXT3));
    ui.add_space(8.0);
}

/// Заголовок группы файлов: щелчок сворачивает и разворачивает её.
fn group_header(ui: &mut Ui, label: &str, count: usize, open: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let ctx = ui.ctx();
    let color = TEXT2.lerp_to_gamma(TEXT, fx::hover(ctx, &resp));
    // Стрелка смотрит вниз у раскрытой группы и вправо у свёрнутой.
    let turn = ctx.animate_bool_with_time(resp.id.with("open"), open, 0.12);
    let (center, angle) = (pos2(rect.left() + 6.0, rect.center().y), (turn - 1.0) * std::f32::consts::FRAC_PI_2);
    let arrow = [vec2(-3.5, -2.0), vec2(0.0, 2.0), vec2(3.5, -2.0)].map(|v| center + egui::emath::Rot2::from_angle(angle) * v);
    ui.painter().add(Shape::line(arrow.to_vec(), Stroke::new(1.4, color)));

    let name = ui.painter().layout_no_wrap(label.to_owned(), theme::font_semibold(11.5), color);
    let number = ui.painter().layout_no_wrap(count.to_string(), theme::font_mono(10.5), TEXT3);
    let (name_w, number_w) = (name.size().x, number.size().x);
    let x = rect.left() + 18.0;
    ui.painter().galley(pos2(x, rect.center().y - name.size().y / 2.0), name, color);
    ui.painter().galley(pos2(x + name_w + 7.0, rect.center().y - number.size().y / 2.0), number, TEXT3);
    let line_from = x + name_w + number_w + 16.0;
    if line_from < rect.right() {
        ui.painter().line_segment(
            [pos2(line_from, rect.center().y), pos2(rect.right(), rect.center().y)],
            Stroke::new(1.0, LINE),
        );
    }
    resp
}

/// Миниатюра перехода: «кадры» A и B в том виде, какой им придаёт переход на доле `progress`.
fn transition_demo(p: &Painter, rect: Rect, kind: TransitionKind, progress: f32) {
    const FRAME_A: Color32 = Color32::from_rgb(0x3B, 0x6E, 0xA8);
    const FRAME_B: Color32 = Color32::from_rgb(0xC9, 0x8A, 0x2B);
    let mut flash = 0.0f32;
    for incoming in [false, true] {
        // Кадры, которые не показываются одновременно, сменяют друг друга в середине перехода.
        if !kind.overlaps() && incoming != (progress >= 0.5) {
            continue;
        }
        let t = kind.tweak(incoming, progress, rect.width(), rect.height());
        let frame = fx::zoom(rect, rect.center(), t.scale).translate(vec2(t.dx, t.dy));
        let (fill, letter) = if incoming { (FRAME_B, "B") } else { (FRAME_A, "A") };
        p.rect_filled(frame, 6.0, fill.gamma_multiply(t.alpha));
        p.text(
            frame.center(),
            Align2::CENTER_CENTER,
            letter,
            theme::font_semibold(15.0 * t.scale),
            Color32::WHITE.gamma_multiply(0.92 * t.alpha),
        );
        flash = flash.max(t.flash);
    }
    if flash > 0.0 {
        p.rect_filled(rect, 6.0, Color32::WHITE.gamma_multiply(flash));
    }
}

impl App {
    pub fn library_ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();

        // Рейка разделов.
        let rail = Rect::from_min_size(rect.min, vec2(RAIL_W, rect.height()));
        ui.painter().rect_filled(rail, widgets::round_left(10.0), RAIL);
        ui.painter().line_segment([rail.right_top(), rail.right_bottom()], Stroke::new(1.0, LINE));
        let ctx = ui.ctx().clone();
        // Подложка активного раздела переезжает к новому.
        let index = Category::ALL.iter().position(|c| *c == self.lib.category).unwrap_or(0);
        let pill_y = ctx.animate_value_with_time(ui.id().with("rail-pill"), rail.top() + 6.0 + index as f32 * 54.0, 0.16);
        let pill = Rect::from_min_size(pos2(rail.left() + 6.0, pill_y), vec2(RAIL_W - 12.0, 52.0));
        ui.painter().rect_filled(pill, 8.0, ACTIVE);
        let mark = Rect::from_min_max(pos2(pill.left(), pill.top() + 15.0), pos2(pill.left() + 3.0, pill.bottom() - 15.0));
        ui.painter().rect_filled(mark, 1.5, ACCENT);

        let mut y = rail.top() + 6.0;
        for cat in Category::ALL {
            let r = Rect::from_min_size(pos2(rail.left() + 6.0, y), vec2(RAIL_W - 12.0, 52.0));
            let resp = ui.interact(r, ui.id().with(("category", cat)), Sense::click());
            let active = self.lib.category == cat;
            let hover = fx::hover(&ctx, &resp);
            let on = ctx.animate_bool_with_time(resp.id.with("on"), active, 0.14);
            let scale = fx::press(&ctx, &resp, 0.08);
            if !active {
                ui.painter().rect_filled(r, 8.0, Color32::TRANSPARENT.lerp_to_gamma(RAISED, hover));
            }
            let color = TEXT2.lerp_to_gamma(TEXT, hover).lerp_to_gamma(ACCENT, on);
            let icon = Rect::from_center_size(pos2(r.center().x, r.top() + 19.0), vec2(20.0, 20.0));
            icons::paint(ui.painter(), fx::zoom(icon, r.center(), scale), cat.icon(), color);
            let galley = ui.painter().layout_no_wrap(cat.label().to_owned(), theme::font(11.0), color);
            let pos = pos2(r.center().x, r.bottom() - 11.0) - galley.size() / 2.0;
            fx::text(ui.painter(), pos, galley, color, r.center(), scale);
            if resp.clicked() {
                self.lib.category = cat;
                fx::tap(&ctx, r, 8.0, TEXT);
            }
            y += 54.0;
        }

        let content = Rect::from_min_max(pos2(rail.right() + 1.0, rect.top()), rect.max);
        let inner = content.shrink2(vec2(12.0, 0.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
            ui.set_clip_rect(content.intersect(ui.clip_rect()));
            // Содержимое раздела проявляется и подъезжает снизу, когда раздел сменили.
            let category = self.lib.category;
            fx::appear(ui, egui::Id::new("library-body"), category, vec2(0.0, 10.0), |ui| match category {
                Category::Media | Category::Audio => self.media_panel(ui),
                Category::Effects => self.effects_panel(ui),
                Category::Transitions => self.transitions_panel(ui),
                Category::Text => self.text_panel(ui),
                Category::Filters => self.filters_panel(ui),
                Category::Subtitles => self.captions_panel(ui),
                other => placeholder_panel(ui, other),
            });
        });
    }

    fn media_panel(&mut self, ui: &mut Ui) {
        let audio_only = self.lib.category == Category::Audio;
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(self.lib.category.label())
                    .font(theme::font_semibold(14.0))
                    .color(TEXT),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::soft_button(ui, Some(icons::PLUS), "Импорт").clicked() {
                    self.import_dialog();
                }
                if self.lib.importing > 0 {
                    ui.spinner();
                }
            });
        });
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.lib.search)
                .hint_text("Поиск")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        if !audio_only {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (f, label) in [
                    (MediaFilter::All, "Все"),
                    (MediaFilter::Video, "Видео"),
                    (MediaFilter::Photo, "Фото"),
                    (MediaFilter::Audio, "Аудио"),
                ] {
                    if widgets::chip(ui, label, self.lib.filter == f).clicked() {
                        self.lib.filter = f;
                    }
                }
            });
            ui.add_space(6.0);
        }

        let needle = self.lib.search.trim().to_lowercase();
        let filter = if audio_only { MediaFilter::Audio } else { self.lib.filter };
        let items: Vec<Item> = self
            .project
            .media
            .iter()
            .filter(|m| match filter {
                MediaFilter::All => true,
                MediaFilter::Video => m.kind == MediaKind::Video,
                MediaFilter::Photo => m.kind == MediaKind::Image,
                MediaFilter::Audio => m.kind == MediaKind::Audio,
            })
            .filter(|m| needle.is_empty() || m.name.to_lowercase().contains(&needle))
            .map(|m| Item {
                id: m.id,
                name: m.name.clone(),
                kind: m.kind,
                duration: m.duration,
                ext: m.path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default(),
            })
            .collect();

        if items.is_empty() {
            let r = ui.available_rect_before_wrap();
            let text = if self.project.media.is_empty() {
                "Перетащите сюда файлы\nили нажмите «Импорт»"
            } else {
                "Ничего не найдено"
            };
            ui.painter().text(
                pos2(r.center().x, r.top() + 90.0),
                Align2::CENTER_CENTER,
                text,
                theme::font(12.5),
                TEXT3,
            );
            return;
        }

        // Во вкладке «Все» файлы собраны в группы по расширению: сначала видео, потом фото и звук.
        let mut groups: Vec<(Option<String>, Vec<&Item>)> = Vec::new();
        if filter == MediaFilter::All {
            let order = |kind: MediaKind| match kind {
                MediaKind::Video => 0,
                MediaKind::Image => 1,
                MediaKind::Audio => 2,
            };
            let mut sorted: Vec<&Item> = items.iter().collect();
            sorted.sort_by(|a, b| (order(a.kind), &a.ext).cmp(&(order(b.kind), &b.ext)));
            for item in sorted {
                match groups.last_mut() {
                    Some((Some(ext), list)) if *ext == item.ext => list.push(item),
                    _ => groups.push((Some(item.ext.clone()), vec![item])),
                }
            }
        } else {
            groups.push((None, items.iter().collect()));
        }

        let mut append: Option<Id> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for (ext, list) in &groups {
                let mut shown = 1.0;
                if let Some(ext) = ext {
                    let open = !self.lib.collapsed.contains(ext);
                    let label = if ext.is_empty() { "БЕЗ РАСШИРЕНИЯ" } else { ext.as_str() };
                    if group_header(ui, label, list.len(), open).clicked() {
                        if open {
                            self.lib.collapsed.insert(ext.clone());
                        } else {
                            self.lib.collapsed.remove(ext);
                        }
                    }
                    // Раскрытая группа проявляется, а не выскакивает.
                    shown = fx::enter(ui.ctx(), egui::Id::new(("group", ext)), open);
                    if !open {
                        continue;
                    }
                    ui.add_space(2.0);
                }
                ui.scope(|ui| {
                    ui.multiply_opacity(shown);
                    for row in list.chunks(COLUMNS) {
                        let row_rect = tile_row(ui, size);
                        for (i, item) in row.iter().enumerate() {
                            if self.media_tile(ui, row_rect, i, size, item) {
                                append = Some(item.id);
                            }
                        }
                    }
                });
            }
        });

        if let Some(id) = append {
            self.commit();
            if let Some(clip) = self.project.append_clip(id) {
                self.selected = Some(clip);
                self.touch();
                self.fx_land(clip, 1.0);
            } else {
                self.history.undo(&mut self.project);
            }
        }
    }

    /// Плитка файла; `true`, если по ней дважды щёлкнули.
    fn media_tile(&self, ui: &mut Ui, row: Rect, column: usize, size: Vec2, item: &Item) -> bool {
        let tile = tile(ui, row, column, size, ("media", item.id));
        match self.lib.thumbs.get(&item.id) {
            Some(tex) => {
                // Обрезаем миниатюру под 16:9.
                let have = tex.size_vec2();
                let (mut u, mut v) = (1.0, 1.0);
                let want = size.x / size.y;
                let ratio = have.x / have.y;
                if ratio > want {
                    u = want / ratio;
                } else {
                    v = ratio / want;
                }
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
                egui::Image::new(tex).uv(uv).rounding(6.0).paint_at(ui, tile.rect);
            }
            None => {
                let icon = match item.kind {
                    MediaKind::Audio => icons::NOTE,
                    MediaKind::Image => icons::IMAGE,
                    MediaKind::Video => icons::FILM,
                };
                icons::paint(ui.painter(), Rect::from_center_size(tile.rect.center(), vec2(22.0, 22.0)), icon, TEXT2);
            }
        }
        if item.kind != MediaKind::Image {
            let galley = ui.painter().layout_no_wrap(short_time(item.duration), theme::font_mono(10.5), TEXT);
            let badge = Rect::from_min_size(
                pos2(tile.rect.right() - galley.size().x - 12.0, tile.rect.bottom() - 19.0),
                galley.size() + vec2(8.0, 3.0),
            );
            ui.painter().rect_filled(badge, 3.0, Color32::from_black_alpha(190));
            ui.painter().galley(badge.min + vec2(4.0, 1.5), galley, TEXT);
        }
        tile_caption(ui, &tile, &item.name);

        if tile.resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), MediaDrag(item.id));
        }
        let picked = tile.resp.double_clicked();
        tile.resp.on_hover_text(format!(
            "{}\nДвойной щелчок — добавить в конец, перетаскивание — на таймлайн",
            item.name
        ));
        picked
    }

    /// Текстура с кадром эффекта для плитки; пересчитывается, когда меняется показываемая доля эффекта.
    fn effect_thumb(&mut self, ctx: &egui::Context, kind: EffectKind, progress: f32) -> TextureHandle {
        if let Some((shown, tex)) = self.lib.effect_thumbs.get(&kind) {
            if *shown == progress {
                return tex.clone();
            }
        }
        let effect = Effect { res: EFFECT_THUMB, ..Effect::new(kind, 1) };
        let image = ColorImage::from_rgba_unmultiplied([EFFECT_THUMB as usize; 2], &effects::render(&effect, progress));
        let tex = match self.lib.effect_thumbs.remove(&kind) {
            Some((_, mut tex)) => {
                tex.set(image, TextureOptions::NEAREST);
                tex
            }
            None => ctx.load_texture(format!("effect-thumb-{kind:?}"), image, TextureOptions::NEAREST),
        };
        self.lib.effect_thumbs.insert(kind, (progress, tex.clone()));
        tex
    }

    fn effects_panel(&mut self, ui: &mut Ui) {
        panel_title(ui, "Эффекты");
        panel_hint(
            ui,
            "Двойной щелчок ставит эффект у курсора, перетаскивание — на видеодорожку. \
             Цвета, размер и разлёт настраиваются справа.",
        );
        self.effects_tiles(ui);
    }

    /// Плитки процедурных эффектов: взрыв, огонь, снег и остальные.
    pub(super) fn effects_tiles(&mut self, ui: &mut Ui) {
        /// Кадров в секунду у эффекта, который проигрывается на плитке.
        const RATE: f64 = 24.0;
        let mut add: Option<EffectKind> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for row in EffectKind::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, kind) in row.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("effect", *kind));
                    // Под указателем эффект проигрывается по кругу, иначе стоит на самом наглядном кадре.
                    let progress = if tile.resp.hovered() {
                        ui.ctx().request_repaint();
                        let frames = (kind.duration() * RATE).round();
                        ((ui.input(|i| i.time) * RATE) % frames).floor() as f32 / frames as f32
                    } else {
                        kind.poster()
                    };
                    let tex = self.effect_thumb(ui.ctx(), *kind, progress);
                    let frame = Rect::from_center_size(tile.rect.center(), Vec2::splat(tile.rect.height()));
                    egui::Image::new(&tex).paint_at(ui, frame);
                    tile_caption(ui, &tile, kind.label());
                    if tile.resp.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), EffectDrag(*kind));
                    }
                    if tile.resp.double_clicked() {
                        add = Some(*kind);
                    }
                }
            }
        });
        if let Some(kind) = add {
            self.add_effect(kind);
        }
    }

    /// Текстура с образцом надписи для плитки.
    fn text_thumb(&mut self, ctx: &egui::Context, preset: TextPreset) -> TextureHandle {
        if let Some(tex) = self.lib.text_thumbs.get(&preset) {
            return tex.clone();
        }
        let raster = text::render(&Text { content: "Текст".to_owned(), ..preset.text(TEXT_THUMB_FRAME) });
        let image = ColorImage::from_rgba_unmultiplied([raster.w as usize, raster.h as usize], &raster.rgba);
        let tex = ctx.load_texture(format!("text-thumb-{preset:?}"), image, TextureOptions::LINEAR);
        self.lib.text_thumbs.insert(preset, tex.clone());
        tex
    }

    fn text_panel(&mut self, ui: &mut Ui) {
        panel_title(ui, "Текст");
        panel_hint(
            ui,
            "Двойной щелчок ставит надпись у курсора, перетаскивание — на видеодорожку. \
             Сам текст, шрифт и цвета меняются справа.",
        );
        let mut add: Option<TextPreset> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for row in TextPreset::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, preset) in row.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("text", *preset));
                    let tex = self.text_thumb(ui.ctx(), *preset);
                    let have = tex.size_vec2();
                    let (w, h) = fit_size(have.x, have.y, tile.rect.width() - 12.0, tile.rect.height() - 10.0);
                    egui::Image::new(&tex).paint_at(ui, Rect::from_center_size(tile.rect.center(), vec2(w, h)));
                    tile_caption(ui, &tile, preset.label());
                    if tile.resp.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), TextDrag(*preset));
                    }
                    if tile.resp.double_clicked() {
                        add = Some(*preset);
                    }
                }
            }
        });
        if let Some(preset) = add {
            self.add_text(preset);
        }
    }

    /// Текстура с картинкой-образцом, как её меняет фильтр.
    fn filter_thumb(&mut self, ctx: &egui::Context, kind: FilterKind) -> TextureHandle {
        if let Some(tex) = self.lib.filter_thumbs.get(&kind) {
            return tex.clone();
        }
        let mut picture = sample(SAMPLE);
        Look { filter: kind, ..Look::default() }.apply(&mut picture);
        let image = ColorImage::from_rgba_unmultiplied(SAMPLE, &picture);
        let tex = ctx.load_texture(format!("filter-thumb-{kind:?}"), image, TextureOptions::LINEAR);
        self.lib.filter_thumbs.insert(kind, tex.clone());
        tex
    }

    fn filters_panel(&mut self, ui: &mut Ui) {
        panel_title(ui, "Фильтры");
        panel_hint(
            ui,
            "Двойной щелчок кладёт фильтр на выбранный клип, а без выбора — на клип под курсором. \
             Сила фильтра, яркость и контраст — справа, на вкладке «Цвет»; эффекты и маски — в VFX-редакторе.",
        );
        let current = self.selected.and_then(|id| self.project.clip(id)).filter(|c| !c.generated()).map(|c| c.look.filter);
        let mut apply: Option<FilterKind> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for row in FilterKind::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, kind) in row.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("filter", *kind));
                    let tex = self.filter_thumb(ui.ctx(), *kind);
                    egui::Image::new(&tex).rounding(6.0).paint_at(ui, tile.rect);
                    // Фильтр выбранного клипа обведён.
                    if current == Some(*kind) {
                        ui.painter().rect_stroke(tile.rect, 6.0, Stroke::new(2.0, ACCENT));
                    }
                    tile_caption(ui, &tile, kind.label());
                    if tile.resp.double_clicked() {
                        apply = Some(*kind);
                    }
                }
            }
        });
        if let Some(kind) = apply {
            self.apply_filter(kind);
        }
    }

    fn captions_panel(&mut self, ui: &mut Ui) {
        panel_title(ui, "Субтитры");
        panel_hint(
            ui,
            "Субтитры лежат на своей дорожке над видео. Текст и оформление выбранного субтитра меняются справа.",
        );
        ui.horizontal_wrapped(|ui| {
            if widgets::accent_button(ui, Some(icons::PLUS), "Добавить").clicked() {
                self.add_caption();
            }
            if widgets::soft_button(ui, Some(icons::FOLDER), "Импорт .srt").clicked() {
                self.import_srt_dialog();
            }
            if widgets::soft_button(ui, Some(icons::EXPORT), "Экспорт .srt").clicked() {
                self.export_srt_dialog();
            }
        });
        ui.add_space(10.0);

        let cues = self.caption_cues();
        let ids: Vec<Id> = self.project.captions().iter().map(|c| c.id).collect();
        if cues.is_empty() {
            panel_hint(ui, "Субтитров пока нет: добавьте первый у курсора воспроизведения или загрузите готовый файл .srt.");
            return;
        }
        let mut pick: Option<(Id, f64)> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (id, cue) in ids.iter().zip(&cues) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                let hover = fx::hover(ui.ctx(), &resp);
                ui.painter().rect_filled(rect, 6.0, RAISED.lerp_to_gamma(HOVER, hover));
                if self.selected == Some(*id) {
                    ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.5, ACCENT));
                }
                let time = format!("{} – {}", clock(cue.start), clock(cue.end));
                ui.painter().text(rect.left_top() + vec2(10.0, 6.0), Align2::LEFT_TOP, time, theme::font_mono(10.5), TEXT3);
                let line = cue.text.replace('\n', " ");
                let galley = widgets::elided(ui, &line, theme::font(12.5), TEXT, rect.width() - 20.0);
                ui.painter().galley(rect.left_top() + vec2(10.0, 21.0), galley, TEXT);
                if resp.clicked() {
                    pick = Some((*id, cue.start));
                }
            }
        });
        if let Some((id, start)) = pick {
            self.selected = Some(id);
            self.inspector_tab = InspectorTab::Text;
            self.seek(start);
        }
    }

    fn transitions_panel(&mut self, ui: &mut Ui) {
        /// Длительность круга на плитке: кадр A, сам переход и кадр B.
        const CYCLE: f64 = 1.8;
        panel_title(ui, "Переходы");
        panel_hint(
            ui,
            "Перетащите переход на стык двух клипов или дважды щёлкните — он встанет у выбранного клипа. \
             Стык есть там, где клип разрезан или клипы стоят вплотную.",
        );
        let mut apply: Option<TransitionKind> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for row in TransitionKind::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, kind) in row.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("transition", *kind));
                    // Под указателем переход проигрывается, иначе застыл на полпути.
                    let progress = if tile.resp.hovered() {
                        ui.ctx().request_repaint();
                        let phase = (ui.input(|i| i.time) % CYCLE / CYCLE) as f32;
                        ((phase - 0.2) / 0.5).clamp(0.0, 1.0)
                    } else if kind.overlaps() {
                        0.5
                    } else {
                        0.3
                    };
                    transition_demo(&ui.painter().with_clip_rect(tile.rect), tile.rect, *kind, progress);
                    tile_caption(ui, &tile, kind.label());
                    if tile.resp.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), TransitionDrag(*kind));
                    }
                    if tile.resp.double_clicked() {
                        apply = Some(*kind);
                    }
                }
            }
        });
        if let Some(kind) = apply {
            self.apply_transition_auto(kind);
        }
    }
}

fn placeholder_panel(ui: &mut Ui, cat: Category) {
    panel_title(ui, cat.label());
    let r = ui.available_rect_before_wrap();
    ui.painter().text(
        pos2(r.center().x, r.top() + 80.0),
        Align2::CENTER_CENTER,
        "Раздел появится в следующих версиях",
        theme::font(12.5),
        TEXT3,
    );
}
