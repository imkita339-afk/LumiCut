//! Векторные иконки: контуры в координатах 24×24, рисуются линиями.
#![allow(dead_code)]

use eframe::egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

pub const FILM: &str = "M4 4h16v16H4zM8 4v16M16 4v16M4 9h4M4 15h4M16 9h4M16 15h4";
pub const NOTE: &str = "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z";
pub const TEXT: &str = "M5 7V4h14v3M12 4v16M9 20h6";
pub const STICKER: &str = "M4 4h16v10l-6 6H4zM20 14h-6v6";
pub const SPARK: &str = "M11 4l2 6 6 2-6 2-2 6-2-6-6-2 6-2zM19 3v4M17 5h4";
pub const BOW: &str = "M4 5l8 7-8 7zM20 5l-8 7 8 7z";
pub const RINGS: &str = "M9 14a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM15 14a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 20a5 5 0 1 0 0-10 5 5 0 0 0 0 10z";
pub const SUBS: &str = "M3 5h18v14H3zM7 14h4M13 14h4M7 10h10";
pub const PLAY: &str = "M8 5l12 7-12 7z";
pub const PAUSE: &str = "M7 5h3v14H7zM14 5h3v14h-3z";
pub const PREV_FRAME: &str = "M18 5l-9 7 9 7zM6 5v14";
pub const NEXT_FRAME: &str = "M6 5l9 7-9 7zM18 5v14";
pub const PLUS: &str = "M12 5v14M5 12h14";
pub const SEARCH: &str = "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4";
pub const EYE: &str = "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z";
pub const EYE_OFF: &str = "M3 3l18 18M10.6 6.2A9.6 9.6 0 0 1 12 6c6 0 10 6 10 6a17 17 0 0 1-3 3.6M6.5 7.6A16 16 0 0 0 2 12s4 6 10 6a9.7 9.7 0 0 0 4-.9";
pub const VOLUME: &str = "M4 9v6h4l5 4V5L8 9zM16 9a4 4 0 0 1 0 6M18.5 6.5a8 8 0 0 1 0 11";
pub const VOLUME_OFF: &str = "M4 9v6h4l5 4V5L8 9zM17 9l5 6M22 9l-5 6";
pub const LOCK: &str = "M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3";
pub const UNLOCK: &str = "M6 11h12v9H6zM8 11V8a4 4 0 0 1 7.5-2";
pub const CURSOR: &str = "M5 3l14 8-6 2-2 6z";
pub const BLADE: &str = "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM20 4L8.1 15.9M14.5 14.5L20 20M8.1 8.1L12 12";
pub const UNDO: &str = "M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3";
pub const REDO: &str = "M15 14l5-5-5-5M20 9H10a6 6 0 0 0 0 12h3";
pub const SPLIT: &str = "M12 3v18M8 7H4v10h4M16 7h4v10h-4";
pub const TRIM_LEFT: &str = "M15 4v16M19 8v8M11 12H3M6 9l-3 3 3 3";
pub const TRIM_RIGHT: &str = "M9 4v16M5 8v8M13 12h8M18 9l3 3-3 3";
pub const TRASH: &str = "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6";
pub const MAGNET: &str = "M6 4v8a6 6 0 0 0 12 0V4h-4v8a2 2 0 0 1-4 0V4zM6 8h4M14 8h4";
pub const ZOOM_OUT: &str = "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4M8 11h6";
pub const ZOOM_IN: &str = "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4M8 11h6M11 8v6";
pub const FIT: &str = "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5";
pub const EXPORT: &str = "M12 15V4M8 8l4-4 4 4M4 15v5h16v-5";
pub const RESET: &str = "M4 12a8 8 0 1 0 2.3-5.7M4 4v5h5";
pub const FOLDER: &str = "M3 6h6l2 3h10v10H3z";
pub const MIC: &str = "M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4";
pub const IMAGE: &str = "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15 9h.01";
pub const CHECK: &str = "M5 12.5l4.5 4.5L19 7.5";
pub const PATH: &str = "M5 21a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM19 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM7 19h6a4 4 0 0 0 0-8h-2a3 3 0 0 1 0-6h6";
pub const DICE: &str = "M5 4h14v16H5zM9 8h.01M15 8h.01M12 12h.01M9 16h.01M15 16h.01";
pub const CHEVRON: &str = "M6 9l6 6 6-6";
pub const KEY: &str = "M12 3l9 9-9 9-9-9z";
pub const COPY: &str = "M9 9h11v11H9zM5 15H4V4h11v1";
pub const PASTE: &str = "M9 3h6v3H9zM9 5H5v16h14V5h-4M9 12h6M9 16h6";
pub const WAND: &str = "M4 20L15 9M13 7l4 4M18 3v4M16 5h4M7 4v3M5.5 5.5h3M19 13v3M17.5 14.5h3";
pub const PIPETTE: &str = "M4 20l1-4 9-9 3 3-9 9zM13 6l2-2a2 2 0 0 1 3 0l2 2a2 2 0 0 1 0 3l-2 2z";
pub const MATTE: &str = "M3 5h18v14H3zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z";
pub const SHAPE: &str = "M12 5a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM3 3h4v4H3zM17 17h4v4h-4z";
pub const LAYERS: &str = "M12 4l9 5-9 5-9-5zM3 14l9 5 9-5";
pub const COMPARE: &str = "M3 5h18v14H3zM12 3v18M6 12h3M15 12h3";
pub const UP: &str = "M6 15l6-6 6 6";
pub const CLOSE: &str = "M6 6l12 12M18 6L6 18";

enum Tok {
    Cmd(char),
    Num(f32),
}

