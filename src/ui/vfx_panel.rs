//! Страница VFX: слева — слои кадра и библиотека эффектов, справа — оформление выбранного клипа.

use eframe::egui::{
    self, pos2, vec2, Align, ColorImage, Layout, Margin, Rect, RichText, ScrollArea, Sense, Stroke, TextureHandle,
    TextureOptions, Ui, UiBuilder,
};

use crate::app::{App, VfxShelf, VfxTool};
use crate::icons;
use crate::look::{ChromaKey, Look};
use crate::model::{short_time, Id, MediaKind, TrackKind};
use crate::theme::{self, *};
use crate::ui::inspector::{caption, divider, header, hint, percent_row, row, Edit, SLIDER_W};
use crate::ui::library::{panel_hint, panel_title, sample, tile, tile_caption, tile_row, tile_size, COLUMNS};
use crate::ui::{fx, widgets};
use crate::vfx::{Blend, Fx, FxKind, Mask, MaskShape, Mattes, Recipe, Vfx, VfxPreset, MAX_CROP, MIRRORS};

/// Размер картинки-образца на плитках эффектов.
const THUMB: [usize; 2] = [192, 108];
/// На сколько короче ползунок внутри карточки эффекта: у карточки свои поля.
const CARD_ROOM: f32 = 18.0;

/// Эффект с такой силой, чтобы его было видно на маленькой плитке.
fn showcase(kind: FxKind) -> Fx {
    let (amount, extra) = match kind {
        FxKind::Blur => (0.6, 0.0),
        FxKind::Glow => (1.0, 0.45),
        FxKind::Sharpen => (1.0, 0.0),
        FxKind::Vignette => (0.9, 0.35),
        FxKind::Grain => (0.9, 0.0),
        FxKind::RgbShift => (1.0, 0.0),
        FxKind::Pixelate => (0.5, 0.0),
        FxKind::Hue => (0.4, 0.0),
        FxKind::Invert => (1.0, 0.0),
        FxKind::Posterize => (0.85, 0.0),
        FxKind::Mirror => (1.0, 0.0),
        FxKind::Scanlines => (0.5, 0.0),
    };
    Fx { amount, extra, ..Fx::new(kind) }
}

/// Текстура с картинкой-образцом, как её меняет стопка эффектов.
fn thumb(ctx: &egui::Context, name: String, stack: Vec<Fx>) -> TextureHandle {
    let recipe = Recipe { vfx: Vfx { stack, ..Vfx::default() }, ..Recipe::default() };
    let picture = recipe.cook(&sample(THUMB), THUMB[0], THUMB[1], 0, &mut Mattes::default());
    ctx.load_texture(name, ColorImage::from_rgba_premultiplied(THUMB, &picture), TextureOptions::LINEAR)
}

/// Раздел со сворачиваемым заголовком; содержимое проявляется, когда раздел раскрывают.
fn fold_section(ui: &mut Ui, title: &str, open: &mut bool, lit: bool, add: impl FnOnce(&mut Ui)) {
    ui.push_id(title, |ui| {
        let head = widgets::fold(ui, title, *open, lit);
        if head.clicked() {
            *open = !*open;
        }
        let t = fx::enter(ui.ctx(), head.id.with("body"), *open);
        if *open {
            ui.scope(|ui| {
                ui.multiply_opacity(t);
                ui.add_space(2.0 + 8.0 * (1.0 - t));
                add(ui);
                ui.add_space(6.0);
            });
        }
    });
}

/// Подписи в ряд: щелчок по подписи выбирает её значение.
fn choice<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(&str, T)], edit: &mut Edit) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (label, option) in options {
            if widgets::chip(ui, label, *value == *option).clicked() && *value != *option {
                *value = *option;
                edit.now();
            }
        }
    });
}

