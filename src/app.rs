//! Состояние приложения и главный цикл.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, emath::easing, epaint::Shadow, vec2, Align, Align2, Color32, ColorImage, Frame, Key, Layout, Margin,
    Modifiers, Pos2, RichText, Sense, TextureHandle, TextureOptions, ViewportCommand,
};

use crate::audio::AudioEngine;
use crate::effects::EffectKind;
use crate::export::{ExportJob, ExportMsg, ExportSettings};
use crate::ffmpeg::{self, Probe};
use crate::icons;
use crate::logo;
use crate::look::{FilterKind, Look};
use crate::model::{Clip, History, Id, MediaItem, MediaKind, Project, TrackKind};
use crate::srt;
use crate::text::{self, TextPreset};
use crate::playback::Preview;
use crate::theme::{self, *};
use crate::transition::TransitionKind;
use crate::ui::{fx, widgets};
use crate::vfx::{Fx, FxKind, Vfx, VfxPreset};

pub const PROJECT_EXT: &str = "lumicut";

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Media,
    Audio,
    Text,
    Stickers,
    Effects,
    Transitions,
    Filters,
    Subtitles,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Media,
        Category::Audio,
        Category::Text,
        Category::Stickers,
        Category::Effects,
        Category::Transitions,
        Category::Filters,
        Category::Subtitles,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Media => "Медиа",
            Category::Audio => "Аудио",
            Category::Text => "Текст",
            Category::Stickers => "Стикеры",
            Category::Effects => "Эффекты",
            Category::Transitions => "Переходы",
            Category::Filters => "Фильтры",
            Category::Subtitles => "Субтитры",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::Media => icons::FILM,
            Category::Audio => icons::NOTE,
            Category::Text => icons::TEXT,
            Category::Stickers => icons::STICKER,
            Category::Effects => icons::SPARK,
            Category::Transitions => icons::BOW,
            Category::Filters => icons::RINGS,
            Category::Subtitles => icons::SUBS,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MediaFilter {
    All,
    Video,
    Photo,
    Audio,
}

pub enum JobMsg {
    Probed { path: PathBuf, result: Result<Probe, String>, to_timeline: bool },
    Thumb { media: Id, w: u32, h: u32, data: Vec<u8> },
    Wave { media: Id, peaks: Vec<f32> },
}

pub struct Library {
    pub category: Category,
    pub filter: MediaFilter,
    pub search: String,
    pub thumbs: HashMap<Id, TextureHandle>,
    pub waves: HashMap<Id, Arc<Vec<f32>>>,
    /// Свёрнутые группы файлов во вкладке «Все» (по расширению).
    pub collapsed: HashSet<String>,
    /// Кадры эффектов на плитках и доля эффекта, которую они показывают.
    pub effect_thumbs: HashMap<EffectKind, (f32, TextureHandle)>,
    /// Образцы готовых надписей и фильтров на плитках.
    pub text_thumbs: HashMap<TextPreset, TextureHandle>,
    pub filter_thumbs: HashMap<FilterKind, TextureHandle>,
    pub tx: Sender<JobMsg>,
    pub rx: Receiver<JobMsg>,
    pub importing: usize,
}

/// Медиафайл, который тащат из библиотеки на таймлайн.
pub struct MediaDrag(pub Id);

/// Эффект, который тащат из библиотеки на видеодорожку.
pub struct EffectDrag(pub EffectKind);

/// Переход, который тащат из библиотеки на стык клипов.
pub struct TransitionDrag(pub TransitionKind);

/// Надпись, которую тащат из библиотеки на видеодорожку.
pub struct TextDrag(pub TextPreset);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Blade,
}

#[derive(Clone, Copy)]
pub enum Drag {
    Scrub,
    /// `grab` — расстояние по времени от точки захвата до начала клипа или его края.
    Move { clip: Id, grab: f64 },
    TrimStart { clip: Id, grab: f64 },
    TrimEnd { clip: Id, grab: f64 },
}

pub struct Timeline {
    /// Пикселей на секунду.
    pub pps: f32,
    pub scroll: f32,
    pub snap: bool,
    pub tool: Tool,
    pub drag: Option<Drag>,
    /// Подогнать масштаб под весь проект при следующей отрисовке.
    pub fit: bool,
    /// Точка времени, к которой сейчас притянут перетаскиваемый край.
    pub snapped: Option<f64>,
    /// Геометрия последнего отрисованного кадра.
    pub view: Option<TimelineView>,
}

/// Где на экране находятся линейка и дорожки.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct TimelineView {
    pub left: f32,
    pub pps: f32,
    pub scroll: f32,
    pub ruler: egui::Rect,
    /// Линейка и дорожки целиком: этой областью обрезается всё, что рисуется на таймлайне.
    pub area: egui::Rect,
    /// Индекс дорожки и её прямоугольник.
    pub rows: Vec<(usize, egui::Rect)>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl TimelineView {
    pub fn x(&self, t: f64) -> f32 {
        self.left + t as f32 * self.pps - self.scroll
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorTab {
    Effect,
    Text,
    Video,
    Color,
    Audio,
    Speed,
    Motion,
}

pub enum StageDrag {
    Move { origin: Pos2, x: f32, y: f32 },
    Scale { dist: f32, scale: f32 },
    /// Тащат точку пути с этим номером.
    PathPoint(usize),
    /// Рисуют путь от руки; `from` — первая точка росчерка.
    PathDraw { from: usize },
    /// Двигают маску: место захвата в долях кадра клипа и середина маски в этот момент.
    MaskMove { grab: (f32, f32), x: f32, y: f32 },
    /// Тянут маску за угол: меняется её размер.
    MaskSize,
}

/// Страница приложения: монтаж или редактор эффектов.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Edit,
    Vfx,
}

/// Полка библиотеки на странице VFX.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum VfxShelf {
    Effects,
    Looks,
    Generators,
}

/// Что делает мышь в кадре на странице VFX.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VfxTool {
    /// Двигает и масштабирует клип, как при монтаже.
    Move,
    /// Берёт цвет фона для хромакея из кадра.
    Pipette,
    /// Двигает маску и меняет её размер.
    Mask,
}

/// Состояние страницы VFX.
pub struct VfxUi {
    pub shelf: VfxShelf,
    pub tool: VfxTool,
    /// Вместо выбранного клипа показана его прозрачность.
    pub matte: bool,
    /// В кадре оставлен только выбранный слой.
    pub solo: bool,
    /// Выбранный клип показан без оформления.
    pub original: bool,
    /// Развёрнуты ли разделы: хромакей, наложение, маска и эффекты.
    pub open: [bool; 4],
    /// Образцы эффектов и готовых наборов на плитках.
    pub fx_thumbs: HashMap<FxKind, TextureHandle>,
    pub preset_thumbs: HashMap<VfxPreset, TextureHandle>,
    /// Скопированное оформление клипа: цвет и эффекты.
    pub copied: Option<(Look, Vfx)>,
}

