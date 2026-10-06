//! Переходы на стыке двух соседних клипов.

use serde::{Deserialize, Serialize};

pub const DEFAULT_DURATION: f64 = 0.6;
pub const MIN_DURATION: f64 = 0.1;
pub const MAX_DURATION: f64 = 3.0;

/// Во сколько раз вырастает уходящий клип при «наезде» и с какого размера начинает входящий.
pub const ZOOM_OUT: f32 = 1.6;
pub const ZOOM_IN: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum TransitionKind {
    Dissolve,
    Fade,
    Flash,
    SlideLeft,
    SlideRight,
    SlideUp,
    SlideDown,
    Zoom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Полная длительность, секунды; середина перехода приходится на стык.
    pub duration: f64,
}

impl Transition {
    pub fn new(kind: TransitionKind) -> Self {
        Self { kind, duration: DEFAULT_DURATION }
    }
}

/// Что переход делает с клипом в данный момент.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tweak {
    pub alpha: f32,
    /// Сдвиг в пикселях проекта.
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    /// Белая вспышка поверх кадра, 0..1.
    pub flash: f32,
}

impl Default for Tweak {
    fn default() -> Self {
        Self { alpha: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, flash: 0.0 }
    }
}

impl Tweak {
    /// Действие двух переходов сразу: на входе и на выходе клипа.
    pub fn and(self, o: Tweak) -> Tweak {
        Tweak {
            alpha: self.alpha * o.alpha,
            dx: self.dx + o.dx,
            dy: self.dy + o.dy,
            scale: self.scale * o.scale,
            flash: self.flash.max(o.flash),
        }
    }
}

/// Плавный разгон и торможение.
pub fn smooth(p: f32) -> f32 {
    p * p * (3.0 - 2.0 * p)
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 8] = [
        TransitionKind::Dissolve,
        TransitionKind::Fade,
        TransitionKind::Flash,
        TransitionKind::SlideLeft,
        TransitionKind::SlideRight,
        TransitionKind::SlideUp,
        TransitionKind::SlideDown,
        TransitionKind::Zoom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Dissolve => "Растворение",
            TransitionKind::Fade => "Затухание",
            TransitionKind::Flash => "Вспышка",
            TransitionKind::SlideLeft => "Сдвиг влево",
            TransitionKind::SlideRight => "Сдвиг вправо",
            TransitionKind::SlideUp => "Сдвиг вверх",
            TransitionKind::SlideDown => "Сдвиг вниз",
            TransitionKind::Zoom => "Наезд",
        }
    }

    /// Видны ли оба клипа одновременно: тогда каждый замирает на крайнем кадре за своей границей.
    pub fn overlaps(self) -> bool {
        !matches!(self, TransitionKind::Fade | TransitionKind::Flash)
    }

    /// Куда уезжает уходящий клип при сдвиге.
    pub fn slide(self) -> Option<(f32, f32)> {
        match self {
            TransitionKind::SlideLeft => Some((-1.0, 0.0)),
            TransitionKind::SlideRight => Some((1.0, 0.0)),
            TransitionKind::SlideUp => Some((0.0, -1.0)),
            TransitionKind::SlideDown => Some((0.0, 1.0)),
            _ => None,
        }
    }

    /// Вид клипа при доле перехода `p` в кадре `w`×`h`: входящего или уходящего.
    pub fn tweak(self, incoming: bool, p: f32, w: f32, h: f32) -> Tweak {
        let p = p.clamp(0.0, 1.0);
        let e = smooth(p);
        let mut t = Tweak::default();
        match self {
            TransitionKind::Dissolve => {
                if incoming {
                    t.alpha = p;
                }
            }
            TransitionKind::Fade => {
                t.alpha = if incoming { 2.0 * p - 1.0 } else { 1.0 - 2.0 * p }.clamp(0.0, 1.0);
            }
            TransitionKind::Flash => {
                t.flash = if incoming { 2.0 - 2.0 * p } else { 2.0 * p }.clamp(0.0, 1.0);
            }
            TransitionKind::Zoom => {
                if incoming {
                    t.alpha = p;
                    t.scale = ZOOM_IN + (1.0 - ZOOM_IN) * e;
                } else {
                    t.scale = 1.0 + (ZOOM_OUT - 1.0) * e;
                }
            }
            _ => {
                let (x, y) = self.slide().unwrap_or((0.0, 0.0));
                let k = if incoming { e - 1.0 } else { e };
                t.dx = x * w * k;
                t.dy = y * h * k;
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_starts_with_old_clip_and_ends_with_new() {
        for kind in TransitionKind::ALL {
            let still = Tweak::default();
            assert_eq!(kind.tweak(false, 0.0, 1920.0, 1080.0), still, "{kind:?}: уходящий клип в начале");
            assert_eq!(kind.tweak(true, 1.0, 1920.0, 1080.0), still, "{kind:?}: входящий клип в конце");
        }
    }

    #[test]
    fn slide_moves_clips_together() {
        let out = TransitionKind::SlideLeft.tweak(false, 0.5, 1920.0, 1080.0);
        let inc = TransitionKind::SlideLeft.tweak(true, 0.5, 1920.0, 1080.0);
        assert_eq!(out.dx, -960.0);
        assert_eq!(inc.dx, 960.0);
    }
}
