//! Правая панель: свойства выбранного клипа, перехода или проекта.

use eframe::egui::{self, pos2, vec2, Align2, Rect, Response, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder};

use crate::app::{App, InspectorTab, Page};
use crate::effects::{Effect, EffectKind, MAX_RES, MIN_RES};
use crate::icons;
use crate::keys::Prop;
use crate::look::{ChromaKey, FilterKind, Look};
use crate::model::{fit_size, short_time, Clip, Id, MediaKind, MAX_SPEED, MIN_SPEED};
use crate::motion::{Motion, MAX_LOOPS};
use crate::text::{self, Text, TextAlign};
use crate::theme::{self, *};
use crate::transition::{TransitionKind, MAX_DURATION, MIN_DURATION};
use crate::ui::fx;
use crate::ui::widgets::{self, Pin};

const LABEL_W: f32 = 112.0;
/// Ширина ползунка, при которой строка с подписью и полем значения умещается в панели.
pub(super) const SLIDER_W: f32 = 100.0;
/// На сколько короче ползунок в строке с кнопкой закрепления: место под кнопку берётся у него.
const KEY_ROOM: f32 = 16.0;
/// За сколько секунд клип проявляется и гаснет по кнопкам «В начале» и «В конце».
const FADE: f64 = 0.5;
const KEY_HINT: &str = "Ромб справа от значения закрепляет его в моменте, где стоит курсор времени. \
    Закрепите значение в двух моментах — между ними оно будет меняться плавно.";

pub(super) fn caption(ui: &mut Ui, label: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 26.0), Sense::hover());
    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, theme::font(12.0), TEXT2);
}

/// Строка «подпись — элемент управления».
pub(super) fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    ui.horizontal(|ui| {
        caption(ui, label);
        add(ui)
    })
    .inner
}

/// Кнопка закрепления свойства: по этому имени её находят и проверки интерфейса.
pub fn pin_id(prop: Prop) -> egui::Id {
    egui::Id::new(("pin", prop))
}

/// Момент правки: где стоит курсор времени и с какой точностью точка считается стоящей под ним.
#[derive(Clone, Copy)]
struct Moment {
    t: f64,
    reach: f64,
}

/// Ползунок в процентах. Хранимое значение обычно дробное: ползунок показывает его округлённым,
/// но сам не правит — иначе один только показ считался бы правкой (и ставил бы закреплённую точку).
fn key_slider(v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Slider<'_> {
    egui::Slider::new(v, range).clamping(egui::SliderClamping::Edits).suffix("%").fixed_decimals(0)
}

/// Строка с ползунком в процентах для значения, которое хранится долей.
pub(super) fn percent_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, edit: &mut Edit) -> Response {
    let mut shown = *value * 100.0;
    let r = row(ui, label, |ui| ui.add(key_slider(&mut shown, range)));
    edit.track(&r);
    if r.changed() {
        *value = shown / 100.0;
    }
    r
}

/// Строка с выбором цвета.
fn color_row(ui: &mut Ui, label: &str, color: &mut [u8; 3], edit: &mut Edit) {
    let r = row(ui, label, |ui| egui::color_picker::color_edit_button_srgb(ui, color));
    edit.burst(ui, &r);
}

