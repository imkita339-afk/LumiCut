//! Ключевые кадры: значения свойств клипа, закреплённые в моментах времени.

use serde::{Deserialize, Serialize};

/// Свойство клипа, значение которого можно закрепить.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Prop {
    Scale,
    PosX,
    PosY,
    Rotation,
    Opacity,
    Volume,
}

impl Prop {
    /// Свойства, которые возвращает к исходным сброс трансформации.
    pub const TRANSFORM: [Prop; 5] = [Prop::Scale, Prop::PosX, Prop::PosY, Prop::Rotation, Prop::Opacity];
}

/// Закреплённое значение.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Момент от начала клипа, секунды таймлайна.
    pub t: f64,
    pub v: f32,
}

/// Точки одного свойства по возрастанию времени. Между точками значение меняется равномерно
/// или с плавным разгоном и торможением, до первой и после последней стоит на месте.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Curve(pub Vec<Key>);

impl Curve {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Значение в момент `t`; без точек его нет. `smooth` — плавный разгон и торможение между точками.
    pub fn at(&self, t: f64, smooth: bool) -> Option<f32> {
        let (first, last) = (self.0.first()?, self.0.last()?);
        if t <= first.t {
            return Some(first.v);
        }
        if t >= last.t {
            return Some(last.v);
        }
        let i = self.0.partition_point(|k| k.t <= t);
        let (a, b) = (self.0[i - 1], self.0[i]);
        let mut k = ((t - a.t) / (b.t - a.t)) as f32;
        if smooth {
            k = k * k * (3.0 - 2.0 * k);
        }
        Some(a.v + (b.v - a.v) * k)
    }

    /// Номер точки, стоящей в момент `t` с точностью `tol`.
    pub fn find(&self, t: f64, tol: f64) -> Option<usize> {
        self.0.iter().position(|k| (k.t - t).abs() <= tol)
    }

    /// Меняет точку в момент `t` или ставит новую.
    pub fn set(&mut self, t: f64, v: f32, tol: f64) {
        match self.find(t, tol) {
            Some(i) => self.0[i].v = v,
            None => {
                let i = self.0.partition_point(|k| k.t < t);
                self.0.insert(i, Key { t, v });
            }
        }
    }
}

/// Закреплённые значения свойств клипа. У свойства без точек действует обычное значение клипа.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Keys {
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub scale: Curve,
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub pos_x: Curve,
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub pos_y: Curve,
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub rotation: Curve,
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub opacity: Curve,
    #[serde(default, skip_serializing_if = "Curve::is_empty")]
    pub volume: Curve,
    /// Свойства, которые между точками идут с плавным разгоном и торможением.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eased: Vec<Prop>,
}

impl Keys {
    pub fn of(&self, prop: Prop) -> &Curve {
        match prop {
            Prop::Scale => &self.scale,
            Prop::PosX => &self.pos_x,
            Prop::PosY => &self.pos_y,
            Prop::Rotation => &self.rotation,
            Prop::Opacity => &self.opacity,
            Prop::Volume => &self.volume,
        }
    }

    pub fn of_mut(&mut self, prop: Prop) -> &mut Curve {
        match prop {
            Prop::Scale => &mut self.scale,
            Prop::PosX => &mut self.pos_x,
            Prop::PosY => &mut self.pos_y,
            Prop::Rotation => &mut self.rotation,
            Prop::Opacity => &mut self.opacity,
            Prop::Volume => &mut self.volume,
        }
    }

    fn curves(&self) -> [&Curve; 6] {
        [&self.scale, &self.pos_x, &self.pos_y, &self.rotation, &self.opacity, &self.volume]
    }

    fn curves_mut(&mut self) -> [&mut Curve; 6] {
        [&mut self.scale, &mut self.pos_x, &mut self.pos_y, &mut self.rotation, &mut self.opacity, &mut self.volume]
    }

    pub fn is_empty(&self) -> bool {
        self.curves().iter().all(|c| c.is_empty())
    }

    pub fn is_eased(&self, prop: Prop) -> bool {
        self.eased.contains(&prop)
    }

    pub fn set_eased(&mut self, prop: Prop, on: bool) {
        self.eased.retain(|p| *p != prop);
        if on {
            self.eased.push(prop);
        }
    }

    /// Моменты всех точек по возрастанию; точки разных свойств в одном моменте идут подряд.
    pub fn times(&self) -> Vec<f64> {
        let mut out: Vec<f64> = self.curves().iter().flat_map(|c| c.0.iter().map(|k| k.t)).collect();
        out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Сдвигает все точки по времени: начало клипа ушло, а точки остаются на своих местах.
    pub fn shift(&mut self, dt: f64) {
        for curve in self.curves_mut() {
            curve.0.iter_mut().for_each(|k| k.t += dt);
        }
    }

    /// Растягивает точки по времени вместе с клипом.
    pub fn stretch(&mut self, k: f64) {
        for curve in self.curves_mut() {
            curve.0.iter_mut().for_each(|key| key.t *= k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(points: &[(f64, f32)]) -> Curve {
        Curve(points.iter().map(|&(t, v)| Key { t, v }).collect())
    }

    #[test]
    fn value_changes_evenly_between_points_and_holds_outside() {
        let c = curve(&[(1.0, 0.0), (3.0, 1.0), (4.0, 0.5)]);
        assert_eq!(c.at(0.0, false), Some(0.0));
        assert_eq!(c.at(2.0, false), Some(0.5));
        assert_eq!(c.at(3.0, false), Some(1.0));
        assert_eq!(c.at(3.5, false), Some(0.75));
        assert_eq!(c.at(9.0, false), Some(0.5));
        assert_eq!(Curve::default().at(1.0, false), None);
        // Плавный ход: те же значения в точках и посередине, но разгон и торможение у краёв.
        assert_eq!(c.at(2.0, true), Some(0.5));
        assert_eq!(c.at(3.0, true), Some(1.0));
        assert!(c.at(1.5, true).unwrap() < 0.2 && c.at(2.5, true).unwrap() > 0.8);
    }

    #[test]
    fn set_edits_the_point_under_the_cursor_or_adds_one_in_order() {
        let mut c = curve(&[(1.0, 0.0), (3.0, 1.0)]);
        c.set(3.004, 0.8, 0.01);
        assert_eq!(c, curve(&[(1.0, 0.0), (3.0, 0.8)]));
        c.set(2.0, 0.2, 0.01);
        c.set(0.0, 0.1, 0.01);
        assert_eq!(c, curve(&[(0.0, 0.1), (1.0, 0.0), (2.0, 0.2), (3.0, 0.8)]));
    }

    #[test]
    fn empty_curves_are_not_saved() {
        let mut keys = Keys::default();
        assert_eq!(serde_json::to_string(&keys).unwrap(), "{}");
        keys.opacity = curve(&[(0.5, 0.2)]);
        let json = serde_json::to_string(&keys).unwrap();
        assert_eq!(json, r#"{"opacity":[{"t":0.5,"v":0.2}]}"#);
        assert_eq!(serde_json::from_str::<Keys>(&json).unwrap(), keys);
    }
}