fn key_section(ui: &mut Ui, look: &mut Look, tool: &mut VfxTool, matte: &mut bool, edit: &mut Edit) {
    let mut on = look.key.is_some();
    if row(ui, "Убрать фон", |ui| widgets::switch(ui, &mut on)).changed() {
        look.key = on.then(|| ChromaKey::studio([0, 255, 0]));
        edit.now();
    }
    let Some(key) = &mut look.key else {
        hint(ui, "Убирает зелёный или синий фон: сквозь клип становится видно то, что лежит на дорожках ниже.");
        return;
    };
    ui.horizontal(|ui| {
        caption(ui, "Цвет фона");
        let r = egui::color_picker::color_edit_button_srgb(ui, &mut key.color);
        edit.burst(ui, &r);
        let picking = *tool == VfxTool::Pipette;
        if widgets::icon_button(ui, icons::PIPETTE, 26.0, "Пипетка: взять цвет фона щелчком по кадру", picking, true).clicked() {
            *tool = if picking { VfxTool::Move } else { VfxTool::Pipette };
        }
    });
    ui.horizontal(|ui| {
        caption(ui, "Фон студии");
        ui.spacing_mut().item_spacing.x = 6.0;
        for (label, color) in [("Зелёный", [0, 255, 0]), ("Синий", [0, 0, 255])] {
            if widgets::chip(ui, label, key.color == color).clicked() && key.color != color {
                key.color = color;
                edit.now();
            }
        }
    });
    percent_row(ui, "Допуск", &mut key.similarity, 1.0..=100.0, edit).on_hover_text("Насколько цвет точки может отличаться от цвета фона");
    percent_row(ui, "Мягкость края", &mut key.blend, 0.0..=100.0, edit);
    percent_row(ui, "Отсвет фона", &mut key.spill, 0.0..=100.0, edit)
        .on_hover_text("Гасит зелёный или синий отсвет фона на самом предмете");
    percent_row(ui, "Сжатие края", &mut key.choke, 0.0..=100.0, edit).on_hover_text("Срезает цветную кайму по краю предмета");
    row(ui, "Показать вырез", |ui| widgets::switch(ui, matte)).on_hover_text("Белое остаётся в кадре, чёрное вырезано");
}

fn blend_section(ui: &mut Ui, blend: &mut Blend, edit: &mut Edit) {
    choice(ui, blend, &Blend::ALL.map(|b| (b.label(), b)), edit);
    ui.add_space(4.0);
    hint(ui, "«Экран» и «Сложение» убирают чёрный фон: так в кадр вставляют огонь, дым, искры и блики, снятые на чёрном.");
}

fn mask_section(ui: &mut Ui, vfx: &mut Vfx, tool: &mut VfxTool, edit: &mut Edit) {
    let mut shape = vfx.mask.as_ref().map(|m| m.shape);
    let before = shape;
    choice(ui, &mut shape, &[("Нет", None), ("Прямоугольник", Some(MaskShape::Rect)), ("Эллипс", Some(MaskShape::Ellipse))], &mut Edit::default());
    if shape != before {
        vfx.mask = shape.map(|shape| Mask { shape, ..vfx.mask.clone().unwrap_or_else(|| Mask::new(shape)) });
        edit.now();
    }
    if let Some(mask) = &mut vfx.mask {
        percent_row(ui, "Середина по X", &mut mask.x, 0.0..=100.0, edit);
        percent_row(ui, "Середина по Y", &mut mask.y, 0.0..=100.0, edit);
        percent_row(ui, "Ширина", &mut mask.w, 2.0..=200.0, edit);
        percent_row(ui, "Высота", &mut mask.h, 2.0..=200.0, edit);
        percent_row(ui, "Мягкость края", &mut mask.feather, 0.0..=100.0, edit);
        if row(ui, "Наоборот", |ui| widgets::switch(ui, &mut mask.invert)).on_hover_text("Видимой остаётся часть клипа вне фигуры").changed() {
            edit.now();
        }
        ui.add_space(2.0);
        let editing = *tool == VfxTool::Mask;
        let clicked = if editing {
            widgets::accent_button(ui, Some(icons::CHECK), "Готово").clicked()
        } else {
            widgets::soft_button(ui, Some(icons::SHAPE), "Править в кадре").clicked()
        };
        if clicked {
            *tool = if editing { VfxTool::Move } else { VfxTool::Mask };
        }
    } else if *tool == VfxTool::Mask {
        *tool = VfxTool::Move;
    }

    divider(ui);
    ui.add_space(4.0);
    ui.label(RichText::new("Кадрирование").font(theme::font_semibold(12.0)).color(TEXT2));
    for (label, value) in ["Слева", "Сверху", "Справа", "Снизу"].into_iter().zip(&mut vfx.crop) {
        percent_row(ui, label, value, 0.0..=MAX_CROP * 100.0, edit);
    }
}