/// Настройки надписи; `true`, если её оформление нужно перенести на все субтитры.
fn text_tab(ui: &mut Ui, t: &mut Text, edit: &mut Edit) -> bool {
    section(ui, if t.caption { "Субтитр" } else { "Текст" });
    let field = egui::TextEdit::multiline(&mut t.content).desired_rows(3).desired_width(f32::INFINITY).hint_text("Введите текст");
    let r = ui.add(field);
    edit.burst(ui, &r);
    ui.add_space(4.0);

    row(ui, "Шрифт", |ui| {
        egui::ComboBox::from_id_salt("text-font")
            .width(ui.available_width() - 8.0)
            .selected_text(t.font.as_str())
            .show_ui(ui, |ui| {
                for (name, _) in text::FONTS {
                    if ui.selectable_label(t.font == name, name).clicked() && t.font != name {
                        t.font = name.to_owned();
                        edit.now();
                    }
                }
            })
            .response
    });
    let r = row(ui, "Размер", |ui| {
        ui.add(egui::DragValue::new(&mut t.size).speed(1.0).range(text::MIN_SIZE..=text::MAX_SIZE).fixed_decimals(0).suffix(" px"))
    });
    edit.track(&r);
    color_row(ui, "Цвет", &mut t.color, edit);
    row(ui, "Выравнивание", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (label, align) in [("Слева", TextAlign::Left), ("Центр", TextAlign::Center), ("Справа", TextAlign::Right)] {
                if widgets::chip(ui, label, t.align == align).clicked() && t.align != align {
                    t.align = align;
                    edit.now();
                }
            }
        })
        .response
    });

    divider(ui);
    section(ui, "Контур");
    percent_row(ui, "Толщина", &mut t.outline, 0.0..=text::MAX_OUTLINE * 100.0, edit);
    color_row(ui, "Цвет контура", &mut t.outline_color, edit);

    divider(ui);
    section(ui, "Тень и плашка");
    if row(ui, "Тень", |ui| widgets::switch(ui, &mut t.shadow)).changed() {
        edit.now();
    }
    percent_row(ui, "Плашка", &mut t.backdrop, 0.0..=100.0, edit);
    color_row(ui, "Цвет плашки", &mut t.backdrop_color, edit);

    let mut share = false;
    ui.add_space(8.0);
    if t.caption {
        share = widgets::soft_button(ui, Some(icons::COPY), "Оформить так все субтитры").clicked();
        ui.add_space(6.0);
        hint(ui, "Остальные субтитры получат этот шрифт, цвета и место в кадре; их текст не изменится.");
    } else {
        hint(ui, "Надпись двигается и масштабируется прямо в кадре; её размер, поворот и непрозрачность — на вкладке «Видео».");
    }
    share
}

/// Фильтр, цветокоррекция и хромакей клипа; `true`, если нажата кнопка перехода в VFX-редактор.
fn color_tab(ui: &mut Ui, look: &mut Look, edit: &mut Edit) -> bool {
    ui.horizontal(|ui| {
        section(ui, "Фильтр");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, icons::RESET, 24.0, "Вернуть исходный цвет", false, true).clicked() {
                *look = Look::default();
                edit.now();
            }
        });
    });
    row(ui, "Фильтр", |ui| {
        egui::ComboBox::from_id_salt("look-filter")
            .width(ui.available_width() - 8.0)
            .selected_text(look.filter.label())
            .show_ui(ui, |ui| {
                for kind in FilterKind::ALL {
                    if ui.selectable_label(look.filter == kind, kind.label()).clicked() && look.filter != kind {
                        look.filter = kind;
                        edit.now();
                    }
                }
            })
            .response
    });
    if look.filter != FilterKind::None {
        percent_row(ui, "Сила", &mut look.strength, 0.0..=100.0, edit);
    }

    divider(ui);
    section(ui, "Коррекция");
    for (label, value) in [
        ("Яркость", &mut look.brightness),
        ("Контраст", &mut look.contrast),
        ("Насыщенность", &mut look.saturation),
        ("Теплота", &mut look.warmth),
    ] {
        percent_row(ui, label, value, -100.0..=100.0, edit);
    }

    divider(ui);
    section(ui, "Хромакей");
    let mut on = look.key.is_some();
    if row(ui, "Убрать фон", |ui| widgets::switch(ui, &mut on)).changed() {
        look.key = on.then(|| ChromaKey::studio([0, 255, 0]));
        edit.now();
    }
    if let Some(key) = &mut look.key {
        color_row(ui, "Цвет фона", &mut key.color, edit);
        percent_row(ui, "Допуск", &mut key.similarity, 1.0..=100.0, edit);
        percent_row(ui, "Мягкость края", &mut key.blend, 0.0..=100.0, edit);
    }
    ui.add_space(8.0);
    hint(ui, "Точки цвета фона становятся прозрачными: сквозь клип видно то, что лежит на дорожках ниже.");
    ui.add_space(8.0);
    let open = widgets::soft_button(ui, Some(icons::WAND), "Открыть VFX-редактор").clicked();
    ui.add_space(6.0);
    hint(ui, "Пипетка, подавление отсвета, маска, наложение и эффекты — в VFX-редакторе (Ctrl+2).");
    open
}

/// Кнопки плавного появления и исчезновения свойства на краях клипа.
fn fade_row(ui: &mut Ui, clip: &mut Clip, prop: Prop, tips: [&str; 2], reach: f64, edit: &mut Edit) {
    row(ui, "Плавно", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (label, tip, at_start) in [("В начале", tips[0], true), ("В конце", tips[1], false)] {
                if widgets::chip(ui, label, false).on_hover_text(tip).clicked() {
                    clip.fade(prop, at_start, FADE, reach);
                    edit.now();
                }
            }
        })
        .response
    });
}

