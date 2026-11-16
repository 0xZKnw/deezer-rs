//! Couleurs dominantes des pochettes : teinte de l'aurora et accent de l'interface.
//! Échantillonnage d'environ 900 pixels par pochette au chargement : coût négligeable.

use eframe::egui::{self, Color32};
use std::time::Instant;

pub type Pal = [Color32; 3];

// ------------------------------------------------------------------ palette

/// Palette par défaut (aucune pochette) : violet Deezer, rose, indigo.
pub const DEFAULT: Pal = [Color32::from_rgb(150, 60, 255), Color32::from_rgb(255, 70, 150), Color32::from_rgb(70, 60, 200)];

fn hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d < 1e-5 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    (h, if mx > 0.0 { d / mx } else { 0.0 }, mx)
}

fn from_hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Extrait 3 couleurs vives d'une pochette (échantillonnage ~900 px, coût négligeable).
pub fn extract(img: &egui::ColorImage) -> Pal {
    let [w, h] = img.size;
    if w == 0 || h == 0 {
        return DEFAULT;
    }
    let step = (((w * h) as f32 / 900.0).sqrt() as usize).max(1);
    // 12 teintes : (poids, somme r, g, b, somme v)
    let mut bins = [(0f32, 0f32, 0f32, 0f32); 12];
    let mut total = 0f32;
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let p = img.pixels[y * w + x];
            let c = [p.r() as f32 / 255.0, p.g() as f32 / 255.0, p.b() as f32 / 255.0];
            let (hh, s, v) = hsv(c);
            let wt = s * s * v * (0.35 + v);
            total += 1.0;
            let b = &mut bins[((hh * 12.0) as usize).min(11)];
            b.0 += wt;
            b.1 += c[0] * wt;
            b.2 += c[1] * wt;
            b.3 += c[2] * wt;
        }
    }
    let mut order: Vec<usize> = (0..12).collect();
    order.sort_by(|a, b| bins[*b].0.total_cmp(&bins[*a].0));
    let b1 = order[0];
    // pochette quasi monochrome => palette par défaut teintée
    if bins[b1].0 < total * 0.012 {
        return DEFAULT;
    }
    let col = |i: usize| {
        let b = bins[i];
        hsv([b.1 / b.0, b.2 / b.0, b.3 / b.0])
    };
    let norm = |(h, s, v): (f32, f32, f32), s0: f32, s1: f32, v0: f32, v1: f32| from_hsv(h, s.clamp(s0, s1), v.clamp(v0, v1));
    let (h1, s1, v1) = col(b1);
    let second = order[1..].iter().copied().find(|&i| {
        let d = (i as i32 - b1 as i32).rem_euclid(12).min((b1 as i32 - i as i32).rem_euclid(12));
        d >= 2 && bins[i].0 > bins[b1].0 * 0.12
    });
    let c1 = norm((h1, s1, v1), 0.55, 0.9, 0.75, 0.95);
    let c2 = match second {
        Some(i) => norm(col(i), 0.5, 0.9, 0.7, 0.95),
        None => from_hsv(h1 + 0.08, s1.clamp(0.5, 0.85), v1.clamp(0.7, 0.92)),
    };
    let c3 = from_hsv(h1 - 0.06, s1.clamp(0.45, 0.8), v1.clamp(0.45, 0.6));
    [c1, c2, c3]
}

/// Teinte (degrés) d'une couleur.
pub fn hue_deg(c: Color32) -> f32 {
    hsv([c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]).0 * 360.0
}

pub fn lerp_pal(a: &Pal, b: &Pal, t: f32) -> Pal {
    [a[0].lerp_to_gamma(b[0], t), a[1].lerp_to_gamma(b[1], t), a[2].lerp_to_gamma(b[2], t)]
}

/// Couleur d'accent lisible (texte blanc dessus) dérivée d'une couleur de palette.
pub fn accent(c: Color32) -> Color32 {
    let (h, s, v) = hsv([c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]);
    from_hsv(h, s.clamp(0.5, 0.82), v.clamp(0.78, 0.92))
}

/// Palette courante, en fondu doux d'un morceau à l'autre.
pub struct State {
    from: Pal,
    to: Pal,
    t0: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self { from: DEFAULT, to: DEFAULT, t0: Instant::now() }
    }
}

impl State {
    pub fn target(&mut self, p: Pal) {
        if p != self.to {
            self.from = self.current();
            self.to = p;
            self.t0 = Instant::now();
        }
    }

    pub fn fading(&self) -> bool {
        self.t0.elapsed().as_secs_f32() < 1.4
    }

    pub fn current(&self) -> Pal {
        let k = (self.t0.elapsed().as_secs_f32() / 1.4).min(1.0);
        lerp_pal(&self.from, &self.to, k * k * (3.0 - 2.0 * k))
    }
}
