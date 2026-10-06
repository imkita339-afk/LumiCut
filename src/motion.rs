//! Движение клипа по пути.

use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

pub const MAX_LOOPS: u32 = 50;
/// На сколько отрезков делится сглаженный участок между двумя точками.
const SUBDIV: usize = 12;

pub fn full_span() -> [f32; 2] {
    [0.0, 1.0]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Точки пути в пикселях проекта, относительно положения клипа.
    pub points: Vec<[f32; 2]>,
    /// Последняя точка соединена с первой.
    pub closed: bool,
    /// Плавная кривая через точки вместо ломаной.
    pub smooth: bool,
    /// Плавный разгон и торможение.
    pub ease: bool,
    /// Сколько раз пройти путь за время клипа; незамкнутый путь проходится туда и обратно.
    pub loops: u32,
    /// Какая часть движения приходится на этот клип: меняется, когда клип разрезают.
    #[serde(default = "full_span")]
    pub span: [f32; 2],
}

impl Default for Motion {
    fn default() -> Self {
        Self { points: Vec::new(), closed: false, smooth: true, ease: false, loops: 1, span: full_span() }
    }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Точка кривой Катмулла — Рома между `p1` и `p2`.
fn catmull_rom(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    [0, 1].map(|i| {
        0.5 * (2.0 * p1[i]
            + (p2[i] - p0[i]) * t
            + (2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i]) * t * t
            + (3.0 * p1[i] - p0[i] - 3.0 * p2[i] + p3[i]) * t * t * t)
    })
}

impl Motion {
    /// Путь задан: по одной точке двигаться некуда.
    pub fn is_active(&self) -> bool {
        self.points.len() >= 2
    }

    /// Круг радиуса `r` вокруг положения клипа, начиная сверху по часовой стрелке.
    pub fn circle(r: f32) -> Self {
        let points = (0..8)
            .map(|i| {
                let a = TAU * i as f32 / 8.0 - TAU / 4.0;
                [r * a.cos(), r * a.sin()]
            })
            .collect();
        Self { points, closed: true, ..Self::default() }
    }

    /// Восьмёрка шириной `2r`.
    pub fn figure_eight(r: f32) -> Self {
        let points = (0..12)
            .map(|i| {
                let a = TAU * i as f32 / 12.0;
                [r * a.sin(), r * a.sin() * a.cos()]
            })
            .collect();
        Self { points, closed: true, ..Self::default() }
    }

    /// Ломаная, по которой идёт клип: сглаженный путь разбит на короткие отрезки.
    pub fn polyline(&self) -> Vec<[f32; 2]> {
        let n = self.points.len();
        if n < 2 {
            return self.points.clone();
        }
        let segments = if self.closed { n } else { n - 1 };
        // Сосед точки с учётом замыкания; у незамкнутого пути крайние точки повторяются.
        let at = |i: isize| -> [f32; 2] {
            if self.closed {
                self.points[i.rem_euclid(n as isize) as usize]
            } else {
                self.points[i.clamp(0, n as isize - 1) as usize]
            }
        };
        let mut line = vec![self.points[0]];
        for s in 0..segments as isize {
            if self.smooth && n > 2 {
                for k in 1..=SUBDIV {
                    line.push(catmull_rom(at(s - 1), at(s), at(s + 1), at(s + 2), k as f32 / SUBDIV as f32));
                }
            } else {
                line.push(at(s + 1));
            }
        }
        line
    }

    /// Смещение клипа, когда пройдена доля `tau` его длительности.
    pub fn at(&self, tau: f32) -> Option<[f32; 2]> {
        if !self.is_active() {
            return None;
        }
        let mut u = self.span[0] + (self.span[1] - self.span[0]) * tau.clamp(0.0, 1.0);
        if self.ease {
            u = u * u * (3.0 - 2.0 * u);
        }
        let v = u * self.loops.clamp(1, MAX_LOOPS) as f32;
        let (lap, part) = (v.floor(), v - v.floor());
        // Незамкнутый путь на каждом втором проходе идёт обратно.
        let along = if !self.closed && lap as u32 % 2 == 1 { 1.0 - part } else { part };

        let line = self.polyline();
        let total: f32 = line.windows(2).map(|w| dist(w[0], w[1])).sum();
        if total <= 0.0 {
            return Some(line[0]);
        }
        let mut left = along * total;
        for w in line.windows(2) {
            let len = dist(w[0], w[1]);
            if left <= len && len > 0.0 {
                let k = left / len;
                return Some([w[0][0] + (w[1][0] - w[0][0]) * k, w[0][1] + (w[1][1] - w[0][1]) * k]);
            }
            left -= len;
        }
        line.last().copied()
    }