/// Строка свойства, значение которого можно закрепить: у правого края стоит кнопка-ромб.
/// `unit` — во сколько раз показанное значение больше хранимого (проценты).
#[allow(clippy::too_many_arguments)]
fn key_row(
    ui: &mut Ui,
    label: &str,
    clip: &mut Clip,
    prop: Prop,
    unit: f32,
    at: Moment,
    edit: &mut Edit,
    add: impl FnOnce(&mut Ui, &mut f32) -> Response,
) -> Response {
    ui.horizontal(|ui| {
        caption(ui, label);
        ui.spacing_mut().slider_width = SLIDER_W - KEY_ROOM;
        let mut shown = clip.value(prop, at.t) * unit;
        let r = add(ui, &mut shown);
        edit.track(&r);
        if r.changed() {
            clip.set_value(prop, at.t, shown / unit, at.reach);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let pinned = !clip.keys.of(prop).is_empty();
            let (pin, tip) = if clip.key_at(prop, at.t, at.reach) {
                (Pin::Here, "Убрать закреплённую точку\nПравый щелчок — плавный ход, убрать все точки")
            } else if pinned {
                (Pin::Elsewhere, "Закрепить значение в этом моменте\nПравый щелчок — плавный ход, убрать все точки")
            } else {
                (Pin::Free, "Закрепить значение в этом моменте")
            };
            let button = widgets::key_button(ui, pin_id(prop), pin).on_hover_text(tip);
            if button.clicked() {
                clip.toggle_key(prop, at.t, at.reach);
                edit.now();
            }
            if pinned {
                button.context_menu(|ui| {
                    let mut smooth = clip.keys.is_eased(prop);
                    if ui.checkbox(&mut smooth, "Плавный разгон и торможение").changed() {
                        ui.close_menu();
                        clip.keys.set_eased(prop, smooth);
                        edit.now();
                    }
                    if ui.button("Убрать все точки").clicked() {
                        ui.close_menu();
                        clip.clear_keys(prop, at.t);
                        edit.now();
                    }
                });
            }
        });
        r
    })
    .inner
}

pub(super) fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).font(theme::font_semibold(12.5)).color(TEXT));
    ui.add_space(2.0);
}

pub(super) fn divider(ui: &mut Ui) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, LINE);
}

pub(super) fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::font(11.5)).color(TEXT3));
}

/// Заголовок панели с линией под ним; возвращает область под заголовком.
pub(super) fn header(ui: &mut Ui, title: &str) -> Rect {
    let rect = ui.max_rect();
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
    ui.painter().text(
        pos2(header.left() + 14.0, header.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(13.0),
        TEXT,
    );
    ui.painter().line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, LINE));
    Rect::from_min_max(pos2(rect.left() + 12.0, header.bottom() + 1.0), pos2(rect.right() - 12.0, rect.bottom()))
}

/// Отслеживает начало правки (для отмены) и сам факт изменения.
#[derive(Default)]
pub(super) struct Edit {
    pub(super) begin: bool,
    pub(super) changed: bool,
}

impl Edit {
    pub(super) fn track(&mut self, r: &Response) {
        if r.drag_started() || (r.changed() && !r.dragged()) {
            self.begin = true;
        }
        if r.changed() {
            self.changed = true;
        }
    }

    /// Разовая правка: нажатие кнопки или выбор из списка.
    pub(super) fn now(&mut self) {
        self.begin = true;
        self.changed = true;
    }

    /// Для палитры, где у правки нет явного начала: изменения, идущие подряд, — одна правка.
    pub(super) fn burst(&mut self, ui: &Ui, r: &Response) {
        if !r.changed() {
            return;
        }
        let (id, now) = (r.id.with("burst"), ui.input(|i| i.time));
        let last = ui.data(|d| d.get_temp::<f64>(id));
        if last.map_or(true, |t| now - t > 0.6) {
            self.begin = true;
        }
        ui.data_mut(|d| d.insert_temp(id, now));
        self.changed = true;
    }
}

