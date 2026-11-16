//! Rendu egui — fond graphite, aurora ASCII (shader GPU) teintée par la pochette en cours,
//! verre discret, boutons primaires blancs, titres SF Pro gras, accent sobre tiré de la pochette
//! (morceau en cours, progression), lecteur en « dock » flottant.
//! Listes virtualisées, textures en LRU borné, repaint uniquement sur événement.

use crate::ambient::{self, Pal};
use crate::app::*;
use crate::icons::{self, Ic};
use crate::model::*;
use crate::workers::Pool;
use eframe::egui::{
    self, epaint::RectShape, epaint::Shadow, pos2, text::LayoutJob, text::TextWrapping, vec2, Align, Align2, Color32,
    Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Key, Layout, Margin, Mesh, Painter,
    Panel, Pos2, Rect, Response, RichText, ScrollArea, Sense, Shape, Stroke, StrokeKind, TextEdit, TextFormat,
    TextureHandle, TextureOptions, Ui,
};
use lru::LruCache;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicU32, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};

// ------------------------------------------------------------------ palette

/// toile principale : graphite froid
pub const BG: Color32 = Color32::from_rgb(13, 13, 17);
/// barre latérale
pub const SIDE: Color32 = Color32::from_rgb(9, 9, 12);
/// surfaces surélevées : menus, fenêtres, dock du lecteur
pub const RAISED: Color32 = Color32::from_rgb(25, 25, 32);
/// voile clair 3.5 %
pub const CARD: Color32 = Color32::from_rgba_premultiplied(9, 9, 10, 9);
/// survol 7 %
pub const HOVER: Color32 = Color32::from_rgba_premultiplied(18, 18, 20, 18);
/// pressé / piste de slider 12 %
pub const PRESS: Color32 = Color32::from_rgba_premultiplied(30, 30, 33, 30);
/// filet 8 %
pub const LINE: Color32 = Color32::from_rgba_premultiplied(20, 20, 23, 20);
pub const INK: Color32 = Color32::from_rgb(244, 244, 247);
pub const SECOND: Color32 = Color32::from_rgb(186, 186, 198);
pub const MUTED: Color32 = Color32::from_rgb(134, 134, 150);
pub const FAINT: Color32 = Color32::from_rgb(86, 86, 100);
/// rose fixe : coup de cœur, erreurs
pub const ACCENT: Color32 = Color32::from_rgb(255, 70, 140);
/// violet Deezer (illustration « Favourite tracks »)
pub const VIOLET: Color32 = Color32::from_rgb(150, 60, 255);

const IMG_BUDGET: usize = 28 << 20; // octets de textures conservés (LRU)

// accent dynamique (tiré de la pochette en cours), partagé par tous les composants
const fn pack(c: Color32) -> u32 {
    let [r, g, b, a] = c.to_array();
    u32::from_le_bytes([r, g, b, a])
}
static PAL: [AtomicU32; 5] = [
    AtomicU32::new(pack(ambient::DEFAULT[0])),
    AtomicU32::new(pack(ambient::DEFAULT[1])),
    AtomicU32::new(pack(ambient::DEFAULT[2])),
    AtomicU32::new(pack(Color32::from_rgb(160, 74, 255))),
    AtomicU32::new(pack(Color32::from_rgb(235, 72, 150))),
];
fn pal_get(i: usize) -> Color32 {
    let [r, g, b, a] = PAL[i].load(Ordering::Relaxed).to_le_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}
pub fn set_palette(p: &Pal) {
    for (i, c) in p.iter().enumerate() {
        PAL[i].store(pack(*c), Ordering::Relaxed);
    }
    PAL[3].store(pack(ambient::accent(p[0])), Ordering::Relaxed);
    PAL[4].store(pack(ambient::accent(p[1])), Ordering::Relaxed);
}
/// accent principal (lisible sous du texte blanc)
pub fn acc() -> Color32 {
    pal_get(3)
}
/// teinte claire de l'accent pour du texte sur fond sombre
fn acc_text() -> Color32 {
    acc().lerp_to_gamma(Color32::WHITE, 0.35)
}

// ------------------------------------------------------------------ style / polices

/// Police système projetée en mémoire (mmap) : pages partagées et propres, rien sur le tas.
fn map(path: &str) -> Option<&'static [u8]> {
    let f = std::fs::File::open(path).ok()?;
    let m = unsafe { memmap2::Mmap::map(&f) }.ok()?;
    Some(&**Box::leak(Box::new(m)))
}

fn face(bytes: &'static [u8], wght: f32) -> Arc<FontData> {
    let mut fd = FontData::from_static(bytes);
    fd.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", wght)]);
    Arc::new(fd)
}

pub fn setup_style(ctx: &Context) {
    let mut fd = FontDefinitions::default();
    let fallback = fd.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_fallback = fd.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let fam = |fd: &mut FontDefinitions, fam: FontFamily, name: &str, data: Option<Arc<FontData>>, fb: &Vec<String>| {
        let mut list = fb.clone();
        if let Some(d) = data {
            fd.font_data.insert(name.into(), d);
            list.insert(0, name.into());
        }
        fd.families.insert(fam, list);
    };
    // SF Pro (variable) : une seule projection du fichier, partagée par les quatre graisses
    let sf = map("/System/Library/Fonts/SFNS.ttf");
    let sfm = map("/System/Library/Fonts/SFNSMono.ttf");
    fam(&mut fd, FontFamily::Proportional, "sf", sf.map(|b| face(b, 400.0)), &fallback);
    fam(&mut fd, FontFamily::Name("med".into()), "sf-med", sf.map(|b| face(b, 560.0)), &fallback);
    fam(&mut fd, FontFamily::Name("bold".into()), "sf-bold", sf.map(|b| face(b, 680.0)), &fallback);
    fam(&mut fd, FontFamily::Name("display".into()), "sf-display", sf.map(|b| face(b, 720.0)), &fallback);
    fam(&mut fd, FontFamily::Monospace, "sf-mono", sfm.map(|b| face(b, 460.0)), &mono_fallback);
    ctx.set_fonts(fd);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = RAISED;
    v.extreme_bg_color = Color32::from_rgb(10, 10, 13);
    v.faint_bg_color = CARD;
    v.code_bg_color = CARD;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(18);
    v.window_shadow = Shadow { offset: [0, 20], blur: 50, spread: 0, color: Color32::from_black_alpha(170) };
    v.popup_shadow = Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(150) };
    v.menu_corner_radius = CornerRadius::same(12);
    v.selection.bg_fill = acc().gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, INK);
    v.hyperlink_color = INK;
    v.override_text_color = Some(INK);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, SECOND);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(8);
        w.fg_stroke = Stroke::new(1.0, INK);
        w.expansion = 0.0;
    }
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.inactive.bg_fill = PRESS;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.active.weak_bg_fill = PRESS;
    v.widgets.active.bg_fill = PRESS;
    v.widgets.active.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.open.weak_bg_fill = HOVER;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(20);
    });
}

fn ease(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

/// Animation booléenne lissée (0..1). Ne demande de repaint que pendant l'animation.
pub fn anim(ui: &Ui, id: egui::Id, on: bool, t: f32) -> f32 {
    ease(ui.ctx().animate_bool_with_time(id, on, t))
}

fn heart_pop(ui: &Ui, id: egui::Id, liked: bool) -> f32 {
    let k = ui.ctx().animate_bool_with_time(id, liked, 0.32);
    1.0 + 0.32 * (1.0 - (2.0 * k - 1.0).abs())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}
pub fn med(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("med".into()))
}
pub fn reg(size: f32) -> FontId {
    FontId::proportional(size)
}
/// titres : SF Pro très gras
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn galley_ls(p: &Painter, s: &str, font: FontId, col: Color32, max_w: f32, ls: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.append(s, 0.0, TextFormat { font_id: font, color: col, extra_letter_spacing: ls, ..Default::default() });
    job.wrap = TextWrapping { max_width: max_w.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

pub fn galley(p: &Painter, s: &str, font: FontId, col: Color32, max_w: f32) -> Arc<egui::Galley> {
    galley_ls(p, s, font, col, max_w, 0.0)
}

/// Texte tronqué (…) positionné par son coin haut-gauche. Retourne le rect occupé.
pub fn text(p: &Painter, pos: Pos2, s: &str, font: FontId, col: Color32, max_w: f32) -> Rect {
    text_ls(p, pos, s, font, col, max_w, 0.0)
}

pub fn text_ls(p: &Painter, pos: Pos2, s: &str, font: FontId, col: Color32, max_w: f32, ls: f32) -> Rect {
    let g = galley_ls(p, s, font, col, max_w, ls);
    let r = Rect::from_min_size(pos, g.size());
    p.galley(pos, g, col);
    r
}

pub fn text_center(p: &Painter, cx: f32, y: f32, s: &str, font: FontId, col: Color32, max_w: f32) {
    let g = galley(p, s, font, col, max_w);
    p.galley(pos2(cx - g.size().x / 2.0, y), g, col);
}

/// Titre en graisse « display », approche resserrée.
pub fn title_text(p: &Painter, pos: Pos2, s: &str, size: f32, col: Color32, max_w: f32) -> Rect {
    text_ls(p, pos, s, display(size), col, max_w, -size * 0.022)
}

/// Petit intitulé en capitales espacées (« LIBRARY », en-têtes de colonnes…).
fn caps(p: &Painter, pos: Pos2, s: &str, col: Color32, max_w: f32) -> Rect {
    text_ls(p, pos, s, bold(10.5), col, max_w, 0.9)
}

/// Rect arrondi rempli d'un dégradé diagonal (a en haut-gauche → b en bas-droite), bords lissés.
pub fn grad_rrect(p: &Painter, r: Rect, rad: f32, a: Color32, b: Color32) {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    p.rect_filled(r, CornerRadius::same(rad as u8), a.lerp_to_gamma(b, 0.5));
    let r = r.shrink(0.6);
    let rad = (rad - 0.6).max(0.0);
    let mut pts = Vec::with_capacity(52);
    for (cx, cy, a0) in [
        (r.right() - rad, r.bottom() - rad, 0.0f32),
        (r.left() + rad, r.bottom() - rad, 90.0),
        (r.left() + rad, r.top() + rad, 180.0),
        (r.right() - rad, r.top() + rad, 270.0),
    ] {
        for k in 0..=12 {
            let ang = (a0 + 90.0 * k as f32 / 12.0).to_radians();
            pts.push(pos2(cx + rad * ang.cos(), cy + rad * ang.sin()));
        }
    }
    let col = |q: Pos2| {
        let t = (((q.x - r.left()) / r.width() + (q.y - r.top()) / r.height()) * 0.5).clamp(0.0, 1.0);
        a.lerp_to_gamma(b, t)
    };
    let mut m = Mesh::default();
    m.colored_vertex(r.center(), col(r.center()));
    for q in &pts {
        m.colored_vertex(*q, col(*q));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(m));
}

/// Dégradé vertical par paliers (y relatif 0..1, couleur).
fn vgrad(p: &Painter, r: Rect, stops: &[(f32, Color32)]) {
    let mut m = Mesh::default();
    for (i, (t, c)) in stops.iter().enumerate() {
        let y = r.top() + r.height() * t;
        m.colored_vertex(pos2(r.left(), y), *c);
        m.colored_vertex(pos2(r.right(), y), *c);
        if i > 0 {
            let k = (i as u32) * 2;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(Shape::mesh(m));
}

fn shadow(p: &Painter, r: Rect, rad: u8, a: f32) {
    if a > 0.01 {
        p.add(Shadow { offset: [0, 14], blur: 34, spread: 0, color: Color32::from_black_alpha((150.0 * a) as u8) }.as_shape(r, CornerRadius::same(rad)));
    }
}

/// Égaliseur (morceau en cours).
fn eq_bars(p: &Painter, c: Pos2, h: f32, t: f32, live: bool, col: Color32) {
    const PH: [f32; 4] = [0.0, 1.7, 0.9, 2.6];
    const FR: [f32; 4] = [5.1, 6.7, 4.3, 7.4];
    const STILL: [f32; 4] = [0.5, 0.9, 0.68, 0.38];
    for i in 0..4 {
        let k = if live { 0.28 + 0.72 * (0.5 + 0.5 * (t * FR[i] + PH[i]).sin()) } else { STILL[i] };
        let bh = (h * k).max(2.0);
        let x = c.x + (i as f32 - 1.5) * 4.2;
        p.rect_filled(Rect::from_min_max(pos2(x - 1.3, c.y + h / 2.0 - bh), pos2(x + 1.3, c.y + h / 2.0)), CornerRadius::same(1), col);
    }
}

// ------------------------------------------------------------------ images

pub struct Images {
    cache: LruCache<String, (TextureHandle, usize)>,
    bytes: usize,
    pending: HashSet<String>,
    failed: HashSet<String>,
    fade: HashMap<String, f64>,
    /// couleurs dominantes de chaque pochette chargée (pour la lueur et l'accent)
    pal: HashMap<String, Pal>,
    pool: Pool,
    ctx: Context,
}

impl Images {
    pub fn new(pool: Pool, ctx: Context) -> Self {
        Self { cache: LruCache::unbounded(), bytes: 0, pending: HashSet::new(), failed: HashSet::new(), fade: HashMap::new(), pal: HashMap::new(), pool, ctx }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.bytes = 0;
        self.failed.clear();
    }

    pub fn on_loaded(&mut self, key: String, img: Option<egui::ColorImage>) {
        self.pending.remove(&key);
        match img {
            Some(ci) => {
                if self.pal.len() > 3000 {
                    self.pal.clear();
                }
                self.pal.insert(key.clone(), ambient::extract(&ci));
                let sz = ci.pixels.len() * 4;
                let tex = self.ctx.load_texture(key.clone(), ci, TextureOptions::LINEAR);
                self.fade.insert(key.clone(), self.ctx.input(|i| i.time));
                self.cache.put(key, (tex, sz));
                self.bytes += sz;
                while self.bytes > IMG_BUDGET {
                    match self.cache.pop_lru() {
                        Some((_, (_, s))) => self.bytes -= s,
                        None => break,
                    }
                }
            }
            None => {
                self.failed.insert(key);
            }
        }
    }

    /// Palette d'une pochette déjà chargée à cette taille.
    pub fn palette(&self, url: &str, px: u32) -> Option<Pal> {
        if url.is_empty() {
            return None;
        }
        self.pal.get(&img_size(url, px)).copied()
    }

    pub fn paint(&mut self, p: &Painter, rect: Rect, url: &str, px: u32, radius: u8) {
        let cr = CornerRadius::same(radius);
        let empty = Color32::from_rgba_premultiplied(14, 14, 16, 14);
        if url.is_empty() {
            p.rect_filled(rect, cr, empty);
            return;
        }
        let key = img_size(url, px);
        if let Some((t, _)) = self.cache.get(&key) {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let mut a = 1.0f32;
            if let Some(t0) = self.fade.get(&key).copied() {
                a = ((self.ctx.input(|i| i.time) - t0) as f32 / 0.35).clamp(0.0, 1.0);
                if a >= 1.0 {
                    self.fade.remove(&key);
                } else {
                    self.ctx.request_repaint();
                    p.rect_filled(rect, cr, empty);
                }
            }
            p.add(RectShape::filled(rect, cr, Color32::WHITE.gamma_multiply(ease(a))).with_texture(t.id(), uv));
            return;
        }
        p.rect_filled(rect, cr, empty);
        if !self.pending.contains(&key) && !self.failed.contains(&key) {
            self.pending.insert(key.clone());
            self.pool.image(key);
        }
    }
}

// ------------------------------------------------------------------ contexte de rendu

pub struct Cx<'a> {
    pub imgs: &'a mut Images,
    pub lib: &'a Library,
    pub acts: &'a mut Vec<Act>,
    pub cur_id: u64,
    pub playing: bool,
    pub shuffle: bool,
    /// animations continues autorisées (fenêtre active, lecture, lueur animée)
    pub live: bool,
    pub t: f32,
}

pub fn route_of(kind: Kind, id: u64) -> Option<Route> {
    Some(match kind {
        Kind::Album => Route::Album(id),
        Kind::Artist => Route::Artist(id),
        Kind::Playlist => Route::Playlist(id),
        Kind::Podcast => Route::Podcast(id),
        Kind::Radio => Route::Radio(id),
        Kind::Genre => Route::Genre(id),
        Kind::Track => return None,
    })
}

fn web_url(kind: Kind, id: u64) -> String {
    let p = match kind {
        Kind::Track => "track",
        Kind::Album => "album",
        Kind::Artist => "artist",
        Kind::Playlist => "playlist",
        Kind::Podcast => "show",
        Kind::Radio => "radio",
        Kind::Genre => "channels/explore",
    };
    format!("https://www.deezer.com/{p}/{id}")
}

fn explicit_badge(p: &Painter, pos: Pos2) -> f32 {
    let r = Rect::from_min_size(pos, vec2(15.0, 15.0));
    p.rect_filled(r, CornerRadius::same(4), PRESS);
    p.text(r.center(), Align2::CENTER_CENTER, "E", bold(9.5), SECOND);
    19.0
}

/// Bouton primaire rond : blanc, icône sombre.
/// Défilement des paroles : ressort amorti (léger rebond), cascade ligne par ligne,
/// galleys mis en cache par morceau. Aucun calcul tant que rien ne bouge.
#[derive(Default)]
pub struct LyAnim {
    key: (u64, u32, usize),
    galleys: Vec<Option<Arc<egui::Galley>>>,
    big: Vec<Option<Arc<egui::Galley>>>,
    starts: Vec<u32>,
    ys: Vec<f32>,
    hs: Vec<f32>,
    hb: Vec<f32>,
    gap: f32,
    total: f32,
    synced: bool,
    cur: Option<usize>,
    from: Vec<f32>,
    target: f32,
    t0: Option<Instant>,
    dur: f32,
    stagger: bool,
    anchor: usize,
    manual: Option<Instant>,
}

const LY_SMALL: f32 = 22.0;
const LY_BIG: f32 = 34.0;

/// Axe dominant du geste de défilement en cours (0 = horizontal, 1 = vertical), verrouillé
/// tant que le geste continue (pause < 180 ms) : évite que x et y se mélangent.
fn scroll_axis(ui: &Ui) -> Option<u8> {
    use std::cell::Cell;
    thread_local!(static LOCK: Cell<(u8, f64)> = const { Cell::new((1, 0.0)) });
    let (d, t) = ui.input(|i| (i.smooth_scroll_delta, i.time));
    LOCK.with(|l| {
        let (ax, last) = l.get();
        if d == egui::Vec2::ZERO {
            return (t - last < 0.18).then_some(ax);
        }
        let ax = if t - last < 0.18 { ax } else if d.x.abs() > d.y.abs() { 0 } else { 1 };
        l.set((ax, t));
        Some(ax)
    })
}

/// Ressort amorti normalisé (ζ ≈ 0,76) : 0 → 1 avec ~2,5 % de dépassement.
fn spring(u: f32) -> f32 {
    if u <= 0.0 {
        0.0
    } else if u >= 1.6 {
        1.0
    } else {
        1.0 - (-7.0 * u).exp() * ((6.0 * u).cos() + 7.0 / 6.0 * (6.0 * u).sin())
    }
}

impl LyAnim {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    fn delay(&self, i: usize) -> f32 {
        if !self.stagger {
            return 0.0;
        }
        ((i as f32 - self.anchor as f32 + 1.0).max(0.0) * 0.05).min(0.45)
    }
    fn value(&self, i: usize, now: Instant) -> f32 {
        let Some(t0) = self.t0 else { return self.from.get(i).copied().unwrap_or(self.target) };
        let t = now.duration_since(t0).as_secs_f32() - self.delay(i);
        let from = self.from.get(i).copied().unwrap_or(self.target);
        from + (self.target - from) * spring(t / self.dur.max(0.05))
    }
    fn retarget(&mut self, target: f32, stagger: bool, anchor: usize, dur: f32, now: Instant) {
        let from: Vec<f32> = (0..self.galleys.len()).map(|i| self.value(i, now)).collect();
        self.from = from;
        self.target = target;
        self.t0 = Some(now);
        self.dur = dur;
        self.stagger = stagger;
        self.anchor = anchor;
    }
    fn moving(&self, now: Instant) -> bool {
        self.t0.is_some_and(|t0| now.duration_since(t0).as_secs_f32() < self.dur * 1.6 + if self.stagger { 0.45 } else { 0.0 })
    }
}

/// Piste de curseur (progression, volume) : fond translucide, remplissage, bouton au survol.
fn slider(p: &Painter, bar: Rect, frac: f32, k: f32, fill: Color32) {
    let bar = bar.expand2(vec2(0.0, k));
    let r = CornerRadius::same((bar.height() / 2.0).ceil() as u8);
    p.rect_filled(bar, r, Color32::from_white_alpha(30));
    if frac > 0.0 {
        p.rect_filled(Rect::from_min_size(bar.min, vec2((bar.width() * frac).max(bar.height()), bar.height())), r, fill);
    }
    if k > 0.01 {
        let c = pos2(bar.min.x + bar.width() * frac, bar.center().y);
        p.circle_filled(c + vec2(0.0, 1.0), 6.5 * k, Color32::from_black_alpha((90.0 * k) as u8));
        p.circle_filled(c, 6.0 * k, Color32::WHITE);
    }
}

/// Interrupteur (style macOS, blanc quand actif).
fn switch(ui: &mut Ui, on: bool) -> Response {
    let (r, resp) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
    let t = ease(ui.ctx().animate_bool_with_time(resp.id, on, 0.16));
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(11), Color32::from_white_alpha(28).lerp_to_gamma(INK, t));
    let x = r.min.x + 11.0 + (r.width() - 22.0) * t;
    p.circle_filled(pos2(x, r.center().y), 8.5, SECOND.lerp_to_gamma(BG, t));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn set_head(ui: &mut Ui, s: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(s.to_uppercase()).font(bold(10.5)).extra_letter_spacing(0.9).color(MUTED));
    ui.add_space(6.0);
}

fn set_group(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(Color32::from_white_alpha(5))
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, LINE))
        .inner_margin(Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

fn set_row(ui: &mut Ui, title: &str, sub: &str, sep: bool, ctrl: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let h = if sub.is_empty() { 46.0 } else { 58.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if sep {
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, LINE));
    }
    let left = ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)), ctrl).response.rect.left();
    let tw = (left.min(rect.max.x) - rect.min.x - 16.0).max(60.0);
    let p = ui.painter();
    if sub.is_empty() {
        text(p, pos2(rect.min.x, rect.center().y - 9.0), title, med(13.5), INK, tw);
    } else {
        text(p, pos2(rect.min.x, rect.center().y - 18.0), title, med(13.5), INK, tw);
        text(p, pos2(rect.min.x, rect.center().y + 2.0), sub, reg(12.0), MUTED, tw);
    }
}

