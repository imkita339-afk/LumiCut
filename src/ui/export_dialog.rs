//! Окно экспорта.

use eframe::egui::{self, vec2, Align, Align2, Layout, RichText, Sense, Ui};

use crate::app::App;
use crate::export::{output_size, Codec, Quality};
use crate::ffmpeg;
use crate::icons;
use crate::model::short_time;
use crate::theme::{self, *};
use crate::ui::{fx, widgets};

const LABEL_W: f32 = 130.0;

fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        // Подписи занимают одну ширину, чтобы поля стояли в столбик.
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 26.0), Sense::hover());
        ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, theme::font(13.0), TEXT2);
        add(ui);
    });
}

fn reveal_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg("/select,").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

impl App {
    pub fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export.open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Экспорт")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(470.0);
                ui.add_space(4.0);
                // Настройки, ход экспорта и итог сменяют друг друга плавно.
                let state = match (&self.export.job, &self.export.result) {
                    (Some(_), _) => 0,
                    (None, Some(Ok(_))) => 1,
                    (None, _) => 2,
                };
                ui.multiply_opacity(fx::enter(ui.ctx(), egui::Id::new("export-state"), state));

                if self.export.job.is_some() {
                    ui.label("Идёт экспорт…");
                    ui.add_space(6.0);
                    widgets::progress(ui, self.export.progress);
                    ui.add_space(4.0);
                    let percent = format!("{:.0}%", self.export.progress.clamp(0.0, 1.0) * 100.0);
                    ui.label(RichText::new(percent).font(theme::font_mono(11.5)).color(TEXT3));
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::soft_button(ui, None, "Отмена").clicked() {
                            if let Some(job) = &self.export.job {
                                job.cancel();
                            }
                        }
                    });
                    return;
                }

                if let Some(Ok(path)) = self.export.result.clone() {
                    ui.horizontal(|ui| {
                        // Галочка выскакивает вместе с залпом конфетти.
                        let (badge, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        let pop = 1.0 + 0.3 * fx::spring(ui.ctx(), egui::Id::new("export-done"), 0.0, 0.0);
                        let badge = fx::zoom(badge, badge.center(), pop);
                        fx::halo(ui.painter(), badge, 11.0, ACCENT.gamma_multiply(0.35), 12.0);
                        ui.painter().circle_filled(badge.center(), badge.width() / 2.0, ACCENT);
                        icons::paint(ui.painter(), badge.shrink(4.5 * pop), icons::CHECK, ON_ACCENT);
                        ui.label(RichText::new("Готово").font(theme::font_semibold(14.0)).color(TEXT));
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new(path.to_string_lossy()).color(TEXT2));
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::accent_button(ui, None, "Закрыть").clicked() {
                            close = true;
                        }
                        if widgets::soft_button(ui, Some(icons::FOLDER), "Показать в папке").clicked() {
                            reveal_in_folder(&path);
                        }
                    });
                    return;
                }

                let project_size = (self.project.width, self.project.height);
                let project_fps = self.project.fps;
                let s = &mut self.export.settings;

                field(ui, "Файл", |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::soft_button(ui, None, "Обзор…").clicked() {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Сохранить видео")
                                .add_filter("Видео MP4", &["mp4"]);
                            if let Some(dir) = s.path.parent() {
                                dialog = dialog.set_directory(dir);
                            }
                            if let Some(name) = s.path.file_name() {
                                dialog = dialog.set_file_name(name.to_string_lossy());
                            }
                            if let Some(mut p) = dialog.save_file() {
                                p.set_extension("mp4");
                                s.path = p;
                            }
                        }
                        let w = ui.available_width();
                        let text = s.path.to_string_lossy().into_owned();
                        let galley = widgets::elided(ui, &text, theme::font(12.5), TEXT, w);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.label(galley).on_hover_text(text);
                        });
                    });
                });

                field(ui, "Разрешение", |ui| {
                    let label = match s.height {
                        None => format!("Как в проекте ({}×{})", project_size.0, project_size.1),
                        Some(h) => format!("{h}p"),
                    };
                    egui::ComboBox::from_id_salt("export-size")
                        .width(ui.available_width())
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut s.height,
                                None,
                                format!("Как в проекте ({}×{})", project_size.0, project_size.1),
                            );
                            for h in [2160u32, 1440, 1080, 720, 480] {
                                ui.selectable_value(&mut s.height, Some(h), format!("{h}p"));
                            }
                        });
                });

                field(ui, "Частота кадров", |ui| {
                    let label = match s.fps {
                        None => format!("Как в проекте ({project_fps} к/с)"),
                        Some(f) => format!("{f} к/с"),
                    };
                    egui::ComboBox::from_id_salt("export-fps")
                        .width(ui.available_width())
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.fps, None, format!("Как в проекте ({project_fps} к/с)"));
                            for f in [24.0, 25.0, 30.0, 50.0, 60.0] {
                                ui.selectable_value(&mut s.fps, Some(f), format!("{f} к/с"));
                            }
                        });
                });

                field(ui, "Кодек", |ui| {
                    let name = |c: Codec| match c {
                        Codec::H264 => "H.264 — совместим со всем",
                        Codec::H265 => "H.265 (HEVC) — файл меньше",
                    };
                    egui::ComboBox::from_id_salt("export-codec")
                        .width(ui.available_width())
                        .selected_text(name(s.codec))
                        .show_ui(ui, |ui| {
                            for c in [Codec::H264, Codec::H265] {
                                ui.selectable_value(&mut s.codec, c, name(c));
                            }
                        });
                });

                field(ui, "Качество", |ui| {
                    let name = |q: Quality| match q {
                        Quality::High => "Высокое",
                        Quality::Standard => "Рекомендуемое",
                        Quality::Compact => "Компактный файл",
                    };
                    egui::ComboBox::from_id_salt("export-quality")
                        .width(ui.available_width())
                        .selected_text(name(s.quality))
                        .show_ui(ui, |ui| {
                            for q in [Quality::High, Quality::Standard, Quality::Compact] {
                                ui.selectable_value(&mut s.quality, q, name(q));
                            }
                        });
                });

                field(ui, "Ускорение", |ui| match ffmpeg::gpu() {
                    Some(gpu) => {
                        widgets::switch(ui, &mut s.accel);
                        let note = if s.accel { format!("кодирует видеокарта {}", gpu.label()) } else { "кодирует процессор".to_owned() };
                        ui.label(RichText::new(note).color(TEXT2));
                    }
                    None => {
                        ui.label(RichText::new("видеокарта для кодирования не найдена, работает процессор").color(TEXT3));
                    }
                });

                ui.add_space(6.0);
                let (ow, oh) = output_size(&self.project, &self.export.settings);
                ui.label(
                    RichText::new(format!(
                        "Длительность {} · кадр {}×{} · формат MP4",
                        short_time(self.project.duration()),
                        ow,
                        oh
                    ))
                    .font(theme::font_mono(11.5))
                    .color(TEXT3),
                );
                if let Some(Err(e)) = &self.export.result {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(DANGER));
                }
                if !self.ffmpeg_ok {
                    ui.add_space(6.0);
                    ui.label(RichText::new("FFmpeg не найден — экспорт недоступен").color(DANGER));
                }

                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::accent_button(ui, Some(icons::EXPORT), "Экспорт").clicked() {
                        self.start_export();
                    }
                    if widgets::soft_button(ui, None, "Отмена").clicked() {
                        close = true;
                    }
                });
            });

        if !open || close {
            if let Some(job) = &self.export.job {
                job.cancel();
            }
            self.export.open = false;
        }
    }
}