fn stack_section(ui: &mut Ui, stack: &mut Vec<Fx>, edit: &mut Edit) {
    enum Act {
        Up(usize),
        Down(usize),
        Remove(usize),
    }
    ui.horizontal(|ui| {
        ui.menu_button("Добавить эффект", |ui| {
            for kind in FxKind::ALL {
                if ui.button(kind.label()).on_hover_text(kind.tip()).clicked() {
                    ui.close_menu();
                    stack.push(Fx::new(kind));
                    edit.now();
                }
            }
        });
        if !stack.is_empty() && widgets::soft_button(ui, Some(icons::TRASH), "Убрать все").clicked() {
            stack.clear();
            edit.now();
        }
    });
    if stack.is_empty() {
        ui.add_space(4.0);
        hint(ui, "Эффектов пока нет: добавьте их кнопкой выше или двойным щелчком по плитке слева.");
        return;
    }

    let mut act: Option<Act> = None;
    let count = stack.len();
    for (i, item) in stack.iter_mut().enumerate() {
        // Новая карточка проявляется и подъезжает на место.
        let t = fx::fresh(ui.ctx(), egui::Id::new(("fx-card", i, item.kind)));
        ui.push_id(("fx-card", i), |ui| {
            ui.multiply_opacity(t);
            ui.add_space(4.0 + 8.0 * (1.0 - t));
            egui::Frame::none().fill(RAISED).rounding(8.0).inner_margin(Margin::symmetric(8.0, 6.0)).show(ui, |ui| {
                ui.spacing_mut().slider_width = SLIDER_W - CARD_ROOM;
                ui.horizontal(|ui| {
                    if widgets::switch(ui, &mut item.on).changed() {
                        edit.now();
                    }
                    let color = if item.on { TEXT } else { TEXT3 };
                    ui.label(RichText::new(item.kind.label()).font(theme::font_semibold(12.5)).color(color)).on_hover_text(item.kind.tip());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if widgets::icon_button(ui, icons::CLOSE, 22.0, "Убрать эффект", false, true).clicked() {
                            act = Some(Act::Remove(i));
                        }
                        if widgets::icon_button(ui, icons::CHEVRON, 22.0, "Ниже: применить позже", false, i + 1 < count).clicked() {
                            act = Some(Act::Down(i));
                        }
                        if widgets::icon_button(ui, icons::UP, 22.0, "Выше: применить раньше", false, i > 0).clicked() {
                            act = Some(Act::Up(i));
                        }
                    });
                });
                let [first, second] = item.kind.knobs();
                if let Some(knob) = first {
                    percent_row(ui, knob.label, &mut item.amount, knob.min..=knob.max, edit);
                }
                if let Some(knob) = second {
                    percent_row(ui, knob.label, &mut item.extra, knob.min..=knob.max, edit);
                }
                if item.kind == FxKind::Mirror {
                    let mut side = (item.extra.round().max(0.0) as usize).min(MIRRORS.len() - 1);
                    let before = side;
                    ui.label(RichText::new("Какая половина кадра остаётся").font(theme::font(11.5)).color(TEXT3));
                    let options: Vec<(&str, usize)> = MIRRORS.iter().copied().zip(0..).collect();
                    choice(ui, &mut side, &options, &mut Edit::default());
                    if side != before {
                        item.extra = side as f32;
                        edit.now();
                    }
                }
            });
        });
    }
    match act {
        Some(Act::Up(i)) => stack.swap(i, i - 1),
        Some(Act::Down(i)) => stack.swap(i, i + 1),
        Some(Act::Remove(i)) => {
            stack.remove(i);
        }
        None => return,
    }
    edit.now();
}

impl App {
    // ---------- Слева: слои и библиотека эффектов ----------