/// Настройки процедурного эффекта.
fn effect_tab(ui: &mut Ui, e: &mut Effect, edit: &mut Edit) {
    ui.horizontal(|ui| {
        section(ui, e.kind.label());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, icons::RESET, 24.0, "Вернуть настройки по умолчанию", false, true).clicked() {
                *e = Effect { span: e.span, ..Effect::new(e.kind, e.seed) };
                edit.now();
            }
        });
    });
    let rings = e.kind == EffectKind::Shockwave;
    let r = row(ui, e.kind.count_label(), |ui| ui.add(egui::Slider::new(&mut e.count, 1..=e.kind.max_count())));
    edit.track(&r);

    let mut spread = e.spread * 100.0;
    let r = row(ui, "Разлёт", |ui| {
        ui.add(egui::Slider::new(&mut spread, 5.0..=200.0).suffix("%").fixed_decimals(0))
    });
    edit.track(&r);
    e.spread = spread / 100.0;

    let mut size = e.size * 100.0;
    let size_label = match e.kind {
        EffectKind::Shockwave => "Толщина кольца",
        EffectKind::Lightning => "Толщина",
        EffectKind::Rain => "Длина струй",
        _ => "Размер частиц",
    };
    let r = row(ui, size_label, |ui| {
        ui.add(egui::Slider::new(&mut size, 0.5..=50.0).logarithmic(true).suffix("%").fixed_decimals(1))
    });
    edit.track(&r);
    e.size = size / 100.0;

    // У колец и молнии падать нечему.
    if !rings && e.kind != EffectKind::Lightning {
        let mut gravity = e.gravity * 100.0;
        let r = row(ui, "Тяга вниз", |ui| {
            ui.add(egui::Slider::new(&mut gravity, -200.0..=200.0).fixed_decimals(0))
        })
        .on_hover_text("Отрицательная тяга поднимает частицы вверх");
        edit.track(&r);
        e.gravity = gravity / 100.0;
    }

    row(ui, "Детализация", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for res in [32, 64, 128, 256] {
                let selected = e.res.clamp(MIN_RES, MAX_RES) == res;
                if widgets::chip(ui, &res.to_string(), selected).clicked() && !selected {
                    e.res = res;
                    edit.now();
                }
            }
        })
        .response
    })
    .on_hover_text("Сторона кадра эффекта в точках: чем меньше, тем крупнее «пиксели»");

    divider(ui);
    section(ui, "Цвет");
    for (i, label) in ["В начале", "В конце"].into_iter().enumerate() {
        let r = row(ui, label, |ui| egui::color_picker::color_edit_button_srgb(ui, &mut e.colors[i]));
        edit.burst(ui, &r);
    }

    divider(ui);
    ui.add_space(8.0);
    if widgets::soft_button(ui, Some(icons::DICE), "Другой вариант").clicked() {
        // Новое зерно: те же настройки, другая случайная раскладка частиц.
        e.seed = e.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        edit.now();
    }
    ui.add_space(8.0);
    hint(ui, "Эффект длится столько же, сколько клип: потяните край клипа на таймлайне, чтобы замедлить или ускорить его.");
}

impl App {
    pub fn inspector_ui(&mut self, ui: &mut Ui) {
        let transition = self.sel_transition.filter(|id| self.project.clip(*id).map_or(false, |c| c.transition.is_some()));
        let clip = self.selected.and_then(|id| self.project.clip(id).cloned());
        // Что показывает панель: переход, клип или проект. При смене содержимое проявляется заново.
        let showing = match (transition, &clip) {
            (Some(_), _) => 0,
            (None, Some(_)) => 1,
            (None, None) => 2,
        };
        fx::appear(ui, egui::Id::new("inspector"), showing, vec2(0.0, 6.0), |ui| match (transition, clip) {
            (Some(id), _) => self.transition_inspector(ui, id),
            (None, Some(clip)) => self.clip_inspector(ui, clip),
            (None, None) => self.project_inspector(ui),
        });
    }