pub struct ExportState {
    pub open: bool,
    pub settings: ExportSettings,
    pub job: Option<ExportJob>,
    pub progress: f32,
    pub result: Option<Result<PathBuf, String>>,
}

#[derive(Clone, Copy)]
pub enum Pending {
    Close,
    New,
    Open,
}

pub struct App {
    pub ctx: egui::Context,
    pub project: Project,
    pub history: History,
    pub project_path: Option<PathBuf>,
    pub dirty: bool,
    /// Счётчик правок таймлайна.
    pub rev: u64,
    pub selected: Option<Id>,
    /// Выбранный переход: клип справа от стыка. Выбран либо клип, либо переход.
    pub sel_transition: Option<Id>,
    /// Клип, которому сейчас рисуют путь в кадре.
    pub path_edit: Option<Id>,
    logo: Option<TextureHandle>,
    pub playhead: f64,
    pub playing: bool,
    play_anchor: (Instant, f64),
    audio_rev: u64,
    audio_restart: Instant,
    pub preview: Preview,
    pub audio: AudioEngine,
    pub ffmpeg_ok: bool,
    pub lib: Library,
    pub tl: Timeline,
    pub inspector_tab: InspectorTab,
    pub page: Page,
    pub vfx_ui: VfxUi,
    pub stage_drag: Option<StageDrag>,
    /// Скопированный клип и дорожка, с которой он взят.
    pub clipboard: Option<(Clip, Id)>,
    /// Где на экране был кадр проекта в последний раз.
    #[cfg_attr(not(test), allow(dead_code))]
    pub canvas: Option<egui::Rect>,
    pub export: ExportState,
    pub pending: Option<Pending>,
    allow_close: bool,
    pub show_shortcuts: bool,
    toast: Option<(String, Instant)>,
    last_title: String,
    /// Параметры запуска: начать воспроизведение и/или встать на момент времени.
    autoplay: bool,
    start_at: Option<f64>,
    start_vfx: bool,
}

fn default_export_path(name: &str) -> PathBuf {
    let dir = std::env::var_os("USERPROFILE")
        .map(|h| PathBuf::from(h).join("Videos"))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let safe: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    dir.join(format!("{}.mp4", if safe.trim().is_empty() { "video" } else { safe.trim() }))
}

impl App {
    pub fn new(ctx: &egui::Context, args: Vec<String>) -> Self {
        theme::install_fonts(ctx);
        theme::apply(ctx);
        let ffmpeg_ok = ffmpeg::available();
        let (tx, rx) = channel();
        let project = Project::new();
        let mut app = App {
            ctx: ctx.clone(),
            export: ExportState {
                open: false,
                settings: ExportSettings::new(default_export_path(&project.name)),
                job: None,
                progress: 0.0,
                result: None,
            },
            project,
            history: History::default(),
            project_path: None,
            dirty: false,
            rev: 0,
            selected: None,
            sel_transition: None,
            path_edit: None,
            logo: None,
            playhead: 0.0,
            playing: false,
            play_anchor: (Instant::now(), 0.0),
            audio_rev: 0,
            audio_restart: Instant::now(),
            preview: Preview::new(ffmpeg_ok),
            audio: AudioEngine::new(),
            ffmpeg_ok,
            lib: Library {
                category: Category::Media,
                filter: MediaFilter::All,
                search: String::new(),
                thumbs: HashMap::new(),
                waves: HashMap::new(),
                collapsed: HashSet::new(),
                effect_thumbs: HashMap::new(),
                text_thumbs: HashMap::new(),
                filter_thumbs: HashMap::new(),
                tx,
                rx,
                importing: 0,
            },
            tl: Timeline { pps: 60.0, scroll: 0.0, snap: true, tool: Tool::Select, drag: None, fit: false, snapped: None, view: None },
            inspector_tab: InspectorTab::Video,
            page: Page::Edit,
            vfx_ui: VfxUi {
                shelf: VfxShelf::Effects,
                tool: VfxTool::Move,
                matte: false,
                solo: false,
                original: false,
                open: [true; 4],
                fx_thumbs: HashMap::new(),
                preset_thumbs: HashMap::new(),
                copied: None,
            },
            stage_drag: None,
            clipboard: None,
            canvas: None,
            pending: None,
            allow_close: false,
            show_shortcuts: false,
            toast: None,
            last_title: String::new(),
            autoplay: false,
            start_at: None,
            start_vfx: false,
        };
        if !ffmpeg_ok {
            app.toast("Не найден FFmpeg: импорт, просмотр и экспорт недоступны");
        }
        #[cfg(not(test))]
        if ffmpeg_ok {
            ffmpeg::detect_gpu();
        }

        // Аргументы: файл проекта и/или медиафайлы; --timeline кладёт их сразу на таймлайн,
        // --seek=СЕКУНДЫ ставит курсор, --play запускает воспроизведение, --vfx открывает VFX-редактор.
        let to_timeline = args.iter().any(|a| a == "--timeline");
        app.autoplay = args.iter().any(|a| a == "--play");
        app.start_vfx = args.iter().any(|a| a == "--vfx");
        app.start_at = args
            .iter()
            .find_map(|a| a.strip_prefix("--seek="))
            .and_then(|v| v.parse::<f64>().ok());
        let mut media = Vec::new();
        for a in args.iter().filter(|a| !a.starts_with("--")) {
            let p = PathBuf::from(a);
            if p.extension().map_or(false, |e| e.eq_ignore_ascii_case(PROJECT_EXT)) {
                app.open_project(&p);
            } else {
                media.push(p);
            }
        }
        if !media.is_empty() {
            app.import_paths(media, to_timeline);
        }
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Запомнить состояние дорожек перед правкой.
    pub fn commit(&mut self) {
        self.history.push(&self.project.tracks);
    }

    /// Отметить, что таймлайн изменился.
    pub fn touch(&mut self) {
        // Переход живёт только на стыке: без соседа слева он пропадает.
        self.project.prune_transitions();
        self.dirty = true;
        self.rev += 1;
    }

    // ---------- Воспроизведение ----------

    pub fn play(&mut self) {
        let end = self.project.duration();
        if end <= 0.0 {
            return;
        }
        if self.playhead >= end - 0.02 {
            self.playhead = 0.0;
        }
        self.playing = true;
        self.play_anchor = (Instant::now(), self.playhead);
        self.restart_audio();
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.audio.stop();
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    fn restart_audio(&mut self) {
        self.audio.play(&self.project, self.playhead);
        self.play_anchor = (Instant::now(), self.playhead);
        self.audio_rev = self.rev;
        self.audio_restart = Instant::now();
    }

    /// Переставить курсор воспроизведения (воспроизведение останавливается).
    pub fn seek(&mut self, t: f64) {
        if self.playing {
            self.pause();
        }
        self.playhead = t.clamp(0.0, self.project.duration().max(0.0));
    }

    pub fn step_frames(&mut self, n: i64) {
        let fps = self.project.fps.max(1.0);
        let frame = (self.playhead * fps).round() as i64 + n;
        self.seek(frame.max(0) as f64 / fps);
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        let t = self
            .audio
            .clock()
            .unwrap_or_else(|| self.play_anchor.1 + self.play_anchor.0.elapsed().as_secs_f64());
        let end = self.project.duration();
        if t >= end {
            self.playhead = end;
            self.pause();
            return;
        }
        self.playhead = t;
        if self.audio_rev != self.rev && self.audio_restart.elapsed() > Duration::from_millis(200) {
            self.restart_audio();
        }
        self.audio.set_volumes(&self.project, self.playhead);
        ctx.request_repaint();
    }

    // ---------- Правки таймлайна ----------

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.project) {
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.project) {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        if let Some(id) = self.selected {
            if self.project.clip(id).is_none() {
                self.selected = None;
            }
        }
        self.touch();
        self.fx_sweep();
    }

    /// Дорожка, начало и конец клипа.
    fn clip_span(&self, id: Id) -> Option<(usize, f64, f64)> {
        let (ti, ci) = self.project.find_clip(id)?;
        let c = &self.project.tracks[ti].clips[ci];
        Some((ti, c.start, c.end()))
    }

    /// Клипы под курсором воспроизведения, к которым применяются действия.
    fn targets_at_playhead(&self) -> Vec<Id> {
        if let Some(id) = self.selected {
            if self.project.clip(id).map_or(false, |c| c.contains(self.playhead)) {
                return vec![id];
            }
        }
        self.project
            .tracks
            .iter()
            .filter(|t| !t.locked)
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.contains(self.playhead))
            .map(|c| c.id)
            .collect()
    }