fn round_btn(ui: &mut Ui, size: f32, ic: Ic) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.14);
    let pr = if resp.is_pointer_button_down_on() { 0.95 } else { 1.0 };
    let r = size / 2.0 * (1.0 + 0.05 * h) * pr;
    ui.painter().circle_filled(rect.center() + vec2(0.0, 3.0), r, Color32::from_black_alpha(70));
    ui.painter().circle_filled(rect.center(), r, INK.lerp_to_gamma(Color32::WHITE, h));
    icons::draw(ui.painter(), ic, rect.center() + vec2(if ic == Ic::Play { size * 0.035 } else { 0.0 }, 0.0), size * 0.36, BG);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bouton rond secondaire en verre.
fn ghost_round(ui: &mut Ui, size: f32, ic: Ic, col: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.14);
    let p = ui.painter();
    p.circle(rect.center(), size / 2.0, CARD.lerp_to_gamma(HOVER, h), Stroke::new(1.0, LINE));
    icons::draw(p, ic, rect.center(), size * 0.42, col.lerp_to_gamma(Color32::WHITE, h * 0.5));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Capsule : `filled` = primaire blanche, sinon verre + filet.
fn pill(ui: &mut Ui, label: &str, filled: bool) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_owned(), med(13.5), INK);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, 36.0), Sense::click());
    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.14);
    let p = ui.painter();
    let fg = if filled {
        p.rect_filled(rect, CornerRadius::same(18), INK.lerp_to_gamma(Color32::WHITE, h));
        BG
    } else {
        p.rect(rect, CornerRadius::same(18), CARD.lerp_to_gamma(HOVER, h), Stroke::new(1.0, LINE), StrokeKind::Inside);
        INK
    };
    let g = ui.painter().layout_no_wrap(label.to_owned(), med(13.5), fg);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Titre de section : SF Pro très gras, approche serrée.
fn section_title(ui: &mut Ui, s: &str, size: f32) {
    ui.label(RichText::new(s).font(display(size)).extra_letter_spacing(-size * 0.022).color(INK));
}

// ------------------------------------------------------------------ cartes / tuiles

pub const CARD_W: f32 = 164.0;

fn card_ui(ui: &mut Ui, cx: &mut Cx, c: &Card) {
    let (slot, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_W + 72.0), Sense::click());
    if !ui.is_rect_visible(slot) {
        return;
    }
    let id = ui.scope_id().with(("card", c.kind as u8, c.id));
    let hot = ui.rect_contains_pointer(slot);
    let h = anim(ui, id.with("h"), hot, 0.18);
    let rect = slot.translate(vec2(0.0, -4.0 * h));
    let p = ui.painter().clone();
    let img = Rect::from_min_size(rect.min, vec2(CARD_W, CARD_W));
    let round = c.kind == Kind::Artist;
    let rad = if round { 255 } else { 12 };
    shadow(&p, img, rad.min(82), 0.3 + 0.7 * h);
    cx.imgs.paint(&p, img, &c.img, 264, rad);
    let playable = !matches!(c.kind, Kind::Genre);
    let mut play_click = false;
    if playable && h > 0.02 {
        let ctr = img.right_bottom() - vec2(28.0, 28.0 - 8.0 * (1.0 - h));
        let pr = Rect::from_center_size(ctr, vec2(42.0, 42.0));
        let presp = ui.interact(pr, id.with("play"), Sense::click());
        let r = (17.0 + 3.0 * h + if presp.hovered() { 1.5 } else { 0.0 }) * (0.85 + 0.15 * h);
        p.circle_filled(ctr + vec2(0.0, 3.0), r, Color32::from_black_alpha((90.0 * h) as u8));
        p.circle_filled(ctr, r, INK.gamma_multiply(h));
        icons::draw(&p, Ic::Play, ctr + vec2(1.2, 0.0), 14.0, BG.gamma_multiply(h));
        play_click = presp.clicked();
    }
    let ty = rect.min.y + CARD_W + 10.0;
    if round {
        let cx_ = rect.center().x;
        text_center(&p, cx_, ty, &c.title, med(14.0), INK, CARD_W);
        if !c.sub.is_empty() {
            text_center(&p, cx_, ty + 20.0, &c.sub, reg(12.5), MUTED, CARD_W);
        }
    } else {
        let mut x = rect.min.x;
        if c.explicit {
            explicit_badge(&p, pos2(x, ty + 1.0));
            x += 19.0;
        }
        text(&p, pos2(x, ty), &c.title, med(14.0), INK, CARD_W - (x - rect.min.x));
        if !c.sub.is_empty() {
            text(&p, pos2(rect.min.x, ty + 20.0), &c.sub, reg(12.5), MUTED, CARD_W);
        }
        if !c.extra.is_empty() {
            text(&p, pos2(rect.min.x, ty + 38.0), &c.extra, reg(12.0), FAINT, CARD_W);
        }
    }
    if play_click {
        match &c.track {
            Some(t) => cx.acts.push(Act::PlayList { tracks: vec![(**t).clone()], start: 0, shuffle: false }),
            None => cx.acts.push(Act::PlayContent(c.kind, c.id, false)),
        }
    } else if resp.clicked() {
        match (&c.track, route_of(c.kind, c.id)) {
            (Some(t), _) => cx.acts.push(Act::PlayList { tracks: vec![(**t).clone()], start: 0, shuffle: false }),
            (None, Some(r)) => cx.acts.push(Act::Go(r)),
            _ => {}
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

fn tile_ui(ui: &mut Ui, cx: &mut Cx, c: &Card, w: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let id = ui.scope_id().with(("tile", c.kind as u8, c.id));
    let h = anim(ui, id.with("h"), ui.rect_contains_pointer(rect), 0.14);
    let p = ui.painter().clone();
    p.rect(rect, CornerRadius::same(12), CARD.lerp_to_gamma(HOVER, h), Stroke::new(1.0, LINE), StrokeKind::Inside);
    let img = Rect::from_min_size(rect.min, vec2(64.0, 64.0)).shrink(6.0);
    cx.imgs.paint(&p, img, &c.img, 120, 8);
    let mut x = rect.min.x + 72.0;
    let tw = w - 72.0 - 52.0;
    let r = text(&p, pos2(x, rect.min.y + 14.0), &c.title, med(14.0), INK, tw);
    if c.explicit {
        x = r.right() + 6.0;
        explicit_badge(&p, pos2(x, rect.min.y + 15.0));
    }
    if !c.sub.is_empty() {
        text(&p, pos2(rect.min.x + 72.0, rect.min.y + 35.0), &c.sub, reg(12.5), MUTED, tw);
    }
    let mut play_click = false;
    if h > 0.02 {
        let ctr = pos2(rect.right() - 28.0 + 6.0 * (1.0 - h), rect.center().y);
        let pr = Rect::from_center_size(ctr, vec2(38.0, 38.0));
        let presp = ui.interact(pr, id.with("play"), Sense::click());
        p.circle_filled(ctr, 15.0 + 1.5 * presp.hovered() as i32 as f32, INK.gamma_multiply(h));
        icons::draw(&p, Ic::Play, ctr + vec2(1.0, 0.0), 12.0, BG.gamma_multiply(h));
        play_click = presp.clicked();
    }
    if play_click {
        cx.acts.push(Act::PlayContent(c.kind, c.id, false));
    } else if resp.clicked() {
        if let Some(r) = route_of(c.kind, c.id) {
            cx.acts.push(Act::Go(r));
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

fn section_ui(ui: &mut Ui, cx: &mut Cx, offs: &mut HashMap<String, CarState>, s: &Section, gen: u64) {
    let key = format!("{gen}:{}", s.title);
    ui.add_space(26.0);
    let step = (CARD_W + 18.0) * 3.0;
    let st = offs.entry(key.clone()).or_default();
    if !s.title.is_empty() {
        ui.horizontal(|ui| {
            section_title(ui, &s.title, 26.0);
            if s.style == Style::Carousel {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ghost_round(ui, 30.0, Ic::Right, SECOND).clicked() {
                        let from = st.anim.map(|a| a.1).unwrap_or(st.off);
                        st.anim = Some((st.off, from + step, Instant::now()));
                    }
                    if ghost_round(ui, 30.0, Ic::Left, SECOND).clicked() {
                        let from = st.anim.map(|a| a.1).unwrap_or(st.off);
                        st.anim = Some((st.off, (from - step).max(0.0), Instant::now()));
                    }
                });
            }
        });
        ui.add_space(10.0);
    }
    match s.style {
        Style::Carousel => {
            // molette/trackpad gérés à la main avec verrouillage d'axe : un geste vertical au-dessus
            // d'un carrousel fait défiler la page, un geste horizontal le carrousel, jamais les deux
            let mut sa = ScrollArea::horizontal()
                .id_salt(&key)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .scroll_source(egui::scroll_area::ScrollSource { scroll_bar: false, drag: egui::scroll_area::DragScroll::OnTouch, mouse_wheel: false })
                .auto_shrink([false, true]);
            if st.rect.is_some_and(|r| ui.rect_contains_pointer(r)) && scroll_axis(ui) == Some(0) {
                let dx = ui.input(|i| i.smooth_scroll_delta.x);
                ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                if dx != 0.0 {
                    st.anim = None;
                    st.off = (st.off - dx).clamp(0.0, st.max.max(0.0));
                    sa = sa.horizontal_scroll_offset(st.off);
                }
            }
            if let Some((from, to, t0)) = st.anim {
                let k = (t0.elapsed().as_secs_f32() / 0.45).min(1.0);
                let e = 1.0 - (1.0 - k).powi(3);
                sa = sa.horizontal_scroll_offset(from + (to - from) * e);
                if k >= 1.0 {
                    st.anim = None;
                } else {
                    ui.ctx().request_repaint();
                }
            }
            let out = sa.show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    for c in &s.cards {
                        card_ui(ui, cx, c);
                    }
                });
            });
            st.off = out.state.offset().x;
            // inner_rect s'étend jusqu'en bas de la page : on ne garde que la hauteur réelle du
            // carrousel, sinon le premier capte le défilement destiné à ceux du dessous
            st.rect = Some(Rect::from_min_size(out.inner_rect.min, vec2(out.inner_rect.width(), out.content_size.y.min(out.inner_rect.height()))));
            st.max = out.content_size.x - out.inner_rect.width();
        }
        Style::Tiles => {
            let gap = 12.0;
            let w = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
            for row in s.cards.chunks(3) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                    for c in row {
                        tile_ui(ui, cx, c, w);
                    }
                });
                ui.add_space(10.0);
            }
        }
        Style::Grid => {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
                for c in &s.cards {
                    card_ui(ui, cx, c);
                }
            });
        }
    }
}