    fn center(&self) -> [f32; 2] {
        let n = self.points.len().max(1) as f32;
        let sum = self.points.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        [sum[0] / n, sum[1] / n]
    }

    /// Размах пути: наибольшее удаление точек от их центра.
    pub fn radius(&self) -> f32 {
        let c = self.center();
        self.points.iter().map(|p| dist(*p, c)).fold(0.0, f32::max)
    }

    /// Растягивает путь от центра точек до размаха `radius`.
    pub fn resize(&mut self, radius: f32) {
        let (c, old) = (self.center(), self.radius());
        if old <= 0.0 {
            return;
        }
        let k = radius.max(1.0) / old;
        for p in &mut self.points {
            *p = [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k];
        }
    }

    /// Делит движение между двумя частями разрезанного клипа; `k` — доля левой части.
    pub fn split(&mut self, right: &mut Motion, k: f32) {
        let mid = self.span[0] + (self.span[1] - self.span[0]) * k;
        self.span[1] = mid;
        right.span[0] = mid;
    }

    /// Убирает лишние точки нарисованного от руки участка, начиная с точки `from`.
    pub fn simplify(&mut self, from: usize, tolerance: f32) {
        if self.points.len() < from + 3 {
            return;
        }
        let tail = self.points.split_off(from);
        let mut keep = vec![false; tail.len()];
        keep[0] = true;
        keep[tail.len() - 1] = true;
        let mut stack = vec![(0, tail.len() - 1)];
        while let Some((a, b)) = stack.pop() {
            let (pa, pb) = (tail[a], tail[b]);
            let len = dist(pa, pb).max(1e-6);
            let far = (a + 1..b)
                .map(|i| {
                    let cross = (pb[0] - pa[0]) * (tail[i][1] - pa[1]) - (pb[1] - pa[1]) * (tail[i][0] - pa[0]);
                    (i, cross.abs() / len)
                })
                .fold((a, 0.0f32), |best, cur| if cur.1 > best.1 { cur } else { best });
            if far.1 > tolerance {
                keep[far.0] = true;
                stack.push((a, far.0));
                stack.push((far.0, b));
            }
        }
        self.points.extend(tail.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Motion {
        Motion { points: vec![[0.0, 0.0], [100.0, 0.0]], smooth: false, ..Motion::default() }
    }

    #[test]
    fn straight_path_is_walked_evenly() {
        let m = line();
        assert_eq!(m.at(0.0), Some([0.0, 0.0]));
        assert_eq!(m.at(0.25), Some([25.0, 0.0]));
        assert_eq!(m.at(1.0), Some([100.0, 0.0]));
    }

    #[test]
    fn open_path_goes_back_and_forth() {
        let m = Motion { loops: 2, ..line() };
        assert_eq!(m.at(0.5), Some([100.0, 0.0]));
        assert_eq!(m.at(0.75), Some([50.0, 0.0]));
        assert_eq!(m.at(1.0), Some([0.0, 0.0]));
    }

    #[test]
    fn circle_returns_to_start_and_keeps_radius() {
        let m = Motion::circle(100.0);
        let (a, b) = (m.at(0.0).unwrap(), m.at(1.0).unwrap());
        assert!(dist(a, b) < 0.01);
        for i in 0..40 {
            let p = m.at(i as f32 / 40.0).unwrap();
            let r = dist(p, [0.0, 0.0]);
            assert!((r - 100.0).abs() < 4.0, "радиус {r}");
        }
    }

    #[test]
    fn split_halves_continue_each_other() {
        let mut left = line();
        let mut right = left.clone();
        left.split(&mut right, 0.4);
        assert_eq!(left.at(1.0), Some([40.0, 0.0]));
        assert_eq!(right.at(0.0), Some([40.0, 0.0]));
        assert_eq!(right.at(1.0), Some([100.0, 0.0]));
    }

    #[test]
    fn single_point_does_not_move_clip() {
        let m = Motion { points: vec![[5.0, 5.0]], ..Motion::default() };
        assert_eq!(m.at(0.5), None);
    }

    #[test]
    fn simplify_drops_points_on_a_straight_stroke() {
        let mut m = Motion { points: (0..=10).map(|i| [i as f32 * 10.0, 0.0]).collect(), ..Motion::default() };
        m.points.push([100.0, 50.0]);
        m.simplify(0, 2.0);
        assert_eq!(m.points, vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0]]);
    }
}