    pub fn split_at_playhead(&mut self) {
        let targets = self.targets_at_playhead();
        if targets.is_empty() {
            return;
        }
        self.commit();
        let mut changed = false;
        for id in targets {
            if self.project.split(id, self.playhead).is_some() {
                changed = true;
                self.fx_cut(id, self.playhead);
            }
        }
        if changed {
            self.touch();
        } else {
            self.history.undo(&mut self.project);
        }
    }

    pub fn split_clip(&mut self, id: Id, t: f64) {
        self.commit();
        if self.project.split(id, t).is_some() {
            self.touch();
            self.fx_cut(id, t);
        } else {
            self.history.undo(&mut self.project);
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.sel_transition.take() {
            self.commit();
            if let Some(clip) = self.project.clip_mut(id) {
                clip.transition = None;
            }
            self.touch();
            return;
        }
        if let Some(id) = self.selected {
            self.commit();
            let gone = self.clip_span(id);
            if self.project.remove_clip(id) {
                self.selected = None;
                self.touch();
                if let Some((track, from, to)) = gone {
                    self.fx_vanish(track, from, to);
                }
            }
        }
    }

    /// Удаляет выбранный клип и сдвигает следующие за ним клипы дорожки влево.
    pub fn ripple_delete_selected(&mut self) {
        if self.sel_transition.is_some() {
            return self.delete_selected();
        }
        let Some(id) = self.selected else { return };
        self.commit();
        let gone = self.clip_span(id);
        if self.project.ripple_delete(id) {
            self.selected = None;
            self.touch();
            if let Some((track, from, to)) = gone {
                self.fx_vanish(track, from, to);
            }
        }
    }

    /// Ставит копию выбранного клипа сразу после него.
    pub fn duplicate_selected(&mut self) {
        let Some(id) = self.selected else { return };
        self.commit();
        match self.project.duplicate(id) {
            Some(copy) => self.clip_added(copy),
            None => {
                self.history.undo(&mut self.project);
            }
        }
    }

    pub fn copy_selected(&mut self) {
        let Some((ti, ci)) = self.selected.and_then(|id| self.project.find_clip(id)) else { return };
        let track = &self.project.tracks[ti];
        self.clipboard = Some((track.clips[ci].clone(), track.id));
        // В буфере обмена системы должен лежать хоть какой-то текст, иначе команда «Вставить» до окна не дойдёт.
        self.ctx.copy_text("Клип LumiCut".to_owned());
        self.toast("Клип скопирован: Ctrl+V вставит его у курсора");
    }

    /// Вставляет скопированный клип у курсора воспроизведения: на его дорожку, а если она
    /// не подходит — на первую подходящую.
    pub fn paste(&mut self) {
        let Some((clip, home)) = self.clipboard.clone() else { return };
        let fits = |i: &usize| self.project.clip_fits(&clip, *i) && !self.project.tracks[*i].locked;
        let track = self.project.tracks.iter().position(|t| t.id == home).filter(fits);
        let Some(track) = track.or_else(|| (0..self.project.tracks.len()).find(fits)) else {
            self.toast("Для клипа нет подходящей дорожки");
            return;
        };
        self.commit();
        match self.project.paste(&clip, track, self.playhead) {
            Some(copy) => self.clip_added(copy),
            None => {
                self.history.undo(&mut self.project);
            }
        }
    }

    /// Новый клип выбран.
    fn clip_added(&mut self, id: Id) {
        self.selected = Some(id);
        self.touch();
        self.fx_land(id, 0.6);
    }

    /// Обрезать всё слева (или справа) от курсора у клипов под ним.
    pub fn trim_at_playhead(&mut self, left: bool) {
        let targets = self.targets_at_playhead();
        if targets.is_empty() {
            return;
        }
        self.commit();
        for id in targets {
            let before = self.clip_span(id);
            if left {
                self.project.trim_start(id, self.playhead);
            } else {
                self.project.trim_end(id, self.playhead);
            }
            if let (Some((track, a, b)), Some((_, a2, b2))) = (before, self.clip_span(id)) {
                let (from, to) = if left { (a, a2) } else { (b2, b) };
                if to > from {
                    self.fx_vanish(track, from, to);
                }
            }
        }
        self.touch();
    }

    // ---------- Эффекты и переходы ----------

    /// Эффект встаёт на верхнюю видеодорожку у курсора воспроизведения.
    pub fn add_effect(&mut self, kind: EffectKind) {
        let Some(track) = self.project.overlay_track() else {
            self.toast("Все видеодорожки заблокированы");
            return;
        };
        self.commit();
        match self.project.add_effect(track, kind, self.playhead) {
            Some(id) => self.effect_added(id),
            None => {
                self.history.undo(&mut self.project);
            }
        }
    }

    /// Новый клип с эффектом выбран, справа открыты его настройки.
    pub fn effect_added(&mut self, id: Id) {
        self.selected = Some(id);
        self.inspector_tab = InspectorTab::Effect;
        self.touch();
        self.fx_land(id, 1.0);
    }

    // ---------- Текст, субтитры и фильтры ----------

    /// Надпись встаёт на верхнюю видеодорожку у курсора воспроизведения.
    pub fn add_text(&mut self, preset: TextPreset) {
        let Some(track) = self.project.overlay_track() else {
            self.toast("Все видеодорожки заблокированы");
            return;
        };
        self.commit();
        let text = preset.text(self.project.height);
        match self.project.add_text(track, text, self.playhead, text::DURATION) {
            Some(id) => self.text_added(id),
            None => {
                self.history.undo(&mut self.project);
            }
        }
    }

    /// Новая надпись выбрана, справа открыт её текст.
    pub fn text_added(&mut self, id: Id) {
        self.selected = Some(id);
        self.inspector_tab = InspectorTab::Text;
        self.touch();
        self.fx_land(id, 1.0);
    }

    /// Новый субтитр у курсора воспроизведения.
    pub fn add_caption(&mut self) {
        self.commit();
        match self.project.add_caption("Текст субтитра", self.playhead, text::DURATION) {
            Some(id) => self.text_added(id),
            None => {
                self.history.undo(&mut self.project);
                self.toast("Дорожка субтитров заблокирована");
            }
        }
    }

    pub fn import_srt_dialog(&mut self) {
        let picked = rfd::FileDialog::new().set_title("Импорт субтитров").add_filter("Субтитры SubRip", &["srt"]).pick_file();
        if let Some(path) = picked {
            self.import_srt(&path);
        }
    }

    /// Реплики из файла становятся субтитрами на своей дорожке.
    pub fn import_srt(&mut self, path: &Path) {
        let cues = match std::fs::read(path) {
            Ok(bytes) => srt::parse(&srt::decode(&bytes)),
            Err(e) => return self.toast(format!("Не удалось прочитать файл: {e}")),
        };
        if cues.is_empty() {
            return self.toast("В файле не нашлось субтитров");
        }
        self.commit();
        let added = cues.iter().filter(|c| self.project.add_caption(&c.text, c.start, c.end - c.start).is_some()).count();
        self.touch();
        self.toast(format!("Добавлено субтитров: {added}"));
    }

    /// Субтитры проекта как реплики файла .srt.
    pub fn caption_cues(&self) -> Vec<srt::Cue> {
        let content = |c: &Clip| c.text.as_ref().map_or(String::new(), |t| t.content.clone());
        self.project.captions().iter().map(|c| srt::Cue { start: c.start, end: c.end(), text: content(c) }).collect()
    }

    pub fn export_srt_dialog(&mut self) {
        let cues = self.caption_cues();
        if cues.is_empty() {
            return self.toast("В проекте пока нет субтитров");
        }
        let picked = rfd::FileDialog::new()
            .set_title("Сохранить субтитры")
            .add_filter("Субтитры SubRip", &["srt"])
            .set_file_name(format!("{}.srt", self.project.name))
            .save_file();
        if let Some(mut path) = picked {
            path.set_extension("srt");
            match std::fs::write(&path, srt::write(&cues)) {
                Ok(()) => self.toast(format!("Субтитры сохранены: {}", path.to_string_lossy())),
                Err(e) => self.toast(format!("Не удалось сохранить файл: {e}")),
            }
        }
    }

    // ---------- Страница VFX и оформление клипа ----------

    pub fn set_page(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        self.page = page;
        // Инструменты кадра и режимы просмотра живут только на странице VFX.
        self.vfx_ui.tool = VfxTool::Move;
        (self.vfx_ui.matte, self.vfx_ui.solo, self.vfx_ui.original) = (false, false, false);
        self.stage_drag = None;
        if page == Page::Vfx {
            self.path_edit = None;
            // Без выбора работа начинается с верхнего клипа под курсором.
            if self.selected.is_none() {
                self.selected = self.vfx_target();
            }
        }
    }

    /// Можно ли оформлять клип: это видео или картинка, а не звук, эффект или надпись.
    pub fn stylable(&self, clip: &Clip) -> bool {
        !clip.generated() && self.project.media(clip.media).map_or(false, |m| m.kind != MediaKind::Audio)
    }

    /// Клип для оформления: выбранный, а без выбора — верхний под курсором воспроизведения.
    pub fn vfx_target(&self) -> Option<Id> {
        let chosen = self.selected.and_then(|id| self.project.clip(id)).filter(|c| self.stylable(c));
        let under = || self.project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.contains(self.playhead)).find(|c| self.stylable(c));
        chosen.or_else(under).map(|c| c.id)
    }

