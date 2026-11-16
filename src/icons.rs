//! Icônes vectorielles dessinées avec le painter egui (aucune police/image => 0 RAM de plus).

use eframe::egui::{
    self, epaint::PathShape, pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    Ui,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ic {
    Play,
    Pause,
    Prev,
    Next,
    Shuffle,
    Repeat,
    RepeatOne,
    Heart,
    HeartOutline,
    Plus,
    Search,
    Volume,
    Mute,
    Queue,
    Dots,
    Left,
    Right,
    Home,
    Explore,
    Grid,
    Disc,
    Person,
    Mic,
    Download,
    Link,
    Close,
    Sidebar,
}

fn poly(p: &Painter, pts: &[(f32, f32)], c: Pos2, u: f32, col: Color32) {
    let pts: Vec<Pos2> = pts.iter().map(|(x, y)| pos2(c.x + x * u, c.y + y * u)).collect();
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}

fn line(p: &Painter, pts: &[(f32, f32)], c: Pos2, u: f32, st: Stroke) {
    let pts: Vec<Pos2> = pts.iter().map(|(x, y)| pos2(c.x + x * u, c.y + y * u)).collect();
    p.add(PathShape::line(pts, st));
}

fn arc(p: &Painter, c: Pos2, r: f32, a0: f32, a1: f32, st: Stroke) {
    let n = 14;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    p.add(PathShape::line(pts, st));
}

/// Cœur « propre » : carré tourné à 45° + deux demi-cercles sur ses côtés hauts
/// (tangence parfaite, encoche nette), pointe légèrement arrondie. Largeur ≈ 0.92·size.
fn heart_pts(c: Pos2, size: f32) -> Vec<Pos2> {
    use std::f32::consts::{FRAC_1_SQRT_2 as R2, PI};
    let s = size * 0.535;
    let b = c.y + 0.80 * s; // pointe (y vers le bas)
    let r = 0.5 * s;
    let cyc = b - 3.0 * s * R2 * 0.5; // centre des lobes
    let dx = 0.5 * s * R2;
    let mut pts = Vec::with_capacity(64);
    // pointe arrondie : arc de 135° -> 45° autour de (0, b - rt·√2)
    let rt = 0.11 * s;
    let tc = pos2(c.x, b - rt * std::f32::consts::SQRT_2);
    for k in 0..=5 {
        let a = (135.0 - 90.0 * k as f32 / 5.0f32).to_radians();
        pts.push(pos2(tc.x + rt * a.cos(), tc.y + rt * a.sin()));
    }
    // lobe droit : 45° -> -135°
    let n = 24;
    for k in 0..=n {
        let a = PI / 4.0 - PI * k as f32 / n as f32;
        pts.push(pos2(c.x + dx + r * a.cos(), cyc + r * a.sin()));
    }
    // lobe gauche : -45° -> -225° (sans répéter l'encoche)
    for k in 1..=n {
        let a = -PI / 4.0 - PI * k as f32 / n as f32;
        pts.push(pos2(c.x - dx + r * a.cos(), cyc + r * a.sin()));
    }
    pts
}

/// Remplissage d'un polygone étoilé autour de `ctr`, avec frange anticrénelée d'1 px physique.
fn fill_aa(p: &Painter, pts: &[Pos2], ctr: Pos2, col: Color32) {
    let f = 1.0 / p.pixels_per_point();
    let n = pts.len();
    let area: f32 = (0..n).map(|i| { let (a, b) = (pts[i], pts[(i + 1) % n]); a.x * b.y - b.x * a.y }).sum();
    let sg = if area > 0.0 { 1.0 } else { -1.0 };
    let mut m = egui::Mesh::default();
    m.colored_vertex(ctr, col);
    for i in 0..n {
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (e1, e2) = ((b - a).normalized(), (c - b).normalized());
        let (n1, n2) = (vec2(e1.y, -e1.x) * sg, vec2(e2.y, -e2.x) * sg);
        let s = n1 + n2;
        let nn = if s.length() > 1e-4 { s.normalized() } else { n1 };
        let off = nn * (0.5 * f / nn.dot(n1).max(0.5));
        m.colored_vertex(b - off, col);
        m.colored_vertex(b + off, Color32::TRANSPARENT);
    }
    let n = n as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        let (ii, io, ji, jo) = (1 + 2 * i, 2 + 2 * i, 1 + 2 * j, 2 + 2 * j);
        m.add_triangle(0, ii, ji);
        m.add_triangle(ii, io, ji);
        m.add_triangle(io, jo, ji);
    }
    p.add(Shape::mesh(m));
}