// ------------------------------------------------------------------ menu contextuel d'un morceau

fn track_menu(ui: &mut Ui, cx: &mut Cx, t: &Track, playlist_ctx: Option<u64>) {
    ui.set_min_width(220.0);
    if ui.button("Play next").clicked() {
        cx.acts.push(Act::PlayNext(t.clone()));
        ui.close();
    }
    if ui.button("Add to queue").clicked() {
        cx.acts.push(Act::Enqueue(t.clone()));
        ui.close();
    }
    let liked = cx.lib.fav.contains(&t.id);
    if ui.button(if liked { "Remove from Favourite tracks" } else { "Add to Favourite tracks" }).clicked() {
        cx.acts.push(Act::Like(t.clone()));
        ui.close();
    }
    ui.menu_button("Add to playlist", |ui| {
        ui.set_min_width(210.0);
        if ui.button("New playlist…").clicked() {
            cx.acts.push(Act::OpenNewPlaylist);
            ui.close();
        }
        let me = cx.lib.me_id;
        ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for pl in cx.lib.playlists.iter().filter(|p| me == 0 || p.owner == me) {
                if ui.button(&pl.title).clicked() {
                    cx.acts.push(Act::AddToPlaylist { pl: pl.id, track: t.id });
                    ui.close();
                }
            }
        });
    });
    if let Some(pl) = playlist_ctx {
        if ui.button("Remove from this playlist").clicked() {
            cx.acts.push(Act::RemoveFromPlaylist { pl, track: t.id });
            ui.close();
        }
    }
    ui.separator();
    if t.artist_id != 0 && ui.button("Go to artist").clicked() {
        cx.acts.push(Act::Go(Route::Artist(t.artist_id)));
        ui.close();
    }
    if t.album_id != 0 && ui.button("Go to album").clicked() {
        cx.acts.push(Act::Go(Route::Album(t.album_id)));
        ui.close();
    }
    if ui.button("Track mix").clicked() {
        cx.acts.push(Act::TrackRadio(t.id));
        ui.close();
    }
    ui.separator();
    if ui.button("Open on deezer.com").clicked() {
        cx.acts.push(Act::OpenWeb(web_url(Kind::Track, t.id)));
        ui.close();
    }
    if ui.button("Copy link").clicked() {
        cx.acts.push(Act::CopyText(web_url(Kind::Track, t.id)));
        ui.close();
    }
}

// ------------------------------------------------------------------ tableau de morceaux (virtualisé)

const ROW_H: f32 = 58.0;

fn track_table(ui: &mut Ui, cx: &mut Cx, ps: &mut PageState, playlist_ctx: Option<u64>) {
    if ps.dirty {
        ps.rebuild_view();
    }
    let w = ui.available_width();
    let xs = [0.0, w * 0.47, w * 0.63, w * 0.79, w * 0.88, w * 0.94];
    // en-tête de colonnes
    let (hr, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    let heads = [("TITLE", Col::Title, 12.0), ("ARTIST", Col::Artist, xs[1]), ("ALBUM", Col::Album, xs[2]), ("ADDED", Col::Added, xs[3]), ("TIME", Col::Dur, xs[4]), ("POP.", Col::Pop, xs[5])];
    for (i, (label, col, x)) in heads.iter().enumerate() {
        let r = Rect::from_min_size(pos2(hr.min.x + x, hr.min.y), vec2(if i + 1 < heads.len() { heads[i + 1].2 - x } else { 60.0 }, 34.0));
        let resp = ui.interact(r, ui.scope_id().with(("th", i)), Sense::click());
        let sorted = ps.sort.filter(|(c, _)| c == col);
        let colr = if sorted.is_some() || resp.hovered() { INK } else { FAINT };
        let tr = caps(ui.painter(), pos2(r.min.x, hr.min.y + 11.0), label, colr, r.width() - 16.0);
        if let Some((_, asc)) = sorted {
            let c = pos2(tr.right() + 9.0, tr.center().y);
            let pts = if asc { vec![c + vec2(-3.5, 2.0), c + vec2(3.5, 2.0), c + vec2(0.0, -2.5)] } else { vec![c + vec2(-3.5, -2.0), c + vec2(3.5, -2.0), c + vec2(0.0, 2.5)] };
            ui.painter().add(Shape::convex_polygon(pts, INK, Stroke::NONE));
        }
        if resp.clicked() {
            cx.acts.push(Act::Sort(*col));
        }
    }
    ui.painter().hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, LINE));
    ui.add_space(4.0);

    let n = if ps.data.preview > 0 && !ps.expanded && ps.filter.is_empty() { ps.view.len().min(ps.data.preview) } else { ps.view.len() };
    let (area, _) = ui.allocate_exact_size(vec2(w, n as f32 * ROW_H), Sense::hover());
    let clip = ui.clip_rect();
    let first = (((clip.min.y - area.min.y) / ROW_H).floor().max(0.0)) as usize;
    let last = ((((clip.max.y - area.min.y) / ROW_H).ceil()).max(0.0) as usize).min(n);
    let p = ui.painter().clone();
    for vi in first..last {
        let t = &ps.data.tracks[ps.view[vi] as usize];
        let rect = Rect::from_min_size(pos2(area.min.x, area.min.y + vi as f32 * ROW_H), vec2(w, ROW_H));
        let id = ui.scope_id().with(("row", vi, t.id));
        // la ligne entière est cliquable (un seul clic = lecture) ; cœur, menu et liens sont
        // enregistrés après => ils passent devant et gardent leurs propres clics.
        let resp = ui.interact(rect, id, Sense::click());
        let is_cur = t.id == cx.cur_id;
        let hot = ui.rect_contains_pointer(rect);
        let hh = anim(ui, id.with("hh"), hot, 0.12);
        let card = rect.shrink2(vec2(0.0, 2.0));
        if is_cur {
            p.rect(card, CornerRadius::same(12), CARD.lerp_to_gamma(HOVER, hh), Stroke::new(1.0, LINE), StrokeKind::Inside);
        } else if hh > 0.01 {
            p.rect_filled(card, CornerRadius::same(12), CARD.lerp_to_gamma(HOVER, 0.4).gamma_multiply(hh));
        }
        if resp.is_pointer_button_down_on() {
            p.rect_filled(card, CornerRadius::same(12), CARD);
        }
        let cy = rect.center().y;
        // pochette + overlay
        let img = Rect::from_center_size(pos2(rect.min.x + 10.0 + 22.0, cy), vec2(44.0, 44.0));
        cx.imgs.paint(&p, img, &t.img, 96, 7);
        let ov = if is_cur { 1.0 } else { hh };
        if ov > 0.01 {
            p.rect_filled(img, CornerRadius::same(7), Color32::from_black_alpha((140.0 * ov) as u8));
            if is_cur && !hot {
                eq_bars(&p, img.center(), 16.0, cx.t, cx.live && cx.playing, acc_text());
            } else {
                icons::draw(&p, if is_cur && cx.playing { Ic::Pause } else { Ic::Play }, img.center() + vec2(1.0, 0.0), 15.0 * (0.75 + 0.25 * ov), INK.gamma_multiply(ov));
            }
        }
        // titre
        let tx = img.right() + 14.0;
        let tw = (xs[1] - tx - 90.0).max(60.0);
        let tr = text(&p, pos2(tx, cy - 9.5), &t.title, med(14.5), if is_cur { acc_text() } else { INK }, tw);
        if t.explicit {
            explicit_badge(&p, pos2(tr.right() + 7.0, cy - 7.5));
        }
        // coeur + menu
        let hx = rect.min.x + xs[1] - 70.0;
        let heart = Rect::from_center_size(pos2(hx, cy), vec2(30.0, 30.0));
        let hresp = ui.interact(heart, id.with("heart"), Sense::click());
        let liked = cx.lib.fav.contains(&t.id);
        let pop = heart_pop(ui, id.with("pop"), liked);
        let hh2 = anim(ui, id.with("hh2"), hresp.hovered(), 0.12);
        if liked {
            icons::draw(&p, Ic::Heart, heart.center(), 16.0 * pop * (1.0 + 0.08 * hh2), ACCENT);
        } else if hh > 0.01 {
            icons::draw(&p, Ic::HeartOutline, heart.center(), 16.0 * (1.0 + 0.08 * hh2), MUTED.lerp_to_gamma(INK, hh2).gamma_multiply(hh));
        }
        if hresp.clicked() {
            cx.acts.push(Act::Like(t.clone()));
        }
        hresp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let dots = Rect::from_center_size(pos2(hx + 34.0, cy), vec2(30.0, 30.0));
        let dresp = ui.interact(dots, id.with("dots"), Sense::click());
        if hh > 0.01 {
            if dresp.hovered() {
                p.circle_filled(dots.center(), 14.0, HOVER);
            }
            icons::draw(&p, Ic::Dots, dots.center(), 16.0, if dresp.hovered() { INK } else { MUTED.gamma_multiply(hh) });
        }
        egui::Popup::menu(&dresp).show(|ui| track_menu(ui, cx, t, playlist_ctx));
        egui::Popup::context_menu(&resp).show(|ui| track_menu(ui, cx, t, playlist_ctx));

        // colonnes texte
        let col_artist = text(&p, pos2(rect.min.x + xs[1], cy - 8.5), &t.artist, reg(14.0), SECOND, xs[2] - xs[1] - 14.0);
        let ar = ui.interact(col_artist.expand(2.0), id.with("artist"), Sense::click());
        if ar.hovered() && t.artist_id != 0 {
            p.hline(col_artist.x_range(), col_artist.bottom(), Stroke::new(1.0, SECOND));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if ar.clicked() && t.artist_id != 0 {
            cx.acts.push(Act::Go(Route::Artist(t.artist_id)));
        }
        if !t.album.is_empty() {
            let col_alb = text(&p, pos2(rect.min.x + xs[2], cy - 8.5), &t.album, reg(14.0), SECOND, xs[3] - xs[2] - 14.0);
            let alr = ui.interact(col_alb.expand(2.0), id.with("album"), Sense::click());
            if alr.hovered() && t.album_id != 0 {
                p.hline(col_alb.x_range(), col_alb.bottom(), Stroke::new(1.0, SECOND));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if alr.clicked() && t.album_id != 0 {
                cx.acts.push(Act::Go(Route::Album(t.album_id)));
            }
        }
        if t.added > 0 {
            text(&p, pos2(rect.min.x + xs[3], cy - 8.0), &fmt_date_ts(t.added), reg(13.0), MUTED, xs[4] - xs[3]);
        }
        text(&p, pos2(rect.min.x + xs[4], cy - 7.5), &fmt_dur(t.dur), mono(12.5), MUTED, 60.0);
        // popularité (5 barres)
        let lvl = ((t.rank as f32 / 180_000.0).ceil() as i32).clamp(0, 5);
        for b in 0..5 {
            let x = rect.min.x + xs[5] + b as f32 * 5.0;
            let h = 8.0 + b as f32 * 1.5;
            let col = if b < lvl { SECOND } else { PRESS };
            p.rect_filled(Rect::from_min_size(pos2(x, cy + 6.0 - h), vec2(2.5, h)), CornerRadius::same(1), col);
        }
        if resp.clicked() {
            if is_cur {
                cx.acts.push(Act::Toggle);
            } else {
                let tracks: Vec<Track> = ps.view.iter().map(|&i| ps.data.tracks[i as usize].clone()).collect();
                cx.acts.push(Act::PlayList { tracks, start: vi, shuffle: cx.shuffle });
            }
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
}

// ------------------------------------------------------------------ en-tête de page

fn kind_label(k: Option<Kind>, fav: bool) -> &'static str {
    if fav {
        return "PLAYLIST";
    }
    match k {
        Some(Kind::Album) => "ALBUM",
        Some(Kind::Artist) => "ARTIST",
        Some(Kind::Playlist) => "PLAYLIST",
        Some(Kind::Podcast) => "PODCAST",
        Some(Kind::Radio) => "MIX",
        Some(Kind::Genre) => "GENRE",
        _ => "",
    }
}

fn header_ui(ui: &mut Ui, cx: &mut Cx, ps: &mut PageState, route: &Route) {
    let Some(h) = ps.data.header.clone() else { return };
    let w = ui.available_width();
    let has_art = !h.img.is_empty();
    let art_sz = 220.0;
    let hh = if has_art { art_sz } else { 64.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(w, hh), Sense::hover());
    let p = ui.painter().clone();
    let mut tx = rect.min.x;
    let is_fav = h.img == "fav";
    if has_art {
        let art = Rect::from_min_size(rect.min, vec2(art_sz, art_sz));
        let rad = if h.round { 110 } else { 16 };
        p.add(Shadow { offset: [0, 22], blur: 56, spread: 0, color: Color32::from_black_alpha(190) }.as_shape(art, CornerRadius::same(rad)));
        if is_fav {
            grad_rrect(&p, art, 16.0, VIOLET, ACCENT);
            // reflet doux + cœur blanc
            p.circle_filled(art.center() + vec2(-40.0, -50.0), 90.0, Color32::from_white_alpha(10));
            icons::draw(&p, Ic::Heart, art.center() + vec2(0.0, 4.0), art_sz * 0.42, Color32::WHITE);
        } else {
            cx.imgs.paint(&p, art, &h.img, 400, rad);
        }
        tx += art_sz + 32.0;
    }
    let tw = (rect.right() - tx).max(100.0);
    let n = h.title.chars().count();
    let size = if !has_art { 40.0 } else if n > 28 { 34.0 } else if n > 16 { 44.0 } else { 56.0 };
    let label = kind_label(h.kind, is_fav);
    let mut y = if has_art { rect.min.y + (art_sz - (size + 104.0)).max(0.0) * 0.55 } else { rect.min.y + 6.0 };
    if has_art && !label.is_empty() {
        caps(&p, pos2(tx + 2.0, y), label, MUTED, tw);
        y += 20.0;
    }
    title_text(&p, pos2(tx - 1.0, y), &h.title, size, INK, tw);
    y += size * 1.18 + 8.0;
    if !h.line.is_empty() {
        let r = text(&p, pos2(tx + 1.0, y), &h.line, med(15.0), INK, tw);
        if h.line_id != 0 {
            let resp = ui.interact(r.expand(3.0), ui.scope_id().with("hline"), Sense::click());
            if resp.hovered() {
                p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, INK));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                cx.acts.push(Act::Go(Route::Artist(h.line_id)));
            }
        }
        y += 24.0;
    }
    if !h.meta.is_empty() {
        text(&p, pos2(tx + 1.0, y), &h.meta, reg(13.5), MUTED, tw);
        y += 22.0;
    }
    if !h.desc.is_empty() && has_art {
        let d: String = h.desc.chars().take(180).collect();
        text(&p, pos2(tx + 1.0, y), &d, reg(13.0), FAINT, tw);
    }

    // barre d'actions
    let kind = h.kind;
    let has_actions = kind.is_some() || is_fav;
    if has_actions {
        ui.add_space(24.0);
        let me = cx.lib.me_id;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let ordered = |ps: &PageState| -> Vec<Track> { ps.view.iter().map(|&i| ps.data.tracks[i as usize].clone()).collect() };
            if round_btn(ui, 54.0, Ic::Play).on_hover_text("Play").clicked() {
                if !ps.data.tracks.is_empty() {
                    cx.acts.push(Act::PlayList { tracks: ordered(ps), start: 0, shuffle: cx.shuffle });
                } else if let Some(k) = kind {
                    cx.acts.push(Act::PlayContent(k, h.id, false));
                }
            }
            if ghost_round(ui, 42.0, Ic::Shuffle, INK).on_hover_text("Shuffle play").clicked() {
                if !ps.data.tracks.is_empty() {
                    cx.acts.push(Act::PlayList { tracks: ordered(ps), start: 0, shuffle: true });
                } else if let Some(k) = kind {
                    cx.acts.push(Act::PlayContent(k, h.id, true));
                }
            }
            match kind {
                Some(Kind::Artist) => {
                    let followed = cx.lib.artist_ids.contains(&h.id);
                    if pill(ui, if followed { "Following" } else { "Follow" }, !followed).clicked() {
                        cx.acts.push(Act::Follow(Kind::Artist, h.id, !followed));
                    }
                    if pill(ui, "Mix", false).clicked() {
                        cx.acts.push(Act::ArtistRadio(h.id));
                    }
                }
                Some(Kind::Album) => {
                    let on = cx.lib.album_ids.contains(&h.id);
                    if ghost_round(ui, 42.0, if on { Ic::Heart } else { Ic::HeartOutline }, if on { ACCENT } else { INK }).on_hover_text(if on { "Remove from library" } else { "Add to library" }).clicked() {
                        cx.acts.push(Act::Follow(Kind::Album, h.id, !on));
                    }
                }
                Some(Kind::Playlist) => {
                    if h.owner_id == me && me != 0 {
                        if pill(ui, "Delete playlist", false).clicked() {
                            cx.acts.push(Act::DeletePlaylist(h.id));
                        }
                    } else {
                        let on = cx.lib.playlist_ids.contains(&h.id);
                        if ghost_round(ui, 42.0, if on { Ic::Heart } else { Ic::HeartOutline }, if on { ACCENT } else { INK }).on_hover_text(if on { "Remove from library" } else { "Add to library" }).clicked() {
                            cx.acts.push(Act::Follow(Kind::Playlist, h.id, !on));
                        }
                    }
                }
                _ => {}
            }
            if let Some(k) = kind {
                if ghost_round(ui, 42.0, Ic::Link, SECOND).on_hover_text("Open on deezer.com").clicked() {
                    cx.acts.push(Act::OpenWeb(web_url(k, h.id)));
                }
            }
            if !ps.data.tracks.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(300.0, 38.0), Sense::hover());
                    ui.painter().rect(r, CornerRadius::same(19), CARD, Stroke::new(1.0, LINE), StrokeKind::Inside);
                    icons::draw(ui.painter(), Ic::Search, pos2(r.min.x + 20.0, r.center().y), 14.0, MUTED);
                    let te = TextEdit::singleline(&mut ps.filter).hint_text(RichText::new("Search within tracks").color(FAINT)).font(reg(13.5)).frame(Frame::NONE).vertical_align(Align::Center).margin(Margin::symmetric(0, 0));
                    let ir = Rect::from_min_max(pos2(r.min.x + 36.0, r.min.y), pos2(r.max.x - 14.0, r.max.y));
                    if ui.put(ir, te).changed() {
                        ps.dirty = true;
                    }
                });
            }
        });
    }
    if *route == Route::Playlists {
        ui.add_space(8.0);
        if pill(ui, "New playlist", true).clicked() {
            cx.acts.push(Act::OpenNewPlaylist);
        }
    }
    ui.add_space(18.0);
}