    fn clip_inspector(&mut self, ui: &mut Ui, original: Clip) {
        let rect = ui.max_rect();
        let media = self.project.media(original.media).cloned();
        if media.is_none() && !original.generated() {
            return;
        }
        let kind = media.as_ref().map(|m| m.kind);

        // Вкладки зависят от типа клипа.
        let tabs: &[(InspectorTab, &str)] = match kind {
            None if original.text.is_some() => &[
                (InspectorTab::Text, "Текст"),
                (InspectorTab::Video, "Видео"),
                (InspectorTab::Motion, "Анимация"),
            ],
            None => &[
                (InspectorTab::Effect, "Эффект"),
                (InspectorTab::Video, "Видео"),
                (InspectorTab::Motion, "Анимация"),
            ],
            Some(MediaKind::Video) => &[
                (InspectorTab::Video, "Видео"),
                (InspectorTab::Color, "Цвет"),
                (InspectorTab::Audio, "Аудио"),
                (InspectorTab::Speed, "Скорость"),
                (InspectorTab::Motion, "Анимация"),
            ],
            Some(MediaKind::Image) => &[
                (InspectorTab::Video, "Видео"),
                (InspectorTab::Color, "Цвет"),
                (InspectorTab::Motion, "Анимация"),
            ],
            Some(MediaKind::Audio) => &[(InspectorTab::Audio, "Аудио"), (InspectorTab::Speed, "Скорость")],
        };
        if !tabs.iter().any(|(t, _)| *t == self.inspector_tab) {
            self.inspector_tab = tabs[0].0;
        }
        let tabs_rect = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(tabs_rect.shrink2(vec2(6.0, 0.0))), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let even = (ui.available_width() / tabs.len().max(3) as f32).floor();
                // Когда вкладок много, место делится по длине подписей: поровну длинные не помещаются.
                let widths: Vec<f32> = tabs
                    .iter()
                    .map(|(_, label)| ui.painter().layout_no_wrap((*label).to_owned(), theme::font_semibold(12.5), TEXT).size().x)
                    .collect();
                let spare = (ui.available_width() - widths.iter().sum::<f32>()) / tabs.len() as f32;
                for ((tab, label), text_w) in tabs.iter().zip(widths) {
                    let w = if tabs.len() > 3 { (text_w + spare).floor() } else { even };
                    if widgets::tab(ui, label, self.inspector_tab == *tab, w, 40.0).clicked() {
                        self.inspector_tab = *tab;
                    }
                }
            });
        });
        ui.painter().line_segment([tabs_rect.left_bottom(), tabs_rect.right_bottom()], Stroke::new(1.0, LINE));

        let body = Rect::from_min_max(pos2(rect.left() + 12.0, tabs_rect.bottom() + 1.0), pos2(rect.right() - 12.0, rect.bottom()));
        let mut clip = original.clone();
        let mut edit = Edit::default();
        let mut speed = clip.speed;
        let at = Moment { t: self.playhead, reach: self.project.key_reach() };
        let mut restyle = false;
        let mut open_vfx = false;
        // Во сколько раз нужно увеличить вписанный клип, чтобы он закрыл кадр целиком.
        let fill = media.as_ref().filter(|m| m.kind != MediaKind::Audio && m.width > 0 && m.height > 0).map(|m| {
            let (pw, ph) = (self.project.width as f32, self.project.height as f32);
            let (bw, bh) = fit_size(m.width as f32, m.height as f32, pw, ph);
            (pw / bw).max(ph / bh)
        });
        let tab = self.inspector_tab;
        let mut path_edit = self.path_edit == Some(clip.id);
        // Размер готового пути по умолчанию: пятая часть меньшей стороны кадра.
        let preset_radius = self.project.width.min(self.project.height) as f32 * 0.2;

        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            // Содержимое вкладки проявляется, когда вкладку сменили.
            fx::appear(ui, egui::Id::new("inspector-tab"), tab, vec2(0.0, 6.0), |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().slider_width = SLIDER_W;
                ui.add_space(12.0);
                // Шапка: что выбрано.
                ui.horizontal(|ui| {
                    let (thumb, _) = ui.allocate_exact_size(vec2(56.0, 32.0), Sense::hover());
                    ui.painter().rect_filled(thumb, 4.0, RAISED);
                    match self.lib.thumbs.get(&original.media) {
                        Some(tex) => {
                            egui::Image::new(tex).rounding(4.0).paint_at(ui, thumb);
                        }
                        None => {
                            let icon = match kind {
                                None if clip.text.is_some() => icons::TEXT,
                                None => icons::SPARK,
                                Some(MediaKind::Audio) => icons::NOTE,
                                Some(_) => icons::FILM,
                            };
                            icons::paint(ui.painter(), Rect::from_center_size(thumb.center(), vec2(16.0, 16.0)), icon, TEXT2);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let w = ui.available_width();
                        let title = match (&media, &clip.effect, &clip.text) {
                            (Some(m), _, _) => m.name.as_str(),
                            (None, Some(e), _) => e.kind.label(),
                            (None, None, Some(t)) => t.title(),
                            (None, None, None) => "",
                        };
                        let name = widgets::elided(ui, title, theme::font_semibold(13.0), TEXT, w);
                        ui.label(name);
                        let info = match &media {
                            None if clip.text.as_ref().map_or(false, |t| t.caption) => format!("субтитр · {}", short_time(clip.duration)),
                            None if clip.text.is_some() => format!("текст · {}", short_time(clip.duration)),
                            None => format!("эффект · {}", short_time(clip.duration)),
                            Some(m) if m.kind == MediaKind::Audio => format!("аудио · {}", short_time(clip.duration)),
                            Some(m) if m.kind == MediaKind::Image => {
                                format!("{}×{} · {}", m.width, m.height, short_time(clip.duration))
                            }
                            Some(m) => {
                                format!("{}×{} · {:.0} к/с · {}", m.width, m.height, m.fps, short_time(clip.duration))
                            }
                        };
                        ui.label(RichText::new(info).font(theme::font_mono(11.0)).color(TEXT3));
                    });
                });
                divider(ui);

                match self.inspector_tab {
                    InspectorTab::Effect => {
                        if let Some(e) = &mut clip.effect {
                            effect_tab(ui, e, &mut edit);
                        }
                    }
                    InspectorTab::Text => {
                        if let Some(t) = &mut clip.text {
                            restyle = text_tab(ui, t, &mut edit);
                        }
                    }
                    InspectorTab::Color => open_vfx = color_tab(ui, &mut clip.look, &mut edit),
                    InspectorTab::Video => {
                        ui.horizontal(|ui| {
                            section(ui, "Трансформация");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let r = widgets::icon_button(ui, icons::RESET, 24.0, "Сбросить", false, true);
                                if r.clicked() {
                                    clip.reset_transform();
                                    edit.now();
                                }
                            });
                        });
                        key_row(ui, "Масштаб", &mut clip, Prop::Scale, 100.0, at, &mut edit, |ui, v| {
                            ui.add(key_slider(v, 5.0..=400.0))
                        });
                        // Клип с другими пропорциями либо вписан в кадр с полями, либо закрывает его целиком.
                        if let Some(fill) = fill.filter(|f| *f > 1.001) {
                            let now = clip.value(Prop::Scale, at.t);
                            row(ui, "В кадре", |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    for (label, tip, scale) in [
                                        ("Вписать", "Клип виден целиком, по краям остаются поля", 1.0),
                                        ("Заполнить", "Клип закрывает весь кадр, лишнее уходит за край", fill),
                                    ] {
                                        let selected = (now - scale).abs() < 0.005;
                                        if widgets::chip(ui, label, selected).on_hover_text(tip).clicked() && !selected {
                                            clip.set_value(Prop::Scale, at.t, scale, at.reach);
                                            clip.set_value(Prop::PosX, at.t, 0.0, at.reach);
                                            clip.set_value(Prop::PosY, at.t, 0.0, at.reach);
                                            edit.now();
                                        }
                                    }
                                })
                                .response
                            });
                        }
                        for (label, prop) in [("Положение X", Prop::PosX), ("Положение Y", Prop::PosY)] {
                            key_row(ui, label, &mut clip, prop, 1.0, at, &mut edit, |ui, v| {
                                ui.add(egui::DragValue::new(v).speed(1.0).fixed_decimals(0).suffix(" px"))
                            });
                        }
                        key_row(ui, "Поворот", &mut clip, Prop::Rotation, 1.0, at, &mut edit, |ui, v| {
                            let range = -360.0..=360.0;
                            ui.add(
                                egui::DragValue::new(v)
                                    .speed(0.5)
                                    .range(range)
                                    .clamp_existing_to_range(false)
                                    .fixed_decimals(1)
                                    .suffix("°"),
                            )
                        });

                        row(ui, "Отразить", |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                for (label, tip, flag) in [
                                    ("Гориз.", "Отразить слева направо", &mut clip.flip_h),
                                    ("Верт.", "Отразить сверху вниз", &mut clip.flip_v),
                                ] {
                                    if widgets::chip(ui, label, *flag).on_hover_text(tip).clicked() {
                                        *flag = !*flag;
                                        edit.now();
                                    }
                                }
                            })
                            .response
                        });

                        divider(ui);
                        section(ui, "Смешивание");
                        key_row(ui, "Непрозрачность", &mut clip, Prop::Opacity, 100.0, at, &mut edit, |ui, v| {
                            ui.add(key_slider(v, 0.0..=100.0))
                        });
                        let tips = ["Клип проявляется за полсекунды в начале", "Клип гаснет за полсекунды в конце"];
                        fade_row(ui, &mut clip, Prop::Opacity, tips, at.reach, &mut edit);
                        ui.add_space(8.0);
                        hint(ui, KEY_HINT);
                    }
                    InspectorTab::Audio => {
                        section(ui, "Звук");
                        if media.as_ref().map_or(true, |m| m.kind == MediaKind::Video && !m.has_audio) {
                            ui.label(RichText::new("В этом клипе нет звуковой дорожки").color(TEXT3));
                        } else {
                            key_row(ui, "Громкость", &mut clip, Prop::Volume, 100.0, at, &mut edit, |ui, v| {
                                ui.add(key_slider(v, 0.0..=200.0))
                            });
                            let tips = ["Звук нарастает за полсекунды в начале", "Звук затихает за полсекунды в конце"];
                            fade_row(ui, &mut clip, Prop::Volume, tips, at.reach, &mut edit);
                            ui.add_space(8.0);
                            hint(ui, KEY_HINT);
                        }
                    }
                    InspectorTab::Speed => {
                        section(ui, "Скорость");
                        let r = row(ui, "Скорость", |ui| {
                            ui.add(
                                egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED)
                                    .logarithmic(true)
                                    .suffix("×")
                                    .fixed_decimals(2),
                            )
                        });
                        edit.track(&r);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(LABEL_W + 8.0);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for preset in [0.5, 1.0, 1.5, 2.0] {
                                let selected = (speed - preset).abs() < 0.005;
                                if widgets::chip(ui, &format!("{preset}×"), selected).clicked() && !selected {
                                    speed = preset;
                                    edit.now();
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "Исходный отрезок {} → на таймлайне {}",
                                short_time(original.source_duration()),
                                short_time(clip.duration)
                            ))
                            .color(TEXT3),
                        );
                    }
                    InspectorTab::Motion => {
                        section(ui, "Движение по пути");
                        ui.horizontal(|ui| {
                            if path_edit {
                                if widgets::accent_button(ui, Some(icons::CHECK), "Готово").clicked() {
                                    path_edit = false;
                                }
                            } else if widgets::soft_button(ui, Some(icons::PATH), "Рисовать путь").clicked() {
                                path_edit = true;
                            }
                            if clip.motion.is_some() && widgets::soft_button(ui, Some(icons::TRASH), "Убрать").clicked() {
                                clip.motion = None;
                                path_edit = false;
                                edit.now();
                            }
                        });
                        ui.add_space(4.0);
                        hint(
                            ui,
                            if path_edit {
                                "Щёлкайте по кадру, чтобы ставить точки, или ведите мышью с нажатой кнопкой, чтобы нарисовать путь от руки. \
                                 Точки можно двигать, правый щелчок удаляет точку."
                            } else {
                                "Клип пройдёт по нарисованному пути за свою длительность."
                            },
                        );

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new("Готовые пути").font(theme::font(12.0)).color(TEXT2));
                            ui.add_space(6.0);
                            if widgets::chip(ui, "Круг", false).clicked() {
                                clip.motion = Some(Motion::circle(preset_radius));
                                edit.now();
                            }
                            if widgets::chip(ui, "Восьмёрка", false).clicked() {
                                clip.motion = Some(Motion::figure_eight(preset_radius * 1.5));
                                edit.now();
                            }
                        });

                        if let Some(m) = &mut clip.motion {
                            divider(ui);
                            section(ui, "Настройки пути");
                            if m.is_active() {
                                let mut radius = m.radius();
                                let r = row(ui, "Размах", |ui| {
                                    ui.add(egui::DragValue::new(&mut radius).speed(2.0).range(1.0..=20000.0).fixed_decimals(0).suffix(" px"))
                                })
                                .on_hover_text("Растягивает путь целиком от его середины");
                                edit.track(&r);
                                if r.changed() {
                                    m.resize(radius);
                                }
                            }
                            let r = row(ui, "Проходов", |ui| ui.add(egui::DragValue::new(&mut m.loops).speed(0.05).range(1..=MAX_LOOPS)))
                                .on_hover_text("Сколько раз клип пройдёт путь; незамкнутый путь проходится туда и обратно");
                            edit.track(&r);
                            for (label, flag) in [
                                ("Замкнуть путь", &mut m.closed),
                                ("Сглаживать", &mut m.smooth),
                                ("Плавный разгон", &mut m.ease),
                            ] {
                                if row(ui, label, |ui| widgets::switch(ui, flag)).changed() {
                                    edit.now();
                                }
                            }
                            ui.add_space(4.0);
                            hint(ui, &format!("Точек в пути: {}", m.points.len()));
                        }
                    }
                }
                ui.add_space(12.0);
            });
            });
        });

        if open_vfx {
            self.set_page(Page::Vfx);
        }
        if edit.begin {
            self.commit();
        }
        if edit.changed {
            let id = original.id;
            // Точку правят в одном моменте: воспроизведение при этом останавливается.
            if self.playing && clip.keys != original.keys {
                self.pause();
            }
            if let Some(c) = self.project.clip_mut(id) {
                *c = clip;
            }
            if (speed - original.speed).abs() > 1e-6 {
                self.project.set_speed(id, speed);
            }
            self.touch();
        }
        if restyle {
            self.commit();
            self.project.restyle_captions(original.id);
            self.touch();
        }
        // Рисовать путь удобно, когда клип виден в кадре.
        if path_edit && self.path_edit.is_none() && !original.contains(self.playhead) {
            self.seek(original.start);
        }
        self.path_edit = path_edit.then_some(original.id);
    }

    fn transition_inspector(&mut self, ui: &mut Ui, id: Id) {
        let body = header(ui, "Переход");
        let Some(mut tr) = self.project.clip(id).and_then(|c| c.transition.clone()) else { return };
        let limit = self.project.transition_limit(id).unwrap_or(MAX_DURATION).clamp(MIN_DURATION, MAX_DURATION);
        let mut edit = Edit::default();
        let mut remove = false;

        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().slider_width = SLIDER_W;
            ui.add_space(12.0);
            let before = tr.kind;
            row(ui, "Вид", |ui| {
                egui::ComboBox::from_id_salt("transition-kind")
                    .width(ui.available_width() - 8.0)
                    .selected_text(tr.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in TransitionKind::ALL {
                            ui.selectable_value(&mut tr.kind, kind, kind.label());
                        }
                    })
                    .response
            });
            if tr.kind != before {
                edit.now();
            }
            let mut duration = tr.duration.min(limit);
            let r = row(ui, "Длительность", |ui| {
                ui.add(egui::Slider::new(&mut duration, MIN_DURATION..=limit).suffix(" с").fixed_decimals(2))
            });
            edit.track(&r);
            if r.changed() {
                tr.duration = duration;
            }
            ui.add_space(10.0);
            remove = widgets::soft_button(ui, Some(icons::TRASH), "Убрать переход").clicked();
            ui.add_space(10.0);
            hint(ui, "Переход стоит на стыке двух клипов и делится между ними пополам, поэтому он не длиннее каждого из них.");
        });

        if remove {
            return self.delete_selected();
        }
        if edit.begin {
            self.commit();
        }
        if edit.changed {
            if let Some(c) = self.project.clip_mut(id) {
                c.transition = Some(tr);
            }
            self.touch();
        }
    }

    fn project_inspector(&mut self, ui: &mut Ui) {
        let body = header(ui, "Проект");
        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            ui.add_space(12.0);
            let r = row(ui, "Название", |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.project.name).desired_width(f32::INFINITY))
            });
            if r.changed() {
                self.dirty = true;
            }

            const SIZES: [(&str, u32, u32); 6] = [
                ("16:9 · 1920×1080", 1920, 1080),
                ("16:9 · 3840×2160", 3840, 2160),
                ("16:9 · 1280×720", 1280, 720),
                ("9:16 · 1080×1920", 1080, 1920),
                ("1:1 · 1080×1080", 1080, 1080),
                ("4:5 · 1080×1350", 1080, 1350),
            ];
            let current = (self.project.width, self.project.height);
            let label = SIZES
                .iter()
                .find(|s| (s.1, s.2) == current)
                .map(|s| s.0.to_owned())
                .unwrap_or_else(|| format!("{}×{}", current.0, current.1));
            let mut size = current;
            row(ui, "Формат кадра", |ui| {
                egui::ComboBox::from_id_salt("project-size")
                    .width(ui.available_width() - 8.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (name, w, h) in SIZES {
                            ui.selectable_value(&mut size, (w, h), name);
                        }
                    })
                    .response
            });
            if size != current {
                self.project.width = size.0;
                self.project.height = size.1;
                self.touch();
            }

            let mut fps = self.project.fps;
            row(ui, "Частота кадров", |ui| {
                egui::ComboBox::from_id_salt("project-fps")
                    .width(ui.available_width() - 8.0)
                    .selected_text(format!("{fps} к/с"))
                    .show_ui(ui, |ui| {
                        for f in [24.0, 25.0, 30.0, 50.0, 60.0] {
                            ui.selectable_value(&mut fps, f, format!("{f} к/с"));
                        }
                    })
                    .response
            });
            if fps != self.project.fps {
                self.project.fps = fps;
                self.touch();
            }

            divider(ui);
            section(ui, "Сведения");
            let clips: usize = self.project.tracks.iter().map(|t| t.clips.len()).sum();
            for (k, v) in [
                ("Длительность", short_time(self.project.duration())),
                ("Клипов", clips.to_string()),
                ("Файлов в библиотеке", self.project.media.len().to_string()),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).color(TEXT2));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v).font(theme::font_mono(12.0)).color(TEXT));
                    });
                });
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Выберите клип на таймлайне, чтобы изменить его свойства.").color(TEXT3));
        });
    }
}