/// Dessine l'icône centrée en `c`, de taille `size` (px).
pub fn draw(p: &Painter, ic: Ic, c: Pos2, size: f32, col: Color32) {
    let u = size / 2.0; // unité : 1.0 = demi-côté
    let st = Stroke::new((size * 0.095).max(1.3), col);
    match ic {
        Ic::Play => poly(p, &[(-0.55, -0.8), (0.85, 0.0), (-0.55, 0.8)], c, u, col),
        Ic::Pause => {
            for dx in [-0.5f32, 0.5] {
                p.rect_filled(
                    Rect::from_center_size(pos2(c.x + dx * u, c.y), vec2(0.5 * u, 1.6 * u)),
                    CornerRadius::same(1),
                    col,
                );
            }
        }
        Ic::Prev => {
            poly(p, &[(0.8, -0.75), (-0.5, 0.0), (0.8, 0.75)], c, u, col);
            p.rect_filled(Rect::from_center_size(pos2(c.x - 0.75 * u, c.y), vec2(0.28 * u, 1.5 * u)), CornerRadius::ZERO, col);
        }
        Ic::Next => {
            poly(p, &[(-0.8, -0.75), (0.5, 0.0), (-0.8, 0.75)], c, u, col);
            p.rect_filled(Rect::from_center_size(pos2(c.x + 0.75 * u, c.y), vec2(0.28 * u, 1.5 * u)), CornerRadius::ZERO, col);
        }
        Ic::Shuffle => {
            line(p, &[(-0.9, -0.5), (-0.35, -0.5), (0.35, 0.5), (0.6, 0.5)], c, u, st);
            line(p, &[(-0.9, 0.5), (-0.35, 0.5), (0.35, -0.5), (0.6, -0.5)], c, u, st);
            poly(p, &[(0.55, -0.85), (0.95, -0.5), (0.55, -0.15)], c, u, col);
            poly(p, &[(0.55, 0.15), (0.95, 0.5), (0.55, 0.85)], c, u, col);
        }
        Ic::Repeat | Ic::RepeatOne => {
            line(p, &[(-0.9, 0.1), (-0.9, -0.5), (0.45, -0.5)], c, u, st);
            line(p, &[(0.9, -0.1), (0.9, 0.5), (-0.45, 0.5)], c, u, st);
            poly(p, &[(0.35, -0.9), (0.85, -0.5), (0.35, -0.1)], c, u, col);
            poly(p, &[(-0.35, 0.1), (-0.85, 0.5), (-0.35, 0.9)], c, u, col);
            if ic == Ic::RepeatOne {
                p.text(c, egui::Align2::CENTER_CENTER, "1", egui::FontId::proportional(size * 0.42), col);
            }
        }
        Ic::Heart => {
            let s = size * 0.535;
            // point intérieur aux deux lobes et au carré => polygone étoilé
            let ctr = pos2(c.x, c.y + 0.80 * s - 1.06 * s);
            fill_aa(p, &heart_pts(c, size), ctr, col);
        }
        Ic::HeartOutline => {
            let w = (size * 0.085).max(1.3);
            p.add(PathShape::closed_line(heart_pts(c, size - w * 1.1), Stroke::new(w, col)));
        }
        Ic::Plus => {
            line(p, &[(-0.7, 0.0), (0.7, 0.0)], c, u, st);
            line(p, &[(0.0, -0.7), (0.0, 0.7)], c, u, st);
        }
        Ic::Search => {
            p.circle_stroke(pos2(c.x - 0.12 * u, c.y - 0.12 * u), 0.62 * u, st);
            line(p, &[(0.33, 0.33), (0.85, 0.85)], c, u, st);
        }
        Ic::Volume | Ic::Mute => {
            poly(p, &[(-0.9, -0.3), (-0.45, -0.3), (0.0, -0.75), (0.0, 0.75), (-0.45, 0.3), (-0.9, 0.3)], c, u, col);
            if ic == Ic::Volume {
                arc(p, pos2(c.x - 0.05 * u, c.y), 0.55 * u, -0.9, 0.9, st);
                arc(p, pos2(c.x - 0.05 * u, c.y), 0.95 * u, -0.9, 0.9, st);
            } else {
                line(p, &[(0.25, -0.35), (0.85, 0.35)], c, u, st);
                line(p, &[(0.25, 0.35), (0.85, -0.35)], c, u, st);
            }
        }
        Ic::Queue => {
            for (i, y) in [-0.6f32, 0.0, 0.6].iter().enumerate() {
                let x1 = if i == 2 { 0.1 } else { 0.9 };
                line(p, &[(-0.9, *y), (x1, *y)], c, u, st);
            }
            poly(p, &[(0.35, 0.25), (0.95, 0.6), (0.35, 0.95)], c, u, col);
        }
        Ic::Dots => {
            for dx in [-0.6f32, 0.0, 0.6] {
                p.circle_filled(pos2(c.x + dx * u, c.y), size * 0.075, col);
            }
        }
        Ic::Left => line(p, &[(0.35, -0.7), (-0.35, 0.0), (0.35, 0.7)], c, u, st),
        Ic::Right => line(p, &[(-0.35, -0.7), (0.35, 0.0), (-0.35, 0.7)], c, u, st),
        Ic::Home => {
            line(p, &[(-0.9, -0.05), (0.0, -0.85), (0.9, -0.05)], c, u, st);
            line(p, &[(-0.65, -0.1), (-0.65, 0.8), (0.65, 0.8), (0.65, -0.1)], c, u, st);
            line(p, &[(-0.15, 0.8), (-0.15, 0.25), (0.15, 0.25), (0.15, 0.8)], c, u, st);
        }
        Ic::Explore => {
            p.circle_stroke(c, 0.85 * u, st);
            poly(p, &[(0.45, -0.45), (0.12, 0.12), (-0.45, 0.45), (-0.12, -0.12)], c, u, col);
        }
        Ic::Grid => {
            p.rect_stroke(Rect::from_center_size(c, vec2(1.7 * u, 1.7 * u)), CornerRadius::same(2), st, egui::StrokeKind::Middle);
            line(p, &[(0.0, -0.85), (0.0, 0.85)], c, u, st);
            line(p, &[(-0.85, 0.0), (0.85, 0.0)], c, u, st);
        }
        Ic::Disc => {
            p.circle_stroke(c, 0.85 * u, st);
            p.circle_filled(c, 0.18 * u, col);
        }
        Ic::Person => {
            p.circle_stroke(pos2(c.x, c.y - 0.4 * u), 0.42 * u, st);
            arc(p, pos2(c.x, c.y + 1.0 * u), 0.85 * u, std::f32::consts::PI * 1.12, std::f32::consts::PI * 1.88, st);
        }
        Ic::Mic => {
            p.rect_stroke(
                Rect::from_center_size(pos2(c.x, c.y - 0.2 * u), vec2(0.7 * u, 1.2 * u)),
                CornerRadius::same(255),
                st,
                egui::StrokeKind::Middle,
            );
            arc(p, pos2(c.x, c.y - 0.1 * u), 0.7 * u, 0.1, std::f32::consts::PI - 0.1, st);
            line(p, &[(0.0, 0.6), (0.0, 0.9)], c, u, st);
        }
        Ic::Download => {
            p.circle_stroke(c, 0.85 * u, st);
            line(p, &[(0.0, -0.4), (0.0, 0.35)], c, u, st);
            line(p, &[(-0.3, 0.05), (0.0, 0.38), (0.3, 0.05)], c, u, st);
        }
        Ic::Link => {
            line(p, &[(-0.1, -0.75), (-0.8, -0.75), (-0.8, 0.8), (0.75, 0.8), (0.75, 0.1)], c, u, st);
            line(p, &[(0.1, -0.1), (0.85, -0.85)], c, u, st);
            line(p, &[(0.2, -0.85), (0.85, -0.85), (0.85, -0.2)], c, u, st);
        }
        Ic::Close => {
            line(p, &[(-0.6, -0.6), (0.6, 0.6)], c, u, st);
            line(p, &[(-0.6, 0.6), (0.6, -0.6)], c, u, st);
        }
        Ic::Sidebar => {
            p.rect_stroke(Rect::from_center_size(c, vec2(1.7 * u, 1.5 * u)), CornerRadius::same(3), st, egui::StrokeKind::Middle);
            line(p, &[(-0.35, -0.75), (-0.35, 0.75)], c, u, st);
        }
    }
}

/// Bouton icône avec halo au survol.
pub fn button(ui: &mut Ui, ic: Ic, size: f32, col: Color32, hover_col: Color32) -> Response {
    let pad = size * 0.55;
    let (rect, resp) = ui.allocate_exact_size(vec2(size + pad, size + pad), Sense::click());
    paint_button(ui, &resp, rect, ic, size, col, hover_col);
    resp
}

pub fn paint_button(ui: &Ui, resp: &Response, rect: Rect, ic: Ic, size: f32, col: Color32, hover_col: Color32) {
    let c = if resp.hovered() { hover_col } else { col };
    draw(ui.painter(), ic, rect.center(), size, c);
}