    pub fn vfx_browser_ui(&mut self, ui: &mut Ui) {
        let inner = ui.max_rect().shrink2(vec2(14.0, 0.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
            panel_title(ui, "VFX-редактор");
            panel_hint(
                ui,
                "Выберите слой и добавляйте эффекты двойным щелчком по плитке. \
                 Хромакей, маска и наложение выбранного слоя настраиваются справа.",
            );
            self.layers_ui(ui);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (shelf, label) in [(VfxShelf::Effects, "Эффекты"), (VfxShelf::Looks, "Наборы"), (VfxShelf::Generators, "Генераторы")] {
                    if widgets::chip(ui, label, self.vfx_ui.shelf == shelf).clicked() {
                        self.vfx_ui.shelf = shelf;
                    }
                }
            });
            ui.add_space(8.0);
            let shelf = self.vfx_ui.shelf;
            fx::appear(ui, egui::Id::new("vfx-shelf"), shelf, vec2(0.0, 10.0), |ui| match shelf {
                VfxShelf::Effects => self.fx_shelf(ui),
                VfxShelf::Looks => self.preset_shelf(ui),
                VfxShelf::Generators => self.effects_tiles(ui),
            });
        });
    }

    /// Клипы под курсором воспроизведения сверху вниз: щелчок выбирает слой.
    fn layers_ui(&mut self, ui: &mut Ui) {
        struct Row {
            id: Id,
            icon: &'static str,
            name: String,
            notes: String,
        }
        let mut rows: Vec<Row> = Vec::new();
        for track in self.project.tracks.iter().filter(|t| t.kind == TrackKind::Video && t.enabled) {
            for clip in track.clips.iter().filter(|c| c.contains(self.playhead)) {
                let media = self.project.media(clip.media).filter(|_| !clip.generated());
                let (icon, name) = match (&clip.effect, &clip.text, media) {
                    (Some(effect), _, _) => (icons::SPARK, effect.kind.label().to_owned()),
                    (_, Some(text), _) => (icons::TEXT, text.title().to_owned()),
                    (_, _, Some(m)) if m.kind == MediaKind::Image => (icons::IMAGE, m.name.clone()),
                    (_, _, Some(m)) if m.kind == MediaKind::Video => (icons::FILM, m.name.clone()),
                    _ => continue,
                };
                // Что уже сделано со слоем.
                let mut notes: Vec<String> = Vec::new();
                if clip.look.key.is_some() {
                    notes.push("хромакей".to_owned());
                }
                if clip.vfx.shaped() {
                    notes.push("маска".to_owned());
                }
                if clip.vfx.blend != Blend::Normal {
                    notes.push(clip.vfx.blend.label().to_lowercase());
                }
                if clip.vfx.active() > 0 {
                    notes.push(format!("эффектов: {}", clip.vfx.active()));
                }
                rows.push(Row { id: clip.id, icon, name, notes: notes.join(" · ") });
            }
        }

        ui.label(RichText::new("Слои в кадре").font(theme::font_semibold(12.0)).color(TEXT2));
        ui.add_space(2.0);
        if rows.is_empty() {
            return hint(ui, "Под курсором времени нет клипов: поставьте курсор на клип или добавьте его на таймлайн.");
        }
        for item in rows {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
            let ctx = ui.ctx();
            let hover = fx::hover(ctx, &resp);
            let on = ctx.animate_bool_with_time(resp.id.with("on"), self.selected == Some(item.id), 0.14);
            let body = fx::zoom(rect, rect.center(), fx::press(ctx, &resp, 0.03));
            ui.painter().rect_filled(body, 6.0, RAISED.lerp_to_gamma(HOVER, hover));
            if on > 0.0 {
                ui.painter().rect_stroke(body.shrink(0.5), 6.0, Stroke::new(1.5, ACCENT.gamma_multiply(on)));
            }
            let icon = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), icon, item.icon, TEXT2.lerp_to_gamma(ACCENT, on));
            let notes = ui.painter().layout_no_wrap(item.notes, theme::font(10.5), TEXT3);
            let notes_w = notes.size().x;
            let name = widgets::elided(ui, &item.name, theme::font(12.0), TEXT, rect.width() - 52.0 - notes_w);
            ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - name.size().y / 2.0), name, TEXT);
            ui.painter().galley(pos2(rect.right() - 10.0 - notes_w, rect.center().y - notes.size().y / 2.0), notes, TEXT3);
            if resp.clicked() {
                self.selected = Some(item.id);
                self.sel_transition = None;
                fx::tap(ctx, rect, 6.0, ACCENT);
            }
        }
    }

    /// Плитки эффектов: двойной щелчок кладёт эффект в стопку клипа.
    fn fx_shelf(&mut self, ui: &mut Ui) {
        let used: Vec<FxKind> = self.vfx_clip().map(|c| c.vfx.stack.iter().map(|fx| fx.kind).collect()).unwrap_or_default();
        let mut add: Option<FxKind> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for kinds in FxKind::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, kind) in kinds.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("fx", *kind));
                    let tex = self
                        .vfx_ui
                        .fx_thumbs
                        .entry(*kind)
                        .or_insert_with(|| thumb(ui.ctx(), format!("fx-thumb-{kind:?}"), vec![showcase(*kind)]))
                        .clone();
                    egui::Image::new(&tex).rounding(6.0).paint_at(ui, tile.rect);
                    // Эффекты, которые уже лежат на выбранном клипе, обведены.
                    if used.contains(kind) {
                        ui.painter().rect_stroke(tile.rect, 6.0, Stroke::new(2.0, ACCENT));
                    }
                    tile_caption(ui, &tile, kind.label());
                    if tile.resp.double_clicked() {
                        add = Some(*kind);
                    }
                    tile.resp.on_hover_text(format!("{}\nДвойной щелчок — добавить к клипу", kind.tip()));
                }
            }
        });
        if let Some(kind) = add {
            self.add_fx(kind);
        }
    }

    /// Плитки готовых наборов: двойной щелчок заменяет стопку эффектов клипа набором.
    fn preset_shelf(&mut self, ui: &mut Ui) {
        let mut apply: Option<VfxPreset> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let size = tile_size(ui.available_width());
            for presets in VfxPreset::ALL.chunks(COLUMNS) {
                let row_rect = tile_row(ui, size);
                for (i, preset) in presets.iter().enumerate() {
                    let tile = tile(ui, row_rect, i, size, ("vfx-preset", *preset));
                    let tex = self
                        .vfx_ui
                        .preset_thumbs
                        .entry(*preset)
                        .or_insert_with(|| thumb(ui.ctx(), format!("vfx-preset-{preset:?}"), preset.stack()))
                        .clone();
                    egui::Image::new(&tex).rounding(6.0).paint_at(ui, tile.rect);
                    tile_caption(ui, &tile, preset.label());
                    if tile.resp.double_clicked() {
                        apply = Some(*preset);
                    }
                    let parts: Vec<&str> = preset.stack().iter().map(|fx| fx.kind.label()).collect();
                    tile.resp.on_hover_text(format!("{}\nДвойной щелчок — заменить эффекты клипа этим набором", parts.join(" + ")));
                }
            }
        });
        if let Some(preset) = apply {
            self.apply_vfx_preset(preset);
        }
    }

    // ---------- Справа: оформление выбранного клипа ----------

    /// Выбранный клип, если его видно в кадре: видео, картинка, эффект или надпись.
    fn vfx_clip(&self) -> Option<&crate::model::Clip> {
        let clip = self.project.clip(self.selected?)?;
        (clip.generated() || self.stylable(clip)).then_some(clip)
    }

    pub fn vfx_inspector_ui(&mut self, ui: &mut Ui) {
        let body = header(ui, "Оформление слоя");
        let Some(original) = self.vfx_clip().cloned() else {
            ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(14.0);
                hint(
                    ui,
                    "Выберите видео или картинку на таймлайне, в кадре или в списке слоёв слева: \
                     здесь появятся хромакей, маска, наложение и эффекты этого слоя.",
                );
            });
            return;
        };
        let stylable = self.stylable(&original);
        let (title, kind) = match (&original.effect, &original.text, self.project.media(original.media)) {
            (Some(effect), _, _) => (effect.kind.label().to_owned(), "эффект"),
            (_, Some(text), _) => (text.title().to_owned(), "текст"),
            (_, _, Some(m)) if m.kind == MediaKind::Image => (m.name.clone(), "картинка"),
            (_, _, Some(m)) => (m.name.clone(), "видео"),
            _ => (String::new(), ""),
        };
        let thumb = self.lib.thumbs.get(&original.media).filter(|_| stylable).cloned();
        let can_paste = self.vfx_ui.copied.is_some();

        let mut clip = original.clone();
        let mut edit = Edit::default();
        let (mut open, mut tool, mut matte) = (self.vfx_ui.open, self.vfx_ui.tool, self.vfx_ui.matte);
        let (mut copy, mut paste) = (false, false);

        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            // Другой слой — содержимое панели проявляется заново.
            fx::appear(ui, egui::Id::new("vfx-inspector"), original.id, vec2(0.0, 6.0), |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let (frame, _) = ui.allocate_exact_size(vec2(56.0, 32.0), Sense::hover());
                        ui.painter().rect_filled(frame, 4.0, RAISED);
                        match &thumb {
                            Some(tex) => {
                                egui::Image::new(tex).rounding(4.0).paint_at(ui, frame);
                            }
                            None => {
                                let icon = if original.text.is_some() { icons::TEXT } else { icons::SPARK };
                                icons::paint(ui.painter(), Rect::from_center_size(frame.center(), vec2(16.0, 16.0)), icon, TEXT2);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let name = widgets::elided(ui, &title, theme::font_semibold(13.0), TEXT, ui.available_width());
                            ui.label(name);
                            let info = format!("{kind} · {}", short_time(original.duration));
                            ui.label(RichText::new(info).font(theme::font_mono(11.0)).color(TEXT3));
                        });
                    });
                    divider(ui);
                    ui.add_space(4.0);

                    if stylable {
                        let lit = clip.look.key.is_some();
                        fold_section(ui, "Хромакей", &mut open[0], lit, |ui| key_section(ui, &mut clip.look, &mut tool, &mut matte, &mut edit));
                    }
                    let lit = clip.vfx.blend != Blend::Normal;
                    fold_section(ui, "Наложение", &mut open[1], lit, |ui| blend_section(ui, &mut clip.vfx.blend, &mut edit));
                    if !stylable {
                        ui.add_space(6.0);
                        hint(ui, "У эффектов и надписей настраивается только наложение. Хромакей, маска и стопка эффектов есть у видео и картинок.");
                        return;
                    }
                    let lit = clip.vfx.shaped();
                    fold_section(ui, "Маска и кадрирование", &mut open[2], lit, |ui| mask_section(ui, &mut clip.vfx, &mut tool, &mut edit));
                    let lit = clip.vfx.active() > 0;
                    fold_section(ui, "Эффекты", &mut open[3], lit, |ui| stack_section(ui, &mut clip.vfx.stack, &mut edit));

                    divider(ui);
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        copy = widgets::soft_button(ui, Some(icons::COPY), "Копировать").on_hover_text("Запомнить цвет и эффекты слоя (Ctrl+Alt+C)").clicked();
                        if can_paste {
                            paste = widgets::soft_button(ui, Some(icons::PASTE), "Вставить").on_hover_text("Перенести сюда запомненное оформление (Ctrl+Alt+V)").clicked();
                        }
                        let dressed = clip.look.key.is_some() || !clip.vfx.is_empty();
                        if dressed && widgets::soft_button(ui, Some(icons::RESET), "Сбросить").on_hover_text("Убрать хромакей, маску, наложение и эффекты").clicked() {
                            clip.look.key = None;
                            clip.vfx = Vfx::default();
                            edit.now();
                        }
                    });
                    ui.add_space(12.0);
                });
            });
        });

        if edit.begin {
            self.commit();
        }
        if edit.changed {
            if let Some(c) = self.project.clip_mut(original.id) {
                (c.look, c.vfx) = (clip.look, clip.vfx);
            }
            self.touch();
        }
        (self.vfx_ui.open, self.vfx_ui.tool, self.vfx_ui.matte) = (open, tool, matte);
        if copy {
            self.copy_style();
        }
        if paste {
            self.paste_style();
        }
    }
}