// ------------------------------------------------------------------ contenu de page

pub fn page_ui(ui: &mut Ui, cx: &mut Cx, ps: &mut PageState, offs: &mut HashMap<String, CarState>, route: &Route, gen: u64) {
    ui.add_space(if matches!(route, Route::Home | Route::Explore) { 14.0 } else { 26.0 });
    if matches!(route, Route::Home | Route::Explore) && ps.data.header.is_none() {
        let title = match route {
            Route::Home if !cx.lib.me_name.is_empty() => format!("Welcome back, {}", cx.lib.me_name),
            Route::Home => "Home".to_owned(),
            _ => "Explore".to_owned(),
        };
        section_title(ui, &title, 36.0);
        ui.add_space(14.0);
    }
    if *route == Route::Home {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if pill(ui, "Flow", true).clicked() {
                cx.acts.push(Act::Flow);
            }
            if pill(ui, "Favourite tracks", false).clicked() {
                cx.acts.push(Act::Go(Route::FavTracks));
            }
        });
    }
    if !ps.err.is_empty() {
        ui.add_space(20.0);
        Frame::new().fill(ACCENT.gamma_multiply(0.08)).stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.35))).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.label(RichText::new(&ps.err).color(INK).font(reg(13.5)));
        });
    }
    if ps.loading && ps.data.sections.is_empty() && ps.data.tracks.is_empty() && ps.data.header.is_none() {
        skeleton(ui);
        return;
    }
    header_ui(ui, cx, ps, route);
    if !ps.data.tracks.is_empty() {
        if matches!(ps.data.header.as_ref().and_then(|h| h.kind), Some(Kind::Artist) | None) && !ps.data.tracks_title.is_empty() {
            section_title(ui, &ps.data.tracks_title, 26.0);
            ui.add_space(8.0);
        }
        let pl = match (&ps.data.header, route) {
            (Some(h), Route::Playlist(id)) if h.owner_id == cx.lib.me_id && cx.lib.me_id != 0 => Some(*id),
            _ => None,
        };
        track_table(ui, cx, ps, pl);
        let pv = ps.data.preview;
        if pv > 0 && ps.filter.is_empty() && ps.view.len() > pv {
            ui.add_space(10.0);
            let label = if ps.expanded { "Show less".to_owned() } else { format!("Show all {} tracks", ps.view.len()) };
            if pill(ui, &label, false).clicked() {
                ps.expanded = !ps.expanded;
            }
            ui.add_space(8.0);
        }
        if ps.view.is_empty() && !ps.filter.is_empty() {
            ui.add_space(20.0);
            ui.label(RichText::new("No tracks match your search").color(MUTED));
        }
    } else if ps.data.header.is_some() && *route == Route::FavTracks && cx.lib.tracks.is_empty() {
        ui.add_space(24.0);
        ui.label(RichText::new(if cx.lib.tracks_done { "No favourite tracks yet" } else { "Loading your favourites…" }).color(MUTED));
    }
    let sections = std::mem::take(&mut ps.data.sections);
    for s in &sections {
        section_ui(ui, cx, offs, s, gen);
    }
    ps.data.sections = sections;
    if ps.loading {
        ui.add_space(16.0);
        ui.spinner();
    }
    ui.add_space(48.0);
}

/// Placeholders pulsants affichés pendant le chargement d'une page.
fn skeleton(ui: &mut Ui) {
    let t = ui.ctx().input(|i| i.time) as f32;
    let a = 0.65 + 0.35 * (t * 3.0).sin();
    let col = HOVER.gamma_multiply(a);
    ui.ctx().request_repaint_after(Duration::from_millis(33));
    ui.add_space(24.0);
    for _ in 0..2 {
        let (r, _) = ui.allocate_exact_size(vec2(220.0, 26.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(8), col);
        ui.add_space(16.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), CARD_W + 60.0), Sense::hover());
        let n = ((row.width() + 18.0) / (CARD_W + 18.0)).floor().max(1.0) as usize;
        for i in 0..n {
            let x = row.min.x + i as f32 * (CARD_W + 18.0);
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, row.min.y), vec2(CARD_W, CARD_W)), CornerRadius::same(10), col);
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, row.min.y + CARD_W + 12.0), vec2(CARD_W * 0.8, 12.0)), CornerRadius::same(6), col);
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, row.min.y + CARD_W + 32.0), vec2(CARD_W * 0.5, 10.0)), CornerRadius::same(5), col);
        }
        ui.add_space(24.0);
    }
}

// ------------------------------------------------------------------ shell : barres, panneaux

/// Marque : petit point plein couleur d'accent.
fn brand_mark(p: &Painter, c: Pos2, size: f32) {
    p.circle_filled(c, size * 0.5, acc());
}