    /// Правит оформление клипа одной правкой, которую можно отменить; клип становится выбранным.
    fn restyle(&mut self, id: Id, edit: impl FnOnce(&mut Clip)) {
        self.commit();
        if let Some(clip) = self.project.clip_mut(id) {
            edit(clip);
        }
        self.selected = Some(id);
        self.touch();
        self.fx_land(id, 0.5);
    }

    /// Эффект встаёт в конец стопки выбранного клипа, а без выбора — клипа под курсором.
    pub fn add_fx(&mut self, kind: FxKind) {
        let Some(id) = self.vfx_target() else {
            return self.toast("Выберите видео или картинку на таймлайне: эффект ляжет на этот клип");
        };
        self.restyle(id, |clip| clip.vfx.stack.push(Fx::new(kind)));
        self.vfx_ui.open[3] = true;
    }

    /// Готовый набор заменяет стопку эффектов клипа.
    pub fn apply_vfx_preset(&mut self, preset: VfxPreset) {
        let Some(id) = self.vfx_target() else {
            return self.toast("Выберите видео или картинку на таймлайне: набор эффектов ляжет на этот клип");
        };
        self.restyle(id, |clip| clip.vfx.stack = preset.stack());
        self.vfx_ui.open[3] = true;
    }

    /// Запоминает цвет и эффекты выбранного клипа, чтобы перенести их на другой.
    pub fn copy_style(&mut self) {
        let style = self.selected.and_then(|id| self.project.clip(id)).filter(|c| self.stylable(c)).map(|c| (c.look.clone(), c.vfx.clone()));
        match style {
            Some(style) => {
                self.vfx_ui.copied = Some(style);
                self.toast("Оформление скопировано: выберите другой клип и вставьте его");
            }
            None => self.toast("Выберите видео или картинку, чьё оформление нужно скопировать"),
        }
    }

    pub fn paste_style(&mut self) {
        let Some((look, vfx)) = self.vfx_ui.copied.clone() else { return };
        let Some(id) = self.vfx_target() else {
            return self.toast("Выберите видео или картинку: оформление ляжет на этот клип");
        };
        self.restyle(id, |clip| (clip.look, clip.vfx) = (look, vfx));
    }

    /// Готовый фильтр ложится на выбранный клип, а без выбора — на верхний клип под курсором.
    pub fn apply_filter(&mut self, kind: FilterKind) {
        let Some(id) = self.vfx_target() else {
            return self.toast("Выберите видео или картинку на таймлайне: фильтр ляжет на этот клип");
        };
        self.commit();
        if let Some(clip) = self.project.clip_mut(id) {
            clip.look.filter = kind;
            clip.look.strength = 1.0;
        }
        self.selected = Some(id);
        self.inspector_tab = InspectorTab::Color;
        self.touch();
        self.fx_land(id, 0.5);
    }

