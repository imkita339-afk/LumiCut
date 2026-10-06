//! Значок LumiCut. Рисуется формулами, чтобы значок окна, значок файла и логотип в шапке
//! получались из одного и того же кода при любом размере.

type P = (f32, f32);

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: P, b: P) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

/// Расстояние со знаком до скруглённого квадрата с центром в середине кадра.
fn rounded_square(p: P, half: f32, radius: f32) -> f32 {
    let q = ((p.0 - 0.5).abs() - half + radius, (p.1 - 0.5).abs() - half + radius);
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - radius
}

/// Расстояние со знаком до треугольника.
fn triangle(p: P, v: [P; 3]) -> f32 {
    let sign = {
        let (e0, e2) = (sub(v[1], v[0]), sub(v[0], v[2]));
        (e0.0 * e2.1 - e0.1 * e2.0).signum()
    };
    let mut best = (f32::MAX, f32::MAX);
    for i in 0..3 {
        let e = sub(v[(i + 1) % 3], v[i]);
        let w = sub(p, v[i]);
        let k = (dot(w, e) / dot(e, e)).clamp(0.0, 1.0);
        let d = sub(w, (e.0 * k, e.1 * k));
        best = (best.0.min(dot(d, d)), best.1.min(sign * (w.0 * e.1 - w.1 * e.0)));
    }
    -best.0.sqrt() * best.1.signum()
}

/// Цвет точки значка: RGB и непрозрачность.
fn sample(p: P) -> ([f32; 3], f32) {
    const INK: [f32; 3] = [23.0, 19.0, 10.0];
    const TOP: [f32; 3] = [255.0, 222.0, 118.0];
    const BOTTOM: [f32; 3] = [255.0, 176.0, 30.0];

    if rounded_square(p, 0.46, 0.21) > 0.0 {
        return ([0.0; 3], 0.0);
    }
    // Треугольник «воспроизведение», рассечённый наискось: половинки разведены в стороны.
    let right = |p: P| (p.0 - 0.5) + (p.1 - 0.5) * 0.36 > 0.0;
    let q = (p.0 + if right(p) { -0.03 } else { 0.03 }, p.1);
    if right(q) == right(p) && triangle(q, [(0.35, 0.26), (0.35, 0.74), (0.74, 0.5)]) < 0.04 {
        return (INK, 1.0);
    }
    let k = ((p.1 - 0.04) / 0.92).clamp(0.0, 1.0);
    ([0, 1, 2].map(|i| TOP[i] + (BOTTOM[i] - TOP[i]) * k), 1.0)
}

/// Значок `size`×`size`: RGBA с обычной (не умноженной) прозрачностью.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = (
                        (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32,
                        (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32,
                    );
                    let (c, a) = sample(p);
                    for i in 0..3 {
                        rgb[i] += c[i] * a;
                    }
                    alpha += a;
                }
            }
            for c in rgb {
                out.push(if alpha > 0.0 { (c / alpha).round() as u8 } else { 0 });
            }
            out.push((alpha / (SS * SS) as f32 * 255.0).round() as u8);
        }
    }
    out
}