fn nav_item(ui: &mut Ui, ic: Ic, label: &str, selected: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let id = ui.scope_id().with(("nav", label));
    let s = anim(ui, id.with("s"), selected, 0.22);
    let h = anim(ui, id.with("h"), ui.rect_contains_pointer(rect), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), CARD.gamma_multiply(h * (1.0 - s) * 1.4));
    if s > 0.01 {
        p.rect(rect, CornerRadius::same(10), HOVER.gamma_multiply(s), Stroke::new(1.0, LINE.gamma_multiply(s)), StrokeKind::Inside);
    }
    let col = SECOND.lerp_to_gamma(INK, s.max(h));
    let icol = if selected && ic == Ic::Heart { ACCENT } else { col };
    icons::draw(p, ic, pos2(rect.min.x + 21.0, rect.center().y), 16.0, icol);
    text(p, pos2(rect.min.x + 42.0, rect.center().y - 8.5), label, med(14.0), col, w - 50.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl App {
    fn drag_area(&self, ui: &mut Ui, rect: Rect, id: &str) {
        let r = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
        if r.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if r.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let App { imgs, lib, acts, route, back, fwd, .. } = self;
        Panel::left("side").exact_size(244.0).resizable(false).frame(Frame::new().inner_margin(Margin::symmetric(10, 0))).show(ui, |ui| {
            // fond : panneau sombre qui laisse filtrer un peu de lueur en haut + filet à droite
            let full = ui.max_rect().expand2(vec2(10.0, 0.0));
            let p = ui.painter();
            vgrad(p, full, &[(0.0, SIDE.gamma_multiply(0.25)), (0.5, SIDE.gamma_multiply(0.78)), (1.0, SIDE.gamma_multiply(0.9))]);
            p.vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0, LINE));

            // zone des boutons macOS + navigation
            let (top, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
            let rr = ui.interact(top, ui.scope_id().with("sidedrag"), Sense::click_and_drag());
            if rr.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            let bx = top.right() - 50.0;
            let by = top.center().y + 3.0;
            let (bl, br) = (Rect::from_center_size(pos2(bx, by), vec2(26.0, 26.0)), Rect::from_center_size(pos2(bx + 28.0, by), vec2(26.0, 26.0)));
            let (rl, rrr) = (ui.interact(bl, ui.scope_id().with("back"), Sense::click()), ui.interact(br, ui.scope_id().with("fwd"), Sense::click()));
            for (r, resp, ic, off) in [(bl, &rl, Ic::Left, back.is_empty()), (br, &rrr, Ic::Right, fwd.is_empty())] {
                if resp.hovered() && !off {
                    ui.painter().rect_filled(r, CornerRadius::same(7), HOVER);
                }
                icons::draw(ui.painter(), ic, r.center(), 14.0, if off { FAINT } else if resp.hovered() { INK } else { SECOND });
            }
            if rl.clicked() {
                acts.push(Act::Back);
            }
            if rrr.clicked() {
                acts.push(Act::Forward);
            }
            ui.add_space(6.0);
            // marque : pastille d'accent (ondes) + nom en display
            let (brand, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            brand_mark(ui.painter(), pos2(brand.min.x + 17.0, brand.center().y + 1.0), 9.0);
            title_text(ui.painter(), pos2(brand.min.x + 29.0, brand.center().y - 13.0), "Deezer", 22.0, INK, 180.0);
            ui.add_space(14.0);
            if nav_item(ui, Ic::Home, "Home", *route == Route::Home).clicked() {
                acts.push(Act::Go(Route::Home));
            }
            if nav_item(ui, Ic::Explore, "Explore", *route == Route::Explore).clicked() {
                acts.push(Act::Go(Route::Explore));
            }
            ui.add_space(18.0);
            let (lr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
            caps(ui.painter(), pos2(lr.min.x + 12.0, lr.min.y + 2.0), "LIBRARY", FAINT, 200.0);
            ui.add_space(4.0);
            for (ic, label, r) in [
                (Ic::HeartOutline, "Favourite tracks", Route::FavTracks),
                (Ic::Grid, "Playlists", Route::Playlists),
                (Ic::Disc, "Albums", Route::Albums),
                (Ic::Person, "Artists", Route::Artists),
                (Ic::Mic, "Podcasts", Route::Podcasts),
            ] {
                let sel = *route == r;
                let ic = if sel && ic == Ic::HeartOutline { Ic::Heart } else { ic };
                if nav_item(ui, ic, label, sel).clicked() {
                    acts.push(Act::Go(r));
                }
            }
            ui.add_space(18.0);
            let (qr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
            caps(ui.painter(), pos2(qr.min.x + 12.0, qr.min.y + 2.0), "QUICK ACCESS", FAINT, 200.0);
            ui.add_space(4.0);
            ScrollArea::vertical().id_salt("quick").auto_shrink([false, false]).show(ui, |ui| {
                let w = ui.available_width();
                for c in lib.artists.iter().take(40) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
                    if h > 0.01 {
                        ui.painter().rect_filled(rect, CornerRadius::same(9), CARD.gamma_multiply(h * 1.5));
                    }
                    let p = ui.painter().clone();
                    imgs.paint(&p, Rect::from_center_size(pos2(rect.min.x + 24.0, rect.center().y), vec2(32.0, 32.0)), &c.img, 96, 255);
                    text(&p, pos2(rect.min.x + 50.0, rect.center().y - 8.5), &c.title, med(14.0), SECOND.lerp_to_gamma(INK, h), w - 60.0);
                    if resp.clicked() {
                        acts.push(Act::Go(Route::Artist(c.id)));
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
                ui.add_space(12.0);
            });
        });
    }

    fn topbar(&mut self, ui: &mut Ui) {
        let mut submit = false;
        let mut avatar_menu = false;
        Panel::top("top").exact_size(62.0).resizable(false).frame(Frame::new().inner_margin(Margin::symmetric(28, 0))).show(ui, |ui| {
            let rect = ui.max_rect();
            self.drag_area(ui, rect, "topdrag");
            let sr = Rect::from_min_size(pos2(rect.min.x, rect.min.y + 13.0), vec2(440.0f32.min(rect.width() - 120.0), 38.0));
            let sid = ui.scope_id().with("search_te");
            let focused = ui.ctx().memory(|m| m.has_focus(sid));
            let fk = anim(ui, sid.with("f"), focused, 0.16);
            ui.painter().rect(sr, CornerRadius::same(19), Color32::from_rgba_premultiplied(7, 7, 9, 150).lerp_to_gamma(HOVER, fk * 0.6), Stroke::new(1.0, LINE.lerp_to_gamma(Color32::from_white_alpha(60), fk)), StrokeKind::Inside);
            icons::draw(ui.painter(), Ic::Search, pos2(sr.min.x + 21.0, sr.center().y), 15.0, MUTED.lerp_to_gamma(INK, fk));
            let te = TextEdit::singleline(&mut self.search).id(sid).hint_text(RichText::new("Search artists, tracks, podcasts").color(FAINT)).frame(Frame::NONE).font(reg(14.5)).vertical_align(Align::Center).margin(Margin::symmetric(0, 0));
            let ir = Rect::from_min_max(pos2(sr.min.x + 40.0, sr.min.y), pos2(sr.max.x - 16.0, sr.max.y));
            let resp = ui.put(ir, te);
            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            // avatar
            let ac = pos2(rect.right() - 18.0, rect.center().y);
            let ar = Rect::from_center_size(ac, vec2(34.0, 34.0));
            let aresp = ui.interact(ar, ui.scope_id().with("avatar"), Sense::click());
            let p = ui.painter().clone();
            self.imgs.paint(&p, ar, &self.lib.me_img, 96, 255);
            let ak = anim(ui, ui.scope_id().with("av_h"), aresp.hovered(), 0.14);
            p.circle_stroke(ac, 17.5 + 1.5 * ak, Stroke::new(1.0, LINE.lerp_to_gamma(Color32::from_white_alpha(90), ak)));
            if self.use_web() {
                // pastille « session complète »
                let dc = ac + vec2(12.0, 12.0);
                p.circle_filled(dc, 5.0, BG);
                p.circle_filled(dc, 3.5, acc());
            }
            egui::Popup::menu(&aresp).show(|ui| {
                ui.set_min_width(220.0);
                if !self.lib.me_name.is_empty() {
                    ui.add_space(2.0);
                    ui.label(RichText::new(&self.lib.me_name).font(med(15.0)));
                    if !self.web_offer.is_empty() {
                        ui.label(RichText::new(&self.web_offer).font(reg(12.5)).color(MUTED));
                    }
                    ui.separator();
                }
                if !self.use_web() && ui.button(RichText::new("Log in to Deezer (full tracks)").color(ACCENT)).clicked() {
                    self.acts.push(Act::WebLogin);
                    ui.close();
                }
                if ui.button("Refresh library").clicked() {
                    self.acts.push(Act::RefreshLib);
                    ui.close();
                }
                if ui.button("Settings").clicked() {
                    avatar_menu = true;
                    ui.close();
                }
                if ui.button("Log out").clicked() {
                    self.acts.push(Act::Logout);
                    ui.close();
                }
            });
            aresp.on_hover_cursor(egui::CursorIcon::PointingHand);
        });
        if avatar_menu {
            self.show_settings = true;
        }
        if submit {
            let q = self.search.trim().to_owned();
            if !q.is_empty() {
                self.acts.push(Act::Go(Route::Search(q)));
            }
        }
    }

    fn player_bar(&mut self, ui: &mut Ui) {
        let playing = self.is_playing();
        let (pos, len) = self.play_pos();
        let full = self.use_web();
        let App { imgs, lib, acts, cur, loading, shuffle, repeat, volume, seeking, show_queue, vol_before_mute, now_playing, show_lyrics, .. } = self;
        // dock : transport à gauche, capsule « en cours » centrée (pochette, titre, progression), réglages à droite
        Panel::bottom("player").exact_size(92.0).resizable(false).frame(Frame::new().inner_margin(Margin { left: 12, right: 12, top: 2, bottom: 12 })).show(ui, |ui| {
            let dock = ui.max_rect();
            let p = ui.painter().clone();
            // même matière que la barre latérale (sombre, légèrement translucide) + filet clair en haut
            p.add(Shadow { offset: [0, 14], blur: 44, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(dock, CornerRadius::same(24)));
            p.rect(dock, CornerRadius::same(24), SIDE.gamma_multiply(0.92), Stroke::new(1.0, LINE), StrokeKind::Inside);
            p.hline(dock.x_range().shrink(28.0), dock.top() + 1.0, Stroke::new(1.0, Color32::from_white_alpha(11)));
            let rect = dock.shrink2(vec2(10.0, 0.0));
            let cy = rect.center().y;
            let x0 = rect.min.x;
            let rx = rect.right() - 12.0;

            // ---- gauche : transport
            let btn = |ui: &mut Ui, x: f32, ic: Ic, size: f32, active: bool, id: &str| -> bool {
                let r = Rect::from_center_size(pos2(x, cy), vec2(34.0, 34.0));
                let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
                let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
                let press = if resp.is_pointer_button_down_on() { 0.9 } else { 1.0 };
                if h > 0.01 {
                    ui.painter().circle_filled(r.center(), 16.0, Color32::from_white_alpha((10.0 * h) as u8));
                }
                icons::draw(ui.painter(), ic, r.center(), size * press, if active { INK } else { MUTED.lerp_to_gamma(INK, h) });
                if active {
                    ui.painter().circle_filled(pos2(x, cy + 15.0), 1.7, acc_text());
                }
                resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            };
            if btn(ui, x0 + 17.0, Ic::Shuffle, 15.0, *shuffle, "b_shuf") {
                acts.push(Act::Shuffle);
            }
            if btn(ui, x0 + 58.0, Ic::Prev, 17.0, false, "b_prev") {
                acts.push(Act::Prev);
            }
            let pc = pos2(x0 + 104.0, cy);
            let presp = ui.interact(Rect::from_center_size(pc, vec2(44.0, 44.0)), ui.scope_id().with("b_play"), Sense::click());
            let ph = anim(ui, ui.scope_id().with("ph"), presp.hovered(), 0.14);
            let press = if presp.is_pointer_button_down_on() { 0.92 } else { 1.0 };
            let pr_ = (20.0 + 1.0 * ph) * press;
            p.circle_filled(pc + vec2(0.0, 2.0), pr_, Color32::from_black_alpha(70));
            p.circle_filled(pc, pr_, INK.lerp_to_gamma(Color32::WHITE, ph));
            if *loading {
                ui.put(Rect::from_center_size(pc, vec2(16.0, 16.0)), egui::Spinner::new().color(BG));
            } else {
                icons::draw(&p, if playing { Ic::Pause } else { Ic::Play }, pc + vec2(if playing { 0.0 } else { 1.3 }, 0.0), 15.0 * press, BG);
            }
            if presp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                acts.push(Act::Toggle);
            }
            if btn(ui, x0 + 150.0, Ic::Next, 17.0, false, "b_next") {
                acts.push(Act::Next);
            }
            let rep_ic = if *repeat == Repeat::One { Ic::RepeatOne } else { Ic::Repeat };
            if btn(ui, x0 + 191.0, rep_ic, 15.0, *repeat != Repeat::Off, "b_rep") {
                acts.push(Act::Repeat);
            }

            // ---- centre : capsule
            let cx_ = rect.center().x;
            let hw = (cx_ - (x0 + 228.0)).min(rx - 214.0 - cx_).min(330.0).max(150.0);
            // capsule « creusée », même traitement que le champ de recherche ; rayons concentriques
            let cap = Rect::from_center_size(pos2(cx_, cy), vec2(hw * 2.0, 58.0));
            let cresp = ui.interact(cap, ui.scope_id().with("pl_cap"), Sense::click());
            let ch = anim(ui, cresp.id.with("h"), cresp.contains_pointer(), 0.16);
            p.rect(cap, CornerRadius::same(18), Color32::from_rgba_premultiplied(5, 5, 7, 165).lerp_to_gamma(HOVER, ch * 0.5), Stroke::new(1.0, LINE.lerp_to_gamma(Color32::from_white_alpha(34), ch)), StrokeKind::Inside);
            if let Some(t) = cur.as_ref() {
                let img = Rect::from_min_size(pos2(cap.min.x + 6.0, cap.min.y + 6.0), vec2(46.0, 46.0));
                shadow(&p, img, 12, 0.6);
                imgs.paint(&p, img, &t.img, 120, 12);
                let ah = anim(ui, ui.scope_id().with("pl_arth"), cresp.hovered(), 0.14);
                if ah > 0.01 {
                    p.rect_filled(img, CornerRadius::same(12), Color32::from_black_alpha((120.0 * ah) as u8));
                    let c = img.center();
                    let d = if *now_playing { 1.0 } else { -1.0 };
                    p.add(egui::epaint::PathShape::line(vec![c + vec2(-6.5, -3.2 * d), c + vec2(0.0, 3.2 * d), c + vec2(6.5, -3.2 * d)], Stroke::new(2.0, INK.gamma_multiply(ah))));
                }
                if cresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    acts.push(Act::NowPlaying(!*now_playing));
                }
                // zone texte : entre la pochette et le cœur, contenu centré
                let tl = img.right() + 12.0;
                let tr_ = cap.right() - 66.0;
                let tcx = (tl + tr_) / 2.0;
                let tw = (tr_ - tl).max(40.0);
                let tg = galley(&p, &t.title, med(13.5), INK, tw - if t.explicit { 22.0 } else { 0.0 });
                let sub = if full { t.artist.clone() } else { format!("{} · 30 s preview", t.artist) };
                let ag = galley(&p, &sub, reg(12.0), MUTED, tw);
                let tot_w = tg.size().x + if t.explicit { 22.0 } else { 0.0 };
                let tpos = pos2(tcx - tot_w / 2.0, cap.min.y + 9.0);
                if t.explicit {
                    explicit_badge(&p, pos2(tpos.x + tg.size().x + 6.0, tpos.y + 2.0));
                }
                p.galley(tpos, tg, INK);
                let ar = Rect::from_min_size(pos2(tcx - ag.size().x / 2.0, cap.min.y + 27.0), ag.size());
                p.galley(ar.min, ag, MUTED);
                let aresp = ui.interact(ar.expand(1.0), ui.scope_id().with("pl_artist"), Sense::click());
                if aresp.hovered() {
                    p.hline(ar.x_range(), ar.bottom(), Stroke::new(1.0, MUTED));
                }
                if aresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && t.artist_id != 0 {
                    acts.push(Act::Go(Route::Artist(t.artist_id)));
                }

                // progression : filet de 3 px au pied de la capsule, temps au survol
                let bar = Rect::from_min_size(pos2(tl, cap.max.y - 9.0), vec2(cap.right() - 14.0 - tl, 3.0));
                let sresp = ui.interact(bar.expand2(vec2(4.0, 6.0)), ui.scope_id().with("seek"), Sense::click_and_drag());
                let total = len.as_secs_f32().max(0.001);
                let frac_now = (pos.as_secs_f32() / total).clamp(0.0, 1.0);
                let ptr = sresp.hover_pos().or(sresp.interact_pointer_pos()).map(|q| ((q.x - bar.min.x) / bar.width()).clamp(0.0, 1.0));
                if (sresp.dragged() || sresp.clicked()) && ptr.is_some() {
                    *seeking = ptr;
                }
                let shown = seeking.unwrap_or(frac_now);
                let kn = anim(ui, ui.scope_id().with("kn"), sresp.hovered() || sresp.dragged(), 0.12);
                // remplissage à la teinte de la pochette (la même que l'aurora), sans dégradé
                slider(&p, bar, shown, kn, acc_text().lerp_to_gamma(INK, 0.25 * (1.0 - kn)));
                if sresp.drag_stopped() || sresp.clicked() {
                    if let Some(f) = seeking.take() {
                        acts.push(Act::Seek(f));
                    }
                }
                if ch > 0.01 && kn < 0.99 {
                    let a = ch * (1.0 - kn);
                    let ts = (shown * total) as u32;
                    let tt = len.as_secs() as u32;
                    let g = galley(&p, &format!("{}:{:02}", ts / 60, ts % 60), reg(10.5), FAINT, 50.0);
                    p.galley(pos2(bar.min.x, bar.min.y - 4.0 - g.size().y), g, FAINT.gamma_multiply(a));
                    let g = galley(&p, &format!("{}:{:02}", tt / 60, tt % 60), reg(10.5), FAINT, 50.0);
                    p.galley(pos2(bar.max.x - g.size().x, bar.min.y - 4.0 - g.size().y), g, FAINT.gamma_multiply(a));
                }
                if let (Some(f), true) = (ptr, kn > 0.05) {
                    let s_ = (f * total) as u32;
                    let g = galley(&p, &format!("{}:{:02}", s_ / 60, s_ % 60), med(11.0), INK, 60.0);
                    let bx = (bar.min.x + bar.width() * f).clamp(bar.min.x + 18.0, bar.max.x - 18.0);
                    let bub = Rect::from_center_size(pos2(bx, bar.min.y - 13.0), g.size() + vec2(14.0, 6.0));
                    p.rect(bub, CornerRadius::same(7), RAISED.gamma_multiply(kn), Stroke::new(1.0, LINE.gamma_multiply(kn)), StrokeKind::Inside);
                    p.galley(bub.center() - g.size() / 2.0, g, INK.gamma_multiply(kn));
                }
                sresp.on_hover_cursor(egui::CursorIcon::PointingHand);

                // cœur (toujours) et « + » (au survol), à droite dans la capsule
                let liked = lib.fav.contains(&t.id);
                let hr = Rect::from_center_size(pos2(cap.right() - 24.0, cap.min.y + 22.0), vec2(28.0, 28.0));
                let hresp = ui.interact(hr, ui.scope_id().with("pl_heart"), Sense::click());
                let pop = heart_pop(ui, ui.scope_id().with("pl_pop"), liked);
                if hresp.hovered() {
                    p.circle_filled(hr.center(), 14.0, Color32::from_white_alpha(12));
                }
                icons::draw(&p, if liked { Ic::Heart } else { Ic::HeartOutline }, hr.center(), 15.5 * pop, if liked { ACCENT } else if hresp.hovered() { INK } else { MUTED });
                if hresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    acts.push(Act::Like(t.clone()));
                }
                let pr = Rect::from_center_size(pos2(cap.right() - 52.0, cap.min.y + 22.0), vec2(26.0, 26.0));
                let presp = ui.interact(pr, ui.scope_id().with("pl_plus"), Sense::click());
                let open_menu = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&presp));
                let pa = if open_menu { 1.0 } else { ch };
                if pa > 0.01 {
                    if presp.hovered() {
                        p.circle_filled(pr.center(), 13.0, Color32::from_white_alpha(12));
                    }
                    icons::draw(&p, Ic::Plus, pr.center(), 14.0, (if presp.hovered() { INK } else { MUTED }).gamma_multiply(pa));
                }
                let t2 = t.clone();
                egui::Popup::menu(&presp).show(|ui| {
                    ui.set_min_width(210.0);
                    let me = lib.me_id;
                    ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for pl in lib.playlists.iter().filter(|c| me == 0 || c.owner == me) {
                            if ui.button(&pl.title).clicked() {
                                acts.push(Act::AddToPlaylist { pl: pl.id, track: t2.id });
                                ui.close();
                            }
                        }
                    });
                });
            } else {
                text_center(&p, cx_, cy - 8.0, "Nothing playing", reg(13.0), FAINT, hw * 2.0);
            }

            // ---- droite : paroles, file d'attente | volume
            let toggle = |ui: &mut Ui, x: f32, ic: Ic, on: bool, id: &str| -> bool {
                let r = Rect::from_center_size(pos2(x, cy), vec2(34.0, 34.0));
                let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
                let h = anim(ui, resp.id.with("h"), resp.hovered(), 0.12);
                let k = anim(ui, resp.id.with("on"), on, 0.18);
                let bg = 10.0 * h + 10.0 * k;
                if bg > 0.3 {
                    ui.painter().circle_filled(r.center(), 16.0, Color32::from_white_alpha(bg as u8));
                }
                icons::draw(ui.painter(), ic, r.center(), 16.0, MUTED.lerp_to_gamma(INK, h.max(k)));
                resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            };
            if toggle(ui, rx, Ic::Queue, *show_queue, "b_queue") {
                *show_queue = !*show_queue;
            }
            if toggle(ui, rx - 40.0, Ic::Lyrics, *now_playing && *show_lyrics, "b_lyrics") && cur.is_some() {
                acts.push(Act::ToggleLyrics);
            }
            p.vline(rx - 63.0, egui::Rangef::new(cy - 10.0, cy + 10.0), Stroke::new(1.0, LINE));
            let vw = 84.0;
            let vbar = Rect::from_min_size(pos2(rx - 80.0 - vw, cy - 1.5), vec2(vw, 3.0));
            let vresp = ui.interact(vbar.expand2(vec2(4.0, 11.0)), ui.scope_id().with("vol"), Sense::click_and_drag());
            if let Some(px) = vresp.interact_pointer_pos() {
                if vresp.dragged() || vresp.clicked() {
                    acts.push(Act::Volume(((px.x - vbar.min.x) / vbar.width()).clamp(0.0, 1.0)));
                }
            }
            if vresp.hovered() {
                let d = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
                if d != 0.0 {
                    acts.push(Act::Volume((*volume + d / 600.0).clamp(0.0, 1.0)));
                }
            }
            let vk = anim(ui, ui.scope_id().with("vk"), vresp.hovered() || vresp.dragged(), 0.12);
            slider(&p, vbar, *volume, vk, INK);
            let mr = Rect::from_center_size(pos2(vbar.min.x - 20.0, cy), vec2(30.0, 30.0));
            let mresp = ui.interact(mr, ui.scope_id().with("mute"), Sense::click());
            let ic = if *volume <= 0.001 { Ic::Mute } else { Ic::Volume };
            icons::draw(&p, ic, mr.center(), 16.0, if mresp.hovered() { INK } else { MUTED });
            if mresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let v = if *volume > 0.001 { 0.0 } else { *vol_before_mute };
                acts.push(Act::Volume(v));
            }
        });
    }

    fn queue_panel(&mut self, ui: &mut Ui) {
        let k = ease(ui.ctx().animate_bool_with_time(egui::Id::new("qslide"), self.show_queue, 0.26));
        if k < 0.005 {
            return;
        }
        let playing = self.is_playing();
        let live = self.live;
        let t_now = ui.ctx().input(|i| i.time) as f32;
        let App { imgs, acts, queue, qi, .. } = self;
        Panel::right("queue").exact_size(330.0 * k).resizable(false).frame(Frame::new().fill(Color32::from_rgba_premultiplied(12, 12, 16, 238))).show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().vline(r.left() + 0.5, r.y_range(), Stroke::new(1.0, LINE));
            let full = Rect::from_min_size(pos2(r.right() - 330.0, r.top()), vec2(330.0, r.height()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(full.shrink2(vec2(16.0, 14.0))), |ui| {
                ui.horizontal(|ui| {
                    section_title(ui, "Queue", 24.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !queue.is_empty() && pill(ui, "Clear", false).clicked() {
                            acts.push(Act::ClearQueue);
                        }
                    });
                });
                ui.add_space(10.0);
                if queue.is_empty() {
                    ui.label(RichText::new("Nothing queued").color(MUTED));
                    return;
                }
                let rh = 54.0;
                ScrollArea::vertical().id_salt("qscroll").auto_shrink([false, false]).show_rows(ui, rh, queue.len(), |ui, range| {
                    let w = ui.available_width();
                    for i in range {
                        let t = &queue[i];
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, rh), Sense::click());
                        let cur = i == *qi;
                        let row = rect.shrink2(vec2(0.0, 2.0));
                        if cur {
                            ui.painter().rect(row, CornerRadius::same(10), HOVER, Stroke::new(1.0, LINE), StrokeKind::Inside);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(row, CornerRadius::same(10), CARD);
                        }
                        let p = ui.painter().clone();
                        let img = Rect::from_center_size(pos2(rect.min.x + 26.0, rect.center().y), vec2(38.0, 38.0));
                        imgs.paint(&p, img, &t.img, 96, 6);
                        if cur {
                            p.rect_filled(img, CornerRadius::same(6), Color32::from_black_alpha(130));
                            eq_bars(&p, img.center(), 14.0, t_now, live && playing, acc_text());
                        }
                        text(&p, pos2(rect.min.x + 56.0, rect.center().y - 17.0), &t.title, med(13.5), if cur { acc_text() } else { INK }, w - 116.0);
                        text(&p, pos2(rect.min.x + 56.0, rect.center().y + 2.0), &t.artist, reg(12.0), MUTED, w - 116.0);
                        text(&p, pos2(rect.right() - 46.0, rect.center().y - 7.0), &fmt_dur(t.dur), mono(11.5), FAINT, 44.0);
                        if resp.clicked() {
                            acts.push(Act::JumpQueue(i));
                        }
                        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    }
                });
            });
        });
    }

    fn central(&mut self, ui: &mut Ui) {
        let playing = self.is_playing();
        let live = self.live;
        let t_now = ui.ctx().input(|i| i.time) as f32;
        let gen = self.pool.gen.load(std::sync::atomic::Ordering::Relaxed);
        let t = (self.page_t0.elapsed().as_secs_f32() / 0.32).min(1.0);
        let fade = 1.0 - (1.0 - t).powi(3);
        if t < 1.0 {
            self.ctx.request_repaint();
        }
        let App { imgs, lib, acts, page, carousel, cur, shuffle, route, .. } = self;
        egui::CentralPanel::default().frame(Frame::new().inner_margin(Margin { left: 30, right: 22, top: 0, bottom: 0 })).show(ui, |ui| {
            let mut cx = Cx { imgs, lib, acts, cur_id: cur.as_ref().map(|t| t.id).unwrap_or(0), playing, shuffle: *shuffle, live, t: t_now };
            ScrollArea::vertical().id_salt(("page", gen)).auto_shrink([false, false]).show(ui, |ui| {
                ui.multiply_opacity(fade);
                ui.add_space((1.0 - fade) * 16.0);
                page_ui(ui, &mut cx, page, carousel, route, gen);
            });
        });
    }

    pub fn main_ui(&mut self, ui: &mut Ui) {
        self.player_bar(ui);
        if self.now_playing && self.cur.is_some() {
            self.now_playing_ui(ui);
            return;
        }
        self.sidebar(ui);
        self.topbar(ui);
        self.queue_panel(ui);
        self.central(ui);
    }

    // -------------------------------------------------------------- Now Playing (plein écran)

    fn now_playing_ui(&mut self, ui: &mut Ui) {
        let (pos, len) = self.play_pos();
        let k = ease((self.np_t0.elapsed().as_secs_f32() / 0.38).min(1.0));
        if k < 1.0 {
            ui.ctx().request_repaint();
        }
        let Some(t) = self.cur.clone() else { return };
        let liked = self.lib.fav.contains(&t.id);
        let lyr_on = self.show_lyrics;
        egui::CentralPanel::default().frame(Frame::new()).show(ui, |ui| {
            let full = ui.max_rect();
            ui.multiply_opacity(k);
            let top = Rect::from_min_size(full.min, vec2(full.width(), 58.0));
            self.drag_area(ui, top, "npdrag");
            let p = ui.painter().clone();
            // fermer
            // fermer : grand bouton rond « verre », chevron épais, touche Esc rappelée au survol
            let cb = Rect::from_center_size(pos2(full.min.x + 112.0, full.min.y + 30.0), vec2(44.0, 44.0));
            let cr = ui.interact(cb, ui.scope_id().with("np_close"), Sense::click());
            let ch = anim(ui, cr.id.with("h"), cr.hovered(), 0.14);
            let press = if cr.is_pointer_button_down_on() { 0.94 } else { 1.0 };
            let rad = (21.0 + 1.0 * ch) * press;
            p.circle(cb.center(), rad, Color32::from_white_alpha((14.0 + 14.0 * ch) as u8), Stroke::new(1.0, Color32::from_white_alpha((16.0 + 14.0 * ch) as u8)));
            let c = cb.center() + vec2(0.0, 1.0 + 1.5 * ch);
            let s = 7.0 * press;
            p.add(egui::epaint::PathShape::line(vec![c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)], Stroke::new(2.4, INK)));
            if ch > 0.01 {
                let kg = galley(&p, "esc", med(10.5), MUTED, 40.0);
                let kr = Rect::from_min_size(pos2(cb.max.x + 10.0, cb.center().y - 10.0), vec2(kg.size().x + 12.0, 20.0));
                p.rect(kr, CornerRadius::same(6), Color32::TRANSPARENT, Stroke::new(1.0, Color32::from_white_alpha((28.0 * ch) as u8)), StrokeKind::Inside);
                p.galley(kr.center() - kg.size() / 2.0, kg, MUTED.gamma_multiply(ch));
            }
            if cr.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.acts.push(Act::NowPlaying(false));
            }
            let label = Rect::from_center_size(pos2(full.center().x, full.min.y + 28.0), vec2(200.0, 14.0));
            let g = galley_ls(&p, "NOW PLAYING", bold(10.5), MUTED, 200.0, 0.9);
            p.galley(pos2(label.center().x - g.size().x / 2.0, label.min.y), g, MUTED);
            // bouton paroles (haut droite)
            let lb = Rect::from_min_size(pos2(full.max.x - 128.0, full.min.y + 11.0), vec2(104.0, 34.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(lb), |ui| {
                if pill(ui, "Lyrics", lyr_on).clicked() {
                    self.acts.push(Act::ToggleLyrics);
                }
            });

            let body = Rect::from_min_max(pos2(full.min.x + 48.0, full.min.y + 70.0), pos2(full.max.x - 48.0, full.max.y - 18.0));
            let (left, right) = if lyr_on {
                let split = body.min.x + body.width() * 0.46;
                (Rect::from_min_max(body.min, pos2(split - 24.0, body.max.y)), Some(Rect::from_min_max(pos2(split + 24.0, body.min.y), body.max)))
            } else {
                (body, None)
            };
            // pochette en grand + titre
            let cs = (left.width() * 0.82).min(left.height() - 150.0).clamp(160.0, 520.0);
            let block = cs + 128.0;
            let y0 = left.center().y - block / 2.0 + (1.0 - k) * 18.0;
            let scale = 0.95 + 0.05 * k;
            let art = Rect::from_center_size(pos2(left.center().x, y0 + cs / 2.0), vec2(cs * scale, cs * scale));
            p.add(Shadow { offset: [0, 28], blur: 90, spread: 0, color: Color32::from_black_alpha(200) }.as_shape(art, CornerRadius::same(18)));
            self.imgs.paint(&p, art, &t.img, 500, 18);
            let tw = left.width().min(cs + 160.0);
            let n = t.title.chars().count();
            let ts = if n > 40 { 24.0 } else if n > 22 { 28.0 } else { 34.0 };
            let ty = y0 + cs + 26.0;
            let tg = galley_ls(&p, &t.title, display(ts), INK, tw, -ts * 0.02);
            p.galley(pos2(left.center().x - tg.size().x / 2.0, ty), tg, INK);
            let ag = galley(&p, &t.artist, med(17.0), SECOND, tw);
            let ar = Rect::from_min_size(pos2(left.center().x - ag.size().x / 2.0, ty + ts * 1.25 + 6.0), ag.size());
            p.galley(ar.min, ag, SECOND);
            let aresp = ui.interact(ar.expand(3.0), ui.scope_id().with("np_artist"), Sense::click());
            if aresp.hovered() && t.artist_id != 0 {
                p.hline(ar.x_range(), ar.bottom(), Stroke::new(1.0, SECOND));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if aresp.clicked() && t.artist_id != 0 {
                self.acts.push(Act::NowPlaying(false));
                self.acts.push(Act::Go(Route::Artist(t.artist_id)));
            }
            let mut y = ar.bottom() + 6.0;
            if !t.album.is_empty() {
                let bg = galley(&p, &t.album, reg(14.0), MUTED, tw);
                let br = Rect::from_min_size(pos2(left.center().x - bg.size().x / 2.0, y), bg.size());
                p.galley(br.min, bg, MUTED);
                let bresp = ui.interact(br.expand(3.0), ui.scope_id().with("np_album"), Sense::click());
                if bresp.hovered() && t.album_id != 0 {
                    p.hline(br.x_range(), br.bottom(), Stroke::new(1.0, MUTED));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if bresp.clicked() && t.album_id != 0 {
                    self.acts.push(Act::NowPlaying(false));
                    self.acts.push(Act::Go(Route::Album(t.album_id)));
                }
                y = br.bottom();
            }
            // coup de cœur
            let hb = Rect::from_center_size(pos2(left.center().x, y + 30.0), vec2(40.0, 40.0));
            let hresp = ui.interact(hb, ui.scope_id().with("np_heart"), Sense::click());
            let pop = heart_pop(ui, ui.scope_id().with("np_pop"), liked);
            if hresp.hovered() {
                p.circle_filled(hb.center(), 19.0, HOVER);
            }
            icons::draw(&p, if liked { Ic::Heart } else { Ic::HeartOutline }, hb.center(), 19.0 * pop, if liked { ACCENT } else if hresp.hovered() { INK } else { SECOND });
            if hresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.acts.push(Act::Like(t.clone()));
            }
            if let Some(r) = right {
                self.lyrics_panel(ui, r, pos, len, t.id);
            }
        });
    }

    fn lyrics_panel(&mut self, ui: &mut Ui, r: Rect, pos: Duration, len: Duration, id: u64) {
        use crate::lyrics::Lyrics;
        let playing = self.is_playing();
        let App { lyrics, ly, acts, .. } = self;
        let p = ui.painter().with_clip_rect(r.expand2(vec2(12.0, 0.0)));
        let msg = |p: &Painter, title: &str, sub: &str| {
            let g = galley(p, title, display(22.0), SECOND, r.width());
            p.galley(pos2(r.min.x, r.center().y - 30.0), g, SECOND);
            if !sub.is_empty() {
                text(p, pos2(r.min.x, r.center().y + 4.0), sub, reg(13.5), MUTED, r.width());
            }
        };
        let st = match lyrics {
            Some((lid, l)) if *lid == id => l,
            _ => &Lyrics::Loading,
        };
        let (synced, src): (bool, Vec<(u32, &str)>) = match st {
            Lyrics::Loading => {
                ui.put(Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(22.0, 22.0)), egui::Spinner::new().color(MUTED));
                return;
            }
            Lyrics::None => return msg(&p, "No lyrics for this track", ""),
            Lyrics::NeedLogin => return msg(&p, "Lyrics need a Deezer login", "Avatar → Log in to Deezer (full tracks)"),
            Lyrics::Err(e) => return msg(&p, "Couldn't load lyrics", e),
            Lyrics::Synced(v) => (true, v.iter().map(|(t, l)| (*t, l.as_str())).collect()),
            Lyrics::Plain(v) => (false, v.iter().map(|l| (u32::MAX, l.as_str())).collect()),
        };
        let now = Instant::now();
        let w = r.width().min(620.0);
        // mise en page une seule fois par morceau / largeur (galleys gardés en cache)
        let key = (id, w as u32, src.len() * 2 + synced as usize);
        if ly.key != key {
            ly.reset();
            ly.key = key;
            ly.synced = synced;
            // ligne en cours en grand, les autres plus petites ; la petite version est mise en page
            // à la largeur réduite pour que le zoom de transition retombe exactement sur elle
            let font = bold(if synced { LY_SMALL } else { 21.0 });
            let bigf = bold(LY_BIG);
            let ratio = LY_SMALL / LY_BIG;
            let gap = if synced { 16.0 } else { 8.0 };
            ly.gap = gap;
            let mut items: Vec<(u32, &str)> = Vec::with_capacity(src.len() + 1);
            if synced && src.first().is_some_and(|l| l.0 > 2500) {
                items.push((0, "")); // intro instrumentale : points d'attente
            }
            items.extend(src.iter().copied());
            let mut y = 0.0;
            for (t, l) in items {
                let sw = if synced { w * ratio } else { w };
                let g = (!l.is_empty()).then(|| p.layout(l.to_owned(), font.clone(), Color32::WHITE, sw));
                let b = (synced && !l.is_empty()).then(|| p.layout(l.to_owned(), bigf.clone(), Color32::WHITE, w));
                let h = g.as_ref().map(|g| g.size().y).unwrap_or(if synced { 26.0 } else { 6.0 });
                ly.ys.push(y);
                ly.hs.push(h);
                ly.hb.push(b.as_ref().map(|b| b.size().y).unwrap_or(h));
                ly.big.push(b);
                ly.starts.push(t);
                ly.galleys.push(g);
                y += h + gap;
            }
            ly.total = y;
        }
        let n = ly.galleys.len();
        let pos_ms = pos.as_millis() as u32 + 250; // un poil d'avance : le mouvement tombe sur la voix
        let cur = if synced { crate::lyrics::current(&ly.starts.iter().map(|t| (*t, String::new())).collect::<Vec<_>>(), pos_ms) } else { None };
        let max_off = (ly.total - r.height() * 0.5).max(0.0);
        let lo = -r.height() * 0.3;
        let auto = match cur {
            Some(i) => ly.ys[i] + ly.hb[i] / 2.0 - r.height() * 0.36,
            None if synced => -r.height() * 0.12,
            None => 0.0,
        }
        .clamp(lo, max_off);
        if ly.t0.is_none() && ly.from.is_empty() {
            ly.target = auto;
            ly.from = vec![auto; n];
        }
        // molette : défilement libre ; le suivi reprend tout seul après 3,5 s
        if ui.rect_contains_pointer(r) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                let t = (ly.target - d).clamp(lo, max_off);
                ly.retarget(t, false, 0, 0.32, now);
                ly.manual = Some(now);
            }
        }
        if synced && ly.manual.is_some_and(|m| now.duration_since(m) > Duration::from_millis(3500)) {
            ly.manual = None;
            ly.cur = None;
        }
        if ly.manual.is_none() && (cur != ly.cur || (auto - ly.target).abs() > 1.0) {
            // changement de ligne : cascade (les lignes du dessous suivent avec un léger retard)
            let stagger = cur != ly.cur && ly.cur.is_some();
            ly.retarget(auto, stagger, cur.unwrap_or(0), 0.9, now);
            ly.cur = cur;
        }
        let manual = ly.manual.is_some();
        let t_now = ui.input(|i| i.time) as f32;
        let len_ms = len.as_millis().max(1) as f32;
        let mut dots_live = false;
        let mut grow = 0.0; // hauteur ajoutée par les lignes agrandies au-dessus
        for i in 0..n {
            let on = if synced { anim(ui, egui::Id::new(("ly", id, i)), cur == Some(i), 0.42) } else { 0.0 };
            let top = r.min.y + ly.ys[i] + grow - ly.value(i, now);
            let h = ly.hs[i] + (ly.hb[i] - ly.hs[i]) * on;
            grow += h - ly.hs[i];
            if top + h < r.min.y - 20.0 || top > r.max.y + 20.0 {
                continue;
            }
            let edge = ((top + h - r.min.y) / 80.0).min((r.max.y - top) / 130.0).clamp(0.0, 1.0);
            let edge = edge * edge * (3.0 - 2.0 * edge);
            let Some(g) = ly.galleys[i].clone() else {
                if !synced {
                    continue;
                }
                // interlude : trois points qui se remplissent jusqu'à la ligne suivante
                let start = ly.starts[i] as f32;
                let next = ly.starts.get(i + 1).map(|t| *t as f32).unwrap_or(len_ms);
                let prog = if cur == Some(i) { ((pos_ms as f32 - start) / (next - start).max(1.0)).clamp(0.0, 1.0) } else { 0.0 };
                let breathe = if cur == Some(i) { dots_live = true; 1.0 + 0.1 * (t_now * 3.2).sin() * on } else { 1.0 };
                for k in 0..3 {
                    let lit = (prog * 3.0 - k as f32).clamp(0.0, 1.0);
                    let a = (0.22 + 0.78 * lit * on).max(if manual { 0.45 } else { 0.0 }) * edge;
                    p.circle_filled(pos2(r.min.x + 7.0 + k as f32 * 20.0, top + h / 2.0), 5.5 * breathe, INK.gamma_multiply(a));
                }
                continue;
            };
            let big = ly.big[i].clone();
            let sc = LY_SMALL / LY_BIG + (1.0 - LY_SMALL / LY_BIG) * on;
            let gw = big.as_ref().map(|b| b.size().x * sc).unwrap_or(g.size().x);
            let lr = Rect::from_min_size(pos2(r.min.x, top), vec2(gw.max(40.0), h));
            let hot = synced && ui.rect_contains_pointer(lr);
            let base = if !synced { 0.82 } else if manual { 0.5 } else { 0.3 };
            let a = ((base + (1.0 - base) * on) + if hot { 0.22 } else { 0.0 }).min(1.0) * edge;
            let col = INK.gamma_multiply(a);
            match big {
                // repos : galley à sa taille native (net) ; transition : grande version mise à l'échelle
                Some(b) if on > 0.995 => p.galley_with_override_text_color(lr.min, b, col),
                Some(b) if on > 0.005 => {
                    let mut ts = egui::epaint::TextShape::new(Pos2::ZERO, b, col).with_override_text_color(col);
                    ts.transform(egui::emath::TSTransform::new(lr.min.to_vec2(), sc));
                    p.add(ts);
                }
                _ => p.galley_with_override_text_color(lr.min, g, col),
            }
            if hot {
                let resp = ui.interact(lr, ui.scope_id().with(("lyl", i)), Sense::click());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    acts.push(Act::Seek((ly.starts[i] as f32 / len_ms).clamp(0.0, 1.0)));
                    ly.manual = None;
                }
            }
        }
        // repaint : plein régime pendant un mouvement, sinon réveil pile à la ligne suivante
        let ctx = ui.ctx();
        if ly.moving(now) {
            ctx.request_repaint();
        } else if dots_live && playing {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if synced && playing {
            if let Some(nx) = ly.starts.iter().find(|t| **t > pos_ms && **t != u32::MAX) {
                ctx.request_repaint_after(Duration::from_millis((nx - pos_ms) as u64 + 5));
            }
        }
    }

    // -------------------------------------------------------------- écran de connexion

    pub fn login_screen(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().frame(Frame::new()).show(ui, |ui| {
            let r = ui.max_rect();
            self.drag_area(ui, Rect::from_min_size(r.min, vec2(r.width(), 40.0)), "logindrag");
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((r.height() * 0.5 - 270.0).max(48.0));
                    ui.label(RichText::new("Deezer").font(display(58.0)).extra_letter_spacing(-1.8).color(INK));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Your music, nothing else.").font(reg(15.0)).color(SECOND));
                    ui.add_space(28.0);
                    let glass = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
                        Frame::new()
                            .fill(Color32::from_rgba_premultiplied(17, 17, 22, 215))
                            .stroke(Stroke::new(1.0, LINE))
                            .corner_radius(CornerRadius::same(18))
                            .shadow(Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(140) })
                            .inner_margin(Margin::same(24))
                            .show(ui, |ui| {
                                ui.set_width(500.0);
                                ui.with_layout(Layout::top_down(Align::Min), |ui| add(ui));
                            });
                    };
                    glass(ui, &mut |ui| {
                        ui.label(RichText::new("EMAIL + PASSWORD").font(bold(10.5)).extra_letter_spacing(0.9).color(MUTED));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Full tracks, your library and your playlists").font(display(20.0)).extra_letter_spacing(-0.4));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Sign in on Deezer's own page, right inside the app (email, Google, Apple…). Uses your subscription. The app never sees your password.").color(MUTED).font(reg(13.0)));
                        ui.add_space(14.0);
                        if pill(ui, "Log in to Deezer", true).clicked() {
                            self.acts.push(Act::WebLogin);
                        }
                    });
                    ui.add_space(14.0);
                    glass(ui, &mut |ui| {
                        ui.label(RichText::new("PUBLIC PROFILE").font(bold(10.5)).extra_letter_spacing(0.9).color(MUTED));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Browse without signing in (30 s previews)").font(med(15.0)));
                        ui.label(RichText::new("Paste your profile link (deezer.com → your avatar → Profile) or your numeric ID. Your profile and favourites must be public.").color(MUTED).font(reg(13.0)));
                        ui.add_space(10.0);
                        if !self.clip_checked {
                            self.clip_checked = true;
                            let c = crate::app::clipboard_text();
                            if c.contains("deezer.com") && c.contains("profile") {
                                self.guest_input = c;
                            }
                        }
                        let r = ui.add(TextEdit::singleline(&mut self.guest_input).hint_text("https://www.deezer.com/profile/123456789").desired_width(452.0).margin(Margin::symmetric(10, 8)));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if pill(ui, "Paste & continue", false).clicked() {
                                self.acts.push(Act::PasteLogin);
                            }
                            if pill(ui, "Continue", false).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                                self.acts.push(Act::GuestLogin);
                            }
                        });
                        if !self.login_err.is_empty() {
                            ui.add_space(8.0);
                            ui.colored_label(ACCENT, &self.login_err);
                        }
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(4.0);
                        egui::CollapsingHeader::new(RichText::new("I have a Deezer developer app (OAuth)").color(MUTED).font(reg(13.0))).show(ui, |ui| {
                            ui.label(RichText::new("1. Create an app on developers.deezer.com/myapps").font(med(13.5)));
                            ui.label(RichText::new("   Redirect URL after authentication (copy it exactly):").color(MUTED).font(reg(12.5)));
                            let ru = self.cfg.redirect_uri();
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                ui.label(RichText::new(&ru).font(mono(12.5)).color(SECOND));
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(ru.clone());
                                }
                                if ui.small_button("Open myapps").clicked() {
                                    let _ = open::that("https://developers.deezer.com/myapps");
                                }
                            });
                            ui.add_space(12.0);
                            ui.label(RichText::new("2. Paste your Application ID (the secret key is optional)").font(med(13.5)));
                            ui.add_space(4.0);
                            egui::Grid::new("creds").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                                ui.label("Application ID");
                                ui.add(TextEdit::singleline(&mut self.cfg.app_id).desired_width(300.0));
                                ui.end_row();
                                ui.label("Secret key (optional)");
                                ui.add(TextEdit::singleline(&mut self.cfg.secret).password(true).desired_width(300.0));
                                ui.end_row();
                                ui.label("Callback port");
                                let mut port = self.cfg.port.to_string();
                                if ui.add(TextEdit::singleline(&mut port).desired_width(80.0)).changed() {
                                    if let Ok(p) = port.parse::<u16>() {
                                        self.cfg.port = p;
                                    }
                                }
                                ui.end_row();
                            });
                            ui.add_space(14.0);
                            ui.horizontal(|ui| {
                                let label = if self.login_busy { "Waiting for the browser…" } else { "Log in with Deezer" };
                                if pill(ui, label, true).clicked() && !self.login_busy {
                                    self.acts.push(Act::Login);
                                }
                                if self.login_busy {
                                    ui.spinner();
                                }
                            });
                        });
                    });
                    ui.add_space(40.0);
                });
            });
        });
    }

    /// Vue de connexion : bandeau egui + page Deezer officielle (WebView native) en dessous.
    pub fn web_login_ui(&mut self, ui: &mut Ui) -> Rect {
        let full = ui.ctx().content_rect();
        let head = Rect::from_min_size(full.min, vec2(full.width(), 52.0));
        egui::CentralPanel::default().frame(Frame::new().fill(BG)).show(ui, |ui| {
            self.drag_area(ui, head, "webdrag");
            let p = ui.painter().clone();
            p.rect_filled(head, CornerRadius::ZERO, SIDE);
            p.hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, LINE));
            let dc = pos2(head.min.x + 100.0, head.center().y);
            brand_mark(&p, dc, 8.0);
            title_text(&p, pos2(dc.x + 12.0, head.center().y - 11.0), "Deezer", 18.0, INK, 120.0);
            text(&p, pos2(dc.x + 90.0, head.center().y - 8.0), "Sign in", reg(13.5), MUTED, 200.0);
            let br = Rect::from_center_size(pos2(head.max.x - 60.0, head.center().y), vec2(88.0, 30.0));
            let resp = ui.interact(br, ui.scope_id().with("web_cancel"), Sense::click());
            let k = anim(ui, ui.scope_id().with("web_cancel_h"), resp.hovered(), 0.15);
            p.rect(br, CornerRadius::same(15), CARD.lerp_to_gamma(HOVER, k), Stroke::new(1.0, LINE), StrokeKind::Inside);
            text_center(&p, br.center().x, br.center().y - 8.0, "Cancel", med(13.0), INK, 80.0);
            if resp.clicked() {
                self.acts.push(Act::CancelWebLogin);
            }
            let body = Rect::from_min_max(pos2(full.min.x, head.max.y), full.max);
            if self.web.is_none() {
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(body.height() * 0.4);
                        ui.spinner();
                    });
                });
            }
        });
        Rect::from_min_max(pos2(full.min.x, head.max.y), full.max)
    }

    // -------------------------------------------------------------- fenêtres / toasts

    pub fn overlays(&mut self, ctx: &Context) {
        if let Some((msg, _)) = &self.toast {
            let el = self.toast.as_ref().map(|t| t.1.elapsed().as_secs_f32()).unwrap_or(0.0);
            let fin = ease((el / 0.25).min(1.0));
            let a = fin * ((3.0 - el) / 0.45).clamp(0.0, 1.0);
            if el < 0.3 || el > 2.5 {
                ctx.request_repaint();
            }
            egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -114.0 + (1.0 - fin) * 12.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_opacity(a);
                Frame::new()
                    .fill(RAISED)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(18))
                    .shadow(Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(150) })
                    .inner_margin(Margin { left: 14, right: 18, top: 9, bottom: 9 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (d, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                            ui.painter().circle_filled(d.center(), 4.0, acc());
                            ui.label(RichText::new(msg).font(med(13.5)));
                        });
                    });
            });
        }
        if let Some(name) = self.new_playlist.as_mut() {
            let mut close = false;
            let mut create = None;
            egui::Window::new(RichText::new("New playlist").font(display(19.0))).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                let r = ui.add(TextEdit::singleline(name).hint_text("Playlist name").desired_width(300.0).margin(Margin::symmetric(10, 8)));
                r.request_focus();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if pill(ui, "Create", true).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        if !name.trim().is_empty() {
                            create = Some(name.trim().to_owned());
                        }
                        close = true;
                    }
                    if pill(ui, "Cancel", false).clicked() {
                        close = true;
                    }
                });
            });
            if let Some(n) = create {
                self.acts.push(Act::NewPlaylist(n));
            }
            if close {
                self.new_playlist = None;
            }
        }
        if self.show_settings {
            let frame = Frame::new()
                .fill(RAISED)
                .corner_radius(CornerRadius::same(20))
                .stroke(Stroke::new(1.0, LINE))
                .inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 22 })
                .shadow(Shadow { offset: [0, 24], blur: 70, spread: 0, color: Color32::from_black_alpha(190) });
            let r = egui::Modal::new(egui::Id::new("settings")).backdrop_color(Color32::from_black_alpha(150)).frame(frame).show(ctx, |ui| {
                ui.set_width(470.0);
                self.settings_ui(ui)
            });
            if r.should_close() || r.inner {
                self.show_settings = false;
            }
        }
    }

    /// Réglages : groupes de lignes (titre + détail à gauche, contrôle à droite).
    fn settings_ui(&mut self, ui: &mut Ui) -> bool {
        use crate::update::{State, VERSION};
        let mut close = false;
        // en-tête
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        text(ui.painter(), pos2(hr.min.x, hr.center().y - 13.0), "Settings", display(22.0), INK, 300.0);
        let cb = Rect::from_center_size(pos2(hr.max.x - 12.0, hr.center().y), vec2(30.0, 30.0));
        let cr = ui.interact(cb, ui.id().with("set_close"), Sense::click());
        if cr.hovered() {
            ui.painter().circle_filled(cb.center(), 15.0, HOVER);
        }
        icons::draw(ui.painter(), Ic::Close, cb.center(), 13.0, if cr.hovered() { INK } else { MUTED });
        if cr.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            close = true;
        }

        set_head(ui, "Appearance");
        set_group(ui, |ui| {
            set_row(ui, "ASCII aurora", "Takes the cover's colours · animates only while playing", false, |ui| {
                if switch(ui, self.cfg.aurora).clicked() {
                    self.cfg.aurora = !self.cfg.aurora;
                    self.cfg.save();
                }
            });
        });

        set_head(ui, "Playback");
        let (cur_q, flac) = (self.ws.quality, self.ws.flac);
        let opts: [(&str, &str, &str); 4] = [("", "Account", "Uses your Deezer account setting"), ("standard", "Standard", "MP3 128 kbps"), ("high", "High", "MP3 320 kbps"), ("lossless", "HiFi", "FLAC 16-bit / 44.1 kHz · Deezer Premium")];
        let sel = opts.iter().position(|o| o.0 == self.cfg.quality).unwrap_or(0);
        let now = match cur_q {
            "low" => "Low",
            "standard" => "MP3 128",
            "high" => "MP3 320",
            "lossless" => "FLAC",
            _ => "",
        };
        let mut sub = opts[sel].2.to_owned();
        if !now.is_empty() {
            sub += &format!(" · playing {now}");
        }
        if sel == 3 && !flac && !cur_q.is_empty() {
            sub += " · HiFi not offered for this account";
        }
        let mut pick = None;
        set_group(ui, |ui| {
            set_row(ui, "Audio quality", &sub, false, |_| {});
            // contrôle segmenté : piste sombre, segment actif blanc qui glisse
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
            let seg_w = r.width() / 4.0;
            let p = ui.painter();
            p.rect_filled(r, CornerRadius::same(10), Color32::from_white_alpha(7));
            let x = ui.ctx().animate_value_with_time(ui.id().with("q_seg"), sel as f32, 0.2);
            let ar = Rect::from_min_size(pos2(r.min.x + seg_w * x, r.min.y), vec2(seg_w, r.height())).shrink(3.0);
            p.rect_filled(ar.translate(vec2(0.0, 1.0)), CornerRadius::same(8), Color32::from_black_alpha(60));
            p.rect_filled(ar, CornerRadius::same(8), INK);
            for (i, o) in opts.iter().enumerate() {
                let sr = Rect::from_min_size(pos2(r.min.x + seg_w * i as f32, r.min.y), vec2(seg_w, r.height()));
                let resp = ui.interact(sr, ui.id().with(("q", i)), Sense::click());
                let on = (1.0 - (x - i as f32).abs()).clamp(0.0, 1.0);
                let col = (if resp.hovered() { INK } else { SECOND }).lerp_to_gamma(BG, on);
                let g = galley(p, o.1, med(12.5), col, seg_w - 8.0);
                p.galley(sr.center() - g.size() / 2.0, g, col);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && i != sel {
                    pick = Some(o.0);
                }
            }
            ui.add_space(14.0);
        });
        if let Some(q) = pick {
            self.cfg.quality = q.to_owned();
            self.cfg.save();
            if let Some(w) = &self.web {
                w.set_quality(q);
            }
        }

        set_head(ui, "Updates");
        let st = self.upd.get();
        let sub = match &st {
            State::Idle => "Unofficial client · open source".to_owned(),
            State::Checking => "Checking…".to_owned(),
            State::UpToDate => "You're up to date".to_owned(),
            State::Available { ver, size, .. } => format!("Version {ver} available · {:.1} MB", *size as f64 / 1_048_576.0),
            State::Downloading { ver, done, size } => format!("Downloading {ver} · {:.0} %", if *size > 0 { *done as f64 * 100.0 / *size as f64 } else { 0.0 }),
            State::Ready { ver, .. } => format!("{ver} is ready to install"),
            State::Error(e) => e.clone(),
        };
        set_group(ui, |ui| {
            set_row(ui, &format!("Deezer {VERSION}"), &sub, false, |ui| match &st {
                State::Idle | State::UpToDate | State::Error(_) => {
                    if pill(ui, "Check", false).clicked() {
                        self.upd.check();
                    }
                }
                State::Checking => {
                    ui.add(egui::Spinner::new().color(MUTED).size(16.0));
                }
                State::Available { .. } => {
                    if pill(ui, "Download", true).clicked() {
                        self.upd.download();
                    }
                }
                State::Downloading { done, size, .. } => {
                    let f = if *size > 0 { *done as f32 / *size as f32 } else { 0.0 };
                    let (r, _) = ui.allocate_exact_size(vec2(110.0, 4.0), Sense::hover());
                    slider(ui.painter(), r, f.min(1.0), 0.0, INK);
                }
                State::Ready { .. } => {
                    if pill(ui, "Restart", true).clicked() {
                        self.cfg.save();
                        if let Err(e) = self.upd.install_and_restart() {
                            *self.upd.state.lock().unwrap() = State::Error(e);
                        }
                    }
                }
            });
            set_row(ui, "Check at launch", "Looks for a new release on GitHub", true, |ui| {
                if switch(ui, self.cfg.auto_update).clicked() {
                    self.cfg.auto_update = !self.cfg.auto_update;
                    self.cfg.save();
                }
            });
            set_row(ui, "Release notes", "", true, |ui| {
                ui.hyperlink_to(RichText::new("GitHub ↗").font(med(13.0)).color(SECOND), format!("https://github.com/{}/releases", crate::update::REPO));
            });
        });
        if let State::Available { notes, .. } = &st {
            let n = notes.trim();
            if !n.is_empty() {
                ui.add_space(8.0);
                let n: String = n.chars().take(500).collect();
                ui.label(RichText::new(n).color(MUTED).font(reg(12.0)));
            }
        }

        set_head(ui, "Account & storage");
        set_group(ui, |ui| {
            set_row(ui, "Image cache", "Covers kept on disk for fast loading", false, |ui| {
                if pill(ui, "Clear", false).clicked() {
                    self.acts.push(Act::ClearImgCache);
                }
            });
            set_row(ui, "Deezer account", "Sign out on this Mac", true, |ui| {
                if pill(ui, "Log out", false).clicked() {
                    self.acts.push(Act::Logout);
                }
            });
        });

        set_head(ui, "Deezer application (OAuth)");
        set_group(ui, |ui| {
            set_row(ui, "Application ID", "", false, |ui| {
                ui.add(TextEdit::singleline(&mut self.cfg.app_id).desired_width(200.0).margin(Margin::symmetric(8, 5)));
            });
            set_row(ui, "Secret key", "", true, |ui| {
                ui.add(TextEdit::singleline(&mut self.cfg.secret).password(true).desired_width(200.0).margin(Margin::symmetric(8, 5)));
            });
        });
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pill(ui, "Save", true).clicked() {
                self.acts.push(Act::SaveConfig);
            }
            if !self.audio_err.is_empty() {
                ui.label(RichText::new(format!("Audio: {}", self.audio_err)).color(ACCENT).font(reg(12.0)));
            }
        });
        close
    }

    /// Aurora ASCII teintée par la pochette en cours (shader GPU) + accent de l'interface.
    /// Optimisé : animée (≈20 fps) seulement pendant la lecture, fenêtre active ; figée sinon
    /// (aucun repaint). Le dessin est limité (scissor) à la zone où la lumière existe.
    fn paint_aurora(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let cover = self.cur.as_ref().and_then(|t| self.imgs.palette(&t.img, 120));
        self.pal_st.target(cover.unwrap_or(ambient::DEFAULT));
        let pal = self.pal_st.current();
        set_palette(&pal);
        let follow = ease(ctx.animate_bool_with_time(egui::Id::new("aur_follow"), cover.is_some(), 1.4));
        let big = !self.logged || (self.now_playing && self.cur.is_some());
        let tall = ease(ctx.animate_bool_with_time(egui::Id::new("aur_tall"), big, 0.7));
        let playing = self.is_playing();
        let active = ctx.input(|i| i.focused && !i.viewport().minimized.unwrap_or(false));
        let running = self.cfg.aurora && active && playing;
        self.live = running;
        let phase = self.aclock.step(playing, running);
        if let Some(a) = self.aurora.clone() {
            let full = ctx.content_rect();
            let opacity = if !self.logged { 0.62 } else { 0.72 + 0.1 * tall };
            let prm = crate::aurora::Params { phase, tall, opacity, cols: crate::aurora::Params::colours(&pal), follow };
            // la lumière n'occupe que le haut de la fenêtre : on coupe le reste (fragments jamais calculés)
            let small = (full.height() * 0.48).min(full.width() * 0.42);
            let h = (small + (full.height() - small) * tall).min(full.height());
            let p = ui.painter().with_clip_rect(Rect::from_min_size(full.min, vec2(full.width(), h)));
            a.add(&p, full, prm);
        }
        if running || (self.pal_st.fading() && active) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

static PROF: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("DEEZER_PROF").is_some());