    /// Ставит переход на стык перед клипом `owner`.
    pub fn apply_transition(&mut self, owner: Id, kind: TransitionKind) {
        self.commit();
        if self.project.set_transition(owner, kind) {
            self.selected = None;
            self.sel_transition = Some(owner);
            self.touch();
            self.fx_land(owner, 0.5);
        } else {
            self.history.undo(&mut self.project);
        }
    }

    /// Переход по двойному щелчку: на стык у выбранного клипа, иначе на ближайший к курсору.
    pub fn apply_transition_auto(&mut self, kind: TransitionKind) {
        let at_selected = self.selected.and_then(|id| {
            let (ti, ci) = self.project.find_clip(id)?;
            let clip = &self.project.tracks[ti].clips[ci];
            if self.project.tracks[ti].kind != TrackKind::Video || self.project.tracks[ti].locked {
                return None;
            }
            let after = self.project.next_adjacent(ti, clip).map(|next| next.id);
            after.or_else(|| self.project.prev_adjacent(ti, clip).map(|_| id))
        });
        let nearest = || {
            let mut best: Option<(Id, f64)> = None;
            for (ti, track) in self.project.tracks.iter().enumerate() {
                if track.locked {
                    continue;
                }
                for (owner, t) in self.project.junctions(ti) {
                    let d = (t - self.playhead).abs();
                    if best.map_or(true, |b| d < b.1) {
                        best = Some((owner, d));
                    }
                }
            }
            best.map(|b| b.0)
        };
        match at_selected.or(self.sel_transition).or_else(nearest) {
            Some(owner) => self.apply_transition(owner, kind),
            None => self.toast("Переход ставится на стык клипов, стоящих вплотную: разрежьте клип или придвиньте клипы друг к другу"),
        }
    }

    // ---------- Импорт ----------

    pub fn import_dialog(&mut self) {
        let mut all: Vec<&str> = Vec::new();
        all.extend(ffmpeg::VIDEO_EXT);
        all.extend(ffmpeg::AUDIO_EXT);
        all.extend(ffmpeg::PHOTO_EXT);
        let picked = rfd::FileDialog::new()
            .set_title("Импорт медиа")
            .add_filter("Медиафайлы", &all)
            .add_filter("Видео", ffmpeg::VIDEO_EXT)
            .add_filter("Аудио", ffmpeg::AUDIO_EXT)
            .add_filter("Изображения", ffmpeg::PHOTO_EXT)
            .add_filter("Все файлы", &["*"])
            .pick_files();
        if let Some(paths) = picked {
            self.import_paths(paths, false);
        }
    }