fn tokenize(d: &str) -> Vec<Tok> {
    let b = d.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let s = i;
            let mut dot = c == '.';
            i += 1;
            while i < b.len() {
                let c2 = b[i] as char;
                if c2.is_ascii_digit() {
                    i += 1;
                } else if c2 == '.' && !dot {
                    dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Num(d[s..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

fn angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
}

#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Pos2>, p0: Pos2, rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, p1: Pos2) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-4 || ry < 1e-4 || p0 == p1 {
        out.push(p1);
        return;
    }
    let (sinp, cosp) = rot.to_radians().sin_cos();
    let dx2 = (p0.x - p1.x) / 2.0;
    let dy2 = (p0.y - p1.y) / 2.0;
    let x1p = cosp * dx2 + sinp * dy2;
    let y1p = -sinp * dx2 + cosp * dy2;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den.abs() < 1e-9 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cosp * cxp - sinp * cyp + (p0.x + p1.x) / 2.0;
    let cy = sinp * cxp + cosp * cyp + (p0.y + p1.y) / 2.0;
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let n = ((delta.abs() / (std::f32::consts::PI / 10.0)).ceil() as usize).max(2);
    for i in 1..=n {
        let t = theta + delta * i as f32 / n as f32;
        let (st, ct) = t.sin_cos();
        out.push(pos2(
            cosp * rx * ct - sinp * ry * st + cx,
            sinp * rx * ct + cosp * ry * st + cy,
        ));
    }
}

fn cubic(out: &mut Vec<Pos2>, p0: Pos2, c1: Pos2, c2: Pos2, p1: Pos2) {
    const N: usize = 12;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let u = 1.0 - t;
        let a = u * u * u;
        let b = 3.0 * u * u * t;
        let c = 3.0 * u * t * t;
        let d = t * t * t;
        out.push(pos2(
            a * p0.x + b * c1.x + c * c2.x + d * p1.x,
            a * p0.y + b * c1.y + c * c2.y + d * p1.y,
        ));
    }
}

/// Разбирает SVG-путь в набор ломаных (точки, замкнута ли).
pub fn flatten(d: &str) -> Vec<(Vec<Pos2>, bool)> {
    let toks = tokenize(d);
    let mut out: Vec<(Vec<Pos2>, bool)> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    let mut p = pos2(0.0, 0.0);
    let mut start = p;
    let mut last_ctrl: Option<Pos2> = None;
    let mut cmd = 'M';
    let mut i = 0;

    let num = |i: &mut usize| -> Option<f32> {
        match toks.get(*i) {
            Some(Tok::Num(n)) => {
                *i += 1;
                Some(*n)
            }
            _ => None,
        }
    };

    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            cmd = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), true));
                }
                p = start;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let upper = cmd.to_ascii_uppercase();
        let to = |x: f32, y: f32, p: Pos2| if rel { pos2(p.x + x, p.y + y) } else { pos2(x, y) };
        if upper != 'M' && cur.is_empty() {
            cur.push(p);
        }
        match upper {
            'M' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
                p = to(x, y, p);
                start = p;
                cur.push(p);
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                p = to(x, y, p);
                cur.push(p);
            }
            'H' => {
                let Some(x) = num(&mut i) else { break };
                p.x = if rel { p.x + x } else { x };
                cur.push(p);
            }
            'V' => {
                let Some(y) = num(&mut i) else { break };
                p.y = if rel { p.y + y } else { y };
                cur.push(p);
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep), Some(x), Some(y)) = (
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                ) else {
                    break;
                };
                let end = to(x, y, p);
                arc(&mut cur, p, rx, ry, rot, large != 0.0, sweep != 0.0, end);
                p = end;
            }
            'C' => {
                let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                ) else {
                    break;
                };
                let c1 = to(x1, y1, p);
                let c2 = to(x2, y2, p);
                let end = to(x, y, p);
                cubic(&mut cur, p, c1, c2, end);
                last_ctrl = Some(c2);
                p = end;
            }
            'S' => {
                let (Some(x2), Some(y2), Some(x), Some(y)) =
                    (num(&mut i), num(&mut i), num(&mut i), num(&mut i))
                else {
                    break;
                };
                let c1 = match last_ctrl {
                    Some(c) => pos2(2.0 * p.x - c.x, 2.0 * p.y - c.y),
                    None => p,
                };
                let c2 = to(x2, y2, p);
                let end = to(x, y, p);
                cubic(&mut cur, p, c1, c2, end);
                last_ctrl = Some(c2);
                p = end;
            }
            _ => {
                i += 1;
            }
        }
        if upper != 'C' && upper != 'S' {
            last_ctrl = None;
        }
    }
    if !cur.is_empty() {
        out.push((cur, false));
    }
    out
}

fn paint_impl(p: &Painter, rect: Rect, d: &str, color: Color32, width: f32, fill: bool) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * s, 12.0 * s);
    for (pts, closed) in flatten(d) {
        let pts: Vec<Pos2> = pts.iter().map(|q| pos2(o.x + q.x * s, o.y + q.y * s)).collect();
        if pts.len() == 2 && (pts[0] - pts[1]).length() < 0.5 {
            p.circle_filled(pts[0], width * 0.9, color);
        } else if fill && closed {
            p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        } else if closed {
            p.add(Shape::closed_line(pts, Stroke::new(width, color)));
        } else {
            p.add(Shape::line(pts, Stroke::new(width, color)));
        }
    }
}

/// Контурная иконка, вписанная в `rect`.
pub fn paint(p: &Painter, rect: Rect, d: &str, color: Color32) {
    paint_impl(p, rect, d, color, 1.4, false);
}

/// Залитая иконка (только для выпуклых контуров: треугольник, ромб, прямоугольник).
pub fn paint_filled(p: &Painter, rect: Rect, d: &str, color: Color32) {
    paint_impl(p, rect, d, color, 1.4, true);
}