impl eframe::App for App {
    fn logic(&mut self, _ctx: &Context, _frame: &mut eframe::Frame) {
        self.tick();
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let t_ui = Instant::now();
        let prev_cpu = frame.info().cpu_usage.unwrap_or(0.0);
        let mut web_rect = None;
        if self.web_login {
            web_rect = Some(self.web_login_ui(ui));
        } else {
            self.paint_aurora(ui);
            if self.logged {
                // raccourcis clavier
                if !ctx.egui_wants_keyboard_input() {
                    if ctx.input(|i| i.key_pressed(Key::Space)) {
                        self.acts.push(Act::Toggle);
                    }
                    if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::ArrowRight)) {
                        self.acts.push(Act::Next);
                    }
                    if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::ArrowLeft)) {
                        self.acts.push(Act::Prev);
                    }
                    if self.now_playing && ctx.input(|i| i.key_pressed(Key::Escape)) {
                        self.acts.push(Act::NowPlaying(false));
                    }
                }
                self.main_ui(ui);
            } else {
                self.login_screen(ui);
            }
        }
        // WebView Deezer : créée à la demande (il faut le handle de fenêtre), cachée hors connexion
        if let Some((url, visible)) = self.web_want.take() {
            let r = web_rect.unwrap_or(Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0)));
            let arl = if url == crate::web::ENGINE_URL { self.cfg.arl.clone() } else { String::new() };
            match crate::web::Web::new(frame, ctx.clone(), url, visible, r, &arl, &self.cfg.quality) {
                Ok(w) => self.web = Some(w),
                Err(e) => {
                    self.web_login = false;
                    self.login_err = format!("WebView error: {e}");
                }
            }
        }
        if let Some(w) = self.web.as_mut() {
            match web_rect {
                Some(r) => {
                    w.set_bounds(r);
                    w.show(true);
                }
                None => w.show(false),
            }
        }
        self.overlays(&ctx);
        let acts = std::mem::take(&mut self.acts);
        for a in acts {
            self.apply(a);
        }
        if *PROF {
            let ms = t_ui.elapsed().as_secs_f32() * 1000.0;
            let dt = ctx.input(|i| i.unstable_dt) * 1000.0;
            if ms > 4.0 || prev_cpu * 1000.0 > 8.0 || dt > 30.0 {
                eprintln!("frame {} ui={ms:.1}ms prev_cpu={:.1}ms dt={dt:.0}ms np={} imgs_loaded={}", ctx.cumulative_pass_nr(), prev_cpu * 1000.0, self.now_playing, self.imgs.cache.len());
            }
        }
    }

    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cfg.save();
    }
}