    pub fn import_paths(&mut self, paths: Vec<PathBuf>, to_timeline: bool) {
        if !self.ffmpeg_ok {
            self.toast("Не найден FFmpeg: импорт недоступен");
            return;
        }
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| p.is_file() && self.project.media_by_path(p).is_none())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.lib.importing += paths.len();
        let tx = self.lib.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for path in paths {
                let result = ffmpeg::probe(&path);
                if tx.send(JobMsg::Probed { path, result, to_timeline }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }

    /// Миниатюра и форма волны считаются в фоне.
    fn request_assets(&self, media: &MediaItem) {
        let tx = self.lib.tx.clone();
        let ctx = self.ctx.clone();
        let m = media.clone();
        std::thread::spawn(move || {
            if m.kind != MediaKind::Audio && m.width > 0 && m.height > 0 {
                let w = 192u32;
                let h = ffmpeg::even(w as f32 * m.height as f32 / m.width as f32);
                let t = if m.kind == MediaKind::Video { (m.duration * 0.1).min(1.0) } else { 0.0 };
                if let Some(data) = ffmpeg::frame_rgba(&m.path, t, w, h) {
                    let _ = tx.send(JobMsg::Thumb { media: m.id, w, h, data });
                    ctx.request_repaint();
                }
            }
            if m.has_audio {
                if let Some(peaks) = ffmpeg::waveform(&m.path) {
                    let _ = tx.send(JobMsg::Wave { media: m.id, peaks });
                    ctx.request_repaint();
                }
            }
        });
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.lib.rx.try_recv() {
            match msg {
                JobMsg::Probed { path, result, to_timeline } => {
                    self.lib.importing = self.lib.importing.saturating_sub(1);
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string_lossy().into_owned());
                    match result {
                        Ok(p) => {
                            if self.project.media_by_path(&path).is_some() {
                                continue;
                            }
                            let id = self.project.alloc_id();
                            let item = MediaItem {
                                id,
                                path,
                                name,
                                kind: p.kind,
                                duration: p.duration,
                                width: p.width,
                                height: p.height,
                                fps: p.fps,
                                has_audio: p.has_audio,
                            };
                            self.request_assets(&item);
                            self.project.media.push(item);
                            self.dirty = true;
                            if to_timeline {
                                self.commit();
                                if let Some(c) = self.project.append_clip(id) {
                                    self.selected = Some(c);
                                    self.fx_land(c, 1.0);
                                }
                                self.touch();
                                self.tl.fit = true;
                            }
                        }
                        Err(e) => self.toast(format!("Не удалось импортировать «{name}»: {e}")),
                    }
                }
                JobMsg::Thumb { media, w, h, data } => {
                    let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &data);
                    let tex = ctx.load_texture(format!("thumb-{media}"), img, TextureOptions::LINEAR);
                    self.lib.thumbs.insert(media, tex);
                }
                JobMsg::Wave { media, peaks } => {
                    self.lib.waves.insert(media, Arc::new(peaks));
                }
            }
        }
    }

    // ---------- Проект ----------

    fn reset_view(&mut self) {
        self.pause();
        self.preview.clear();
        self.lib.thumbs.clear();
        self.lib.waves.clear();
        self.history.clear();
        self.selected = None;
        self.sel_transition = None;
        self.path_edit = None;
        self.stage_drag = None;
        self.vfx_ui.tool = VfxTool::Move;
        self.playhead = 0.0;
        self.tl.scroll = 0.0;
        self.tl.fit = true;
        self.rev += 1;
    }

    fn new_project(&mut self) {
        self.reset_view();
        self.project = Project::new();
        self.project_path = None;
        self.dirty = false;
        self.export.settings.path = default_export_path(&self.project.name);
    }

    fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Открыть проект")
            .add_filter("Проект LumiCut", &[PROJECT_EXT])
            .pick_file();
        if let Some(path) = picked {
            self.open_project(&path);
        }
    }

    pub fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(project) => {
                self.reset_view();
                self.project = project;
                self.project_path = Some(path.to_owned());
                self.dirty = false;
                self.export.settings.path = default_export_path(&self.project.name);
                let missing = self.project.media.iter().filter(|m| !m.path.is_file()).count();
                for m in &self.project.media {
                    if m.path.is_file() {
                        self.request_assets(m);
                    }
                }
                if missing > 0 {
                    self.toast(format!("Не найдено исходных файлов: {missing}"));
                }
            }
            Err(e) => self.toast(format!("Не удалось открыть проект: {e}")),
        }
    }

    pub fn save(&mut self) -> bool {
        match self.project_path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) -> bool {
        let picked = rfd::FileDialog::new()
            .set_title("Сохранить проект")
            .add_filter("Проект LumiCut", &[PROJECT_EXT])
            .set_file_name(format!("{}.{PROJECT_EXT}", self.project.name))
            .save_file();
        match picked {
            Some(mut path) => {
                if path.extension().is_none() {
                    path.set_extension(PROJECT_EXT);
                }
                self.save_to(&path)
            }
            None => false,
        }
    }

    fn save_to(&mut self, path: &Path) -> bool {
        match self.project.save(path) {
            Ok(()) => {
                self.project_path = Some(path.to_owned());
                self.dirty = false;
                true
            }
            Err(e) => {
                self.toast(format!("Не удалось сохранить проект: {e}"));
                false
            }
        }
    }

    fn has_unsaved(&self) -> bool {
        self.dirty && !self.project.media.is_empty()
    }

    pub fn request(&mut self, action: Pending) {
        if self.has_unsaved() {
            self.pending = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Pending) {
        match action {
            Pending::Close => {
                self.allow_close = true;
                self.ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Pending::New => self.new_project(),
            Pending::Open => self.open_dialog(),
        }
    }

    // ---------- Экспорт ----------

    pub fn open_export(&mut self) {
        if self.project.is_empty() {
            self.toast("На таймлайне пока нет клипов");
            return;
        }
        self.pause();
        self.export.result = None;
        self.export.open = true;
    }

    pub fn start_export(&mut self) {
        // Видеокарта кодирует, если это включено и она нашлась.
        let settings = &mut self.export.settings;
        settings.gpu = if settings.accel { ffmpeg::gpu() } else { None };
        match crate::export::start(&self.project, &self.export.settings) {
            Ok(job) => {
                self.export.job = Some(job);
                self.export.progress = 0.0;
                self.export.result = None;
            }
            Err(e) => self.export.result = Some(Err(e)),
        }
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export.job else { return };
        let mut finished = None;
        while let Ok(msg) = job.rx.try_recv() {
            match msg {
                ExportMsg::Progress(p) => self.export.progress = p,
                ExportMsg::Done => finished = Some(Ok(self.export.settings.path.clone())),
                ExportMsg::Failed(e) => finished = Some(Err(e)),
            }
        }
        if let Some(result) = finished {
            if result.is_ok() {
                fx::confetti(ctx, ctx.screen_rect().center() - vec2(0.0, 40.0));
                fx::kick(ctx, egui::Id::new("export-done"), fx::KICK);
            }
            self.export.job = None;
            self.export.result = Some(result);
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    // ---------- Кадр интерфейса ----------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.export.open || self.pending.is_some() {
            return;
        }
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let hit = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));

        if hit(ctrl_shift, Key::Z) || hit(ctrl, Key::Y) {
            self.redo();
        } else if hit(ctrl, Key::Z) {
            self.undo();
        }
        if hit(ctrl_shift, Key::S) {
            self.save_as();
        } else if hit(ctrl, Key::S) {
            self.save();
        }
        if hit(ctrl, Key::O) {
            self.request(Pending::Open);
        }
        if hit(ctrl, Key::N) {
            self.request(Pending::New);
        }
        if hit(ctrl, Key::I) {
            self.import_dialog();
        }
        if hit(ctrl, Key::E) {
            self.open_export();
        }
        if hit(ctrl, Key::B) {
            self.split_at_playhead();
        }
        if hit(ctrl, Key::Num1) {
            self.set_page(Page::Edit);
        }
        if hit(ctrl, Key::Num2) {
            self.set_page(Page::Vfx);
        }

        if ctx.wants_keyboard_input() {
            return;
        }
        if hit(ctrl, Key::D) {
            self.duplicate_selected();
        }
        // «Копировать» и «Вставить» приходят не клавишами, а готовыми командами.
        // С нажатым Alt они переносят не сам клип, а его оформление: цвет и эффекты.
        let (copy, paste, alt) = ctx.input(|i| {
            let has = |f: fn(&egui::Event) -> bool| i.events.iter().any(f);
            (has(|e| matches!(e, egui::Event::Copy)), has(|e| matches!(e, egui::Event::Paste(_))), i.modifiers.alt)
        });
        let style = Modifiers::COMMAND | Modifiers::ALT;
        if hit(style, Key::C) || (copy && alt) {
            self.copy_style();
        } else if copy {
            self.copy_selected();
        }
        if hit(style, Key::V) || (paste && alt) {
            self.paste_style();
        } else if paste {
            self.paste();
        }
        if ctx.input(|i| i.modifiers.shift_only() && i.key_pressed(Key::Delete)) {
            self.ripple_delete_selected();
        }
        let plain = |k: Key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(k));
        if plain(Key::Space) {
            self.toggle_play();
        }
        if plain(Key::Delete) || plain(Key::Backspace) {
            self.delete_selected();
        }
        if plain(Key::ArrowLeft) {
            self.step_frames(-1);
        }
        if plain(Key::ArrowRight) {
            self.step_frames(1);
        }
        if plain(Key::Home) {
            self.seek(0.0);
        }
        if plain(Key::End) {
            self.seek(self.project.duration());
        }
        if plain(Key::V) {
            self.tl.tool = Tool::Select;
        }
        if plain(Key::B) {
            self.tl.tool = Tool::Blade;
        }
        if plain(Key::Q) {
            self.trim_at_playhead(true);
        }
        if plain(Key::W) {
            self.trim_at_playhead(false);
        }
        if plain(Key::Escape) {
            // Сначала заканчивается рисование пути или работа инструмента в кадре,
            // выделение снимается следующим нажатием.
            if self.vfx_ui.tool != VfxTool::Move {
                self.vfx_ui.tool = VfxTool::Move;
            } else if self.path_edit.take().is_none() {
                self.selected = None;
                self.sel_transition = None;
                self.tl.tool = Tool::Select;
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(44.0)
            .frame(Frame::none().fill(PANEL).inner_margin(Margin { left: 12.0, right: 10.0, top: 7.0, bottom: 7.0 }))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                ui.spacing_mut().interact_size.y = 30.0;
                egui::menu::bar(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    fx::halo(ui.painter(), rect.shrink(1.0), 6.0, ACCENT.gamma_multiply(0.3), 12.0);
                    let logo = self.logo.get_or_insert_with(|| {
                        const SIDE: u32 = 88;
                        let image = ColorImage::from_rgba_unmultiplied([SIDE as usize; 2], &logo::rgba(SIDE));
                        ctx.load_texture("logo", image, TextureOptions::LINEAR)
                    });
                    egui::Image::new(&*logo).paint_at(ui, rect);
                    ui.label(RichText::new("LumiCut").font(theme::font_semibold(14.0)).color(TEXT));
                    ui.add_space(6.0);

                    ui.menu_button(RichText::new("Файл").color(TEXT2), |ui| {
                        ui.set_min_width(230.0);
                        if ui.add(egui::Button::new("Новый проект").shortcut_text("Ctrl+N")).clicked() {
                            ui.close_menu();
                            self.request(Pending::New);
                        }
                        if ui.add(egui::Button::new("Открыть проект…").shortcut_text("Ctrl+O")).clicked() {
                            ui.close_menu();
                            self.request(Pending::Open);
                        }
                        ui.separator();
                        if ui.add(egui::Button::new("Сохранить").shortcut_text("Ctrl+S")).clicked() {
                            ui.close_menu();
                            self.save();
                        }
                        if ui.add(egui::Button::new("Сохранить как…").shortcut_text("Ctrl+Shift+S")).clicked() {
                            ui.close_menu();
                            self.save_as();
                        }
                        ui.separator();
                        if ui.add(egui::Button::new("Импорт медиа…").shortcut_text("Ctrl+I")).clicked() {
                            ui.close_menu();
                            self.import_dialog();
                        }
                        if ui.add(egui::Button::new("Экспорт…").shortcut_text("Ctrl+E")).clicked() {
                            ui.close_menu();
                            self.open_export();
                        }
                        ui.separator();
                        if ui.button("Выход").clicked() {
                            ui.close_menu();
                            self.request(Pending::Close);
                        }
                    });
                    ui.menu_button(RichText::new("Правка").color(TEXT2), |ui| {
                        ui.set_min_width(230.0);
                        let undo = egui::Button::new("Отменить").shortcut_text("Ctrl+Z");
                        if ui.add_enabled(self.history.can_undo(), undo).clicked() {
                            ui.close_menu();
                            self.undo();
                        }
                        let redo = egui::Button::new("Повторить").shortcut_text("Ctrl+Shift+Z");
                        if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                            ui.close_menu();
                            self.redo();
                        }
                        ui.separator();
                        if ui.add(egui::Button::new("Разделить").shortcut_text("Ctrl+B")).clicked() {
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
                    });
                    ui.menu_button(RichText::new("Вид").color(TEXT2), |ui| {
                        ui.set_min_width(230.0);
                        if ui.add(egui::Button::new("Монтаж").shortcut_text("Ctrl+1")).clicked() {
                            ui.close_menu();
                            self.set_page(Page::Edit);
                        }
                        if ui.add(egui::Button::new("VFX-редактор").shortcut_text("Ctrl+2")).clicked() {
                            ui.close_menu();
                            self.set_page(Page::Vfx);
                        }
                        ui.separator();
                        if ui.button("Показать весь проект").clicked() {
                            ui.close_menu();
                            self.tl.fit = true;
                        }
                        ui.checkbox(&mut self.tl.snap, "Привязка на таймлайне");
                    });
                    ui.menu_button(RichText::new("Справка").color(TEXT2), |ui| {
                        if ui.button("Горячие клавиши").clicked() {
                            ui.close_menu();
                            self.show_shortcuts = true;
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::accent_button(ui, Some(icons::EXPORT), "Экспорт").clicked() {
                            self.open_export();
                        }
                        ui.add_space(6.0);
                        let now = if self.page == Page::Edit { 0 } else { 1 };
                        match widgets::segmented(ui, egui::Id::new("page"), &["Монтаж", "VFX"], now) {
                            Some(0) => self.set_page(Page::Edit),
                            Some(_) => self.set_page(Page::Vfx),
                            None => {}
                        }
                    });
                });

                let status = if self.project_path.is_none() && !self.has_unsaved() {
                    "не сохранён"
                } else if self.dirty {
                    "есть изменения"
                } else {
                    "сохранено"
                };
                ui.painter().text(
                    full.center(),
                    Align2::CENTER_CENTER,
                    format!("{}  ·  {status}", self.project.name),
                    theme::font(13.0),
                    TEXT2,
                );
            });
    }

    /// Команды над выбранным клипом: общая часть меню «Правка» и меню таймлайна.
    pub fn clip_commands(&mut self, ui: &mut egui::Ui) {
        let clip = self.selected.is_some();
        if ui.add_enabled(clip, egui::Button::new("Удалить и сдвинуть следующие").shortcut_text("Shift+Del")).clicked() {
            ui.close_menu();
            self.ripple_delete_selected();
        }
        ui.separator();
        if ui.add_enabled(clip, egui::Button::new("Дублировать").shortcut_text("Ctrl+D")).clicked() {
            ui.close_menu();
            self.duplicate_selected();
        }
        if ui.add_enabled(clip, egui::Button::new("Копировать").shortcut_text("Ctrl+C")).clicked() {
            ui.close_menu();
            self.copy_selected();
        }
        if ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Вставить у курсора").shortcut_text("Ctrl+V")).clicked() {
            ui.close_menu();
            self.paste();
        }
        ui.separator();
        if ui.add_enabled(clip, egui::Button::new("Копировать оформление").shortcut_text("Ctrl+Alt+C")).clicked() {
            ui.close_menu();
            self.copy_style();
        }
        if ui.add_enabled(self.vfx_ui.copied.is_some(), egui::Button::new("Вставить оформление").shortcut_text("Ctrl+Alt+V")).clicked() {
            ui.close_menu();
            self.paste_style();
        }
    }

    /// Подпись команды удаления и есть ли что удалять.
    pub fn deletable(&self) -> (&'static str, bool) {
        if self.sel_transition.is_some() {
            ("Удалить переход", true)
        } else {
            ("Удалить клип", self.selected.is_some())
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending else { return };
        egui::Window::new("Несохранённые изменения")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(format!("Сохранить изменения в проекте «{}»?", self.project.name));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::accent_button(ui, None, "Сохранить").clicked() && self.save() {
                        self.pending = None;
                        self.perform(action);
                    }
                    if widgets::soft_button(ui, None, "Не сохранять").clicked() {
                        self.pending = None;
                        self.dirty = false;
                        self.perform(action);
                    }
                    if widgets::soft_button(ui, None, "Отмена").clicked() {
                        self.pending = None;
                    }
                });
            });
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new("Горячие клавиши")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts").num_columns(2).spacing([28.0, 6.0]).show(ui, |ui| {
                    for (keys, what) in [
                        ("Пробел", "Воспроизведение / пауза"),
                        ("← / →", "Кадр назад / вперёд"),
                        ("Home / End", "В начало / в конец"),
                        ("Ctrl+B", "Разделить клип"),
                        ("Q / W", "Обрезать слева / справа от курсора"),
                        ("Del", "Удалить клип или переход"),
                        ("Shift+Del", "Удалить клип и сдвинуть следующие"),
                        ("Ctrl+D", "Дублировать клип"),
                        ("Ctrl+C / Ctrl+V", "Копировать / вставить клип"),
                        ("Ctrl+Alt+C / Ctrl+Alt+V", "Копировать / вставить цвет и эффекты клипа"),
                        ("Ctrl+1 / Ctrl+2", "Монтаж / VFX-редактор"),
                        ("Esc", "Закончить рисование пути, убрать инструмент, снять выделение"),
                        ("V / B", "Выделение / лезвие"),
                        ("Ctrl+Z / Ctrl+Shift+Z", "Отменить / повторить"),
                        ("Ctrl+колесо", "Масштаб таймлайна"),
                        ("Ctrl+I", "Импорт медиа"),
                        ("Ctrl+S", "Сохранить проект"),
                        ("Ctrl+E", "Экспорт"),
                    ] {
                        ui.label(RichText::new(keys).font(theme::font_mono(12.0)).color(TEXT));
                        ui.label(RichText::new(what).color(TEXT2));
                        ui.end_row();
                    }
                });
            });
        self.show_shortcuts = open;
    }

    fn drag_ghost(&self, ctx: &egui::Context) {
        // Подпись того, что тащат из библиотеки: файл, эффект или переход.
        let name = if let Some(drag) = egui::DragAndDrop::payload::<MediaDrag>(ctx) {
            let Some(media) = self.project.media(drag.0) else { return };
            media.name.clone()
        } else if let Some(drag) = egui::DragAndDrop::payload::<EffectDrag>(ctx) {
            drag.0.label().to_owned()
        } else if let Some(drag) = egui::DragAndDrop::payload::<TransitionDrag>(ctx) {
            drag.0.label().to_owned()
        } else if let Some(drag) = egui::DragAndDrop::payload::<TextDrag>(ctx) {
            drag.0.label().to_owned()
        } else {
            return;
        };
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("media-drag")));
        let galley = painter.layout_no_wrap(name, theme::font(12.0), TEXT);
        let rect = egui::Rect::from_min_size(pos + vec2(12.0, 12.0), galley.size() + vec2(16.0, 10.0));
        painter.rect_filled(rect, 6.0, HOVER);
        painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, ACCENT));
        painter.galley(rect.min + vec2(8.0, 5.0), galley, TEXT);
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        const LIFE: f32 = 5.0;
        const FADE: f32 = 0.4;
        let Some((msg, at)) = &self.toast else { return };
        let age = at.elapsed().as_secs_f32();
        if age > LIFE {
            self.toast = None;
            return;
        }
        // Выезжает снизу с перелётом, в конце растворяется.
        let rise = easing::back_out((age / 0.3).min(1.0));
        let opacity = (age / 0.15).min(1.0) * ((LIFE - age) / FADE).min(1.0);
        let shadow = Shadow { offset: vec2(0.0, 6.0), blur: 18.0, spread: 0.0, color: Color32::from_black_alpha(110) };
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0 + 30.0 * (1.0 - rise)))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                Frame::none()
                    .fill(HOVER)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .rounding(8.0)
                    .shadow(shadow)
                    .inner_margin(Margin::symmetric(14.0, 9.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(TEXT));
                    });
            });
        // Пока уведомление стоит на месте, перерисовка нужна только к началу растворения.
        let idle = if age < 0.3 { 0.0 } else { (LIFE - FADE - age).clamp(0.0, 0.5) };
        ctx.request_repaint_after(Duration::from_secs_f32(idle));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

impl App {
    /// Один кадр интерфейса.
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.poll_jobs(ctx);
        self.poll_export(ctx);

        if self.lib.importing == 0 && !self.project.is_empty() {
            if let Some(t) = self.start_at.take() {
                self.seek(t);
            }
            if std::mem::take(&mut self.start_vfx) {
                self.set_page(Page::Vfx);
            }
            if std::mem::take(&mut self.autoplay) {
                self.play();
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let (projects, media): (Vec<PathBuf>, Vec<PathBuf>) = dropped
                .into_iter()
                .partition(|p| p.extension().map_or(false, |e| e.eq_ignore_ascii_case(PROJECT_EXT)));
            if let Some(p) = projects.first() {
                self.open_project(p);
            }
            self.import_paths(media, false);
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.has_unsaved() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        // Выбран либо клип, либо переход; путь рисуют только у выбранного клипа.
        if self.selected.is_some() {
            self.sel_transition = None;
        }
        if let Some(id) = self.sel_transition {
            if self.project.clip(id).map_or(true, |c| c.transition.is_none()) {
                self.sel_transition = None;
            }
        }
        if self.path_edit != self.selected {
            self.path_edit = None;
        }

        self.tick(ctx);
        self.shortcuts(ctx);

        let title = format!("{}{} — LumiCut", self.project.name, if self.has_unsaved() { "*" } else { "" });
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        self.top_bar(ctx);

        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(330.0)
            .height_range(230.0..=560.0)
            .show_separator_line(false)
            .frame(Frame::none().fill(BG).inner_margin(Margin::same(6.0)))
            .show(ctx, |ui| widgets::card(ui, |ui| self.timeline_ui(ui)));

        egui::SidePanel::left("library")
            .exact_width(414.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::none().fill(BG).inner_margin(Margin { left: 6.0, right: 0.0, top: 6.0, bottom: 0.0 }))
            .show(ctx, |ui| {
                widgets::card(ui, |ui| {
                    // Смена страницы: панель въезжает со своего края.
                    let page = self.page;
                    fx::appear(ui, egui::Id::new("page-left"), page, vec2(-18.0, 0.0), |ui| match page {
                        Page::Edit => self.library_ui(ui),
                        Page::Vfx => self.vfx_browser_ui(ui),
                    })
                })
            });

        egui::SidePanel::right("inspector")
            .exact_width(322.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::none().fill(BG).inner_margin(Margin { left: 0.0, right: 6.0, top: 6.0, bottom: 0.0 }))
            .show(ctx, |ui| {
                widgets::card(ui, |ui| {
                    let page = self.page;
                    fx::appear(ui, egui::Id::new("page-right"), page, vec2(18.0, 0.0), |ui| match page {
                        Page::Edit => self.inspector_ui(ui),
                        Page::Vfx => self.vfx_inspector_ui(ui),
                    })
                })
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(BG).inner_margin(Margin { left: 6.0, right: 6.0, top: 6.0, bottom: 0.0 }))
            .show(ctx, |ui| widgets::card(ui, |ui| self.player_ui(ui)));

        // Диалог выходит на первый план: остальное притемняется.
        let modal = self.export.open || self.pending.is_some();
        let dim = ctx.animate_bool_with_time(egui::Id::new("modal-dim"), modal, 0.18);
        if dim > 0.0 {
            let layer = egui::LayerId::new(egui::Order::PanelResizeLine, egui::Id::new("modal-dim"));
            ctx.layer_painter(layer).rect_filled(ctx.screen_rect(), 0.0, Color32::from_black_alpha((120.0 * dim) as u8));
        }

        self.export_window(ctx);
        self.confirm_window(ctx);
        self.shortcuts_window(ctx);
        self.drag_ghost(ctx);
        self.toast_ui(ctx);
        fx::paint(ctx);

        if self.preview.pending() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}
