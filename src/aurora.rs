//! Aurora ASCII en shader OpenGL (inspirée de StudioAurora, MLXL3 v2.0.6), teintée par la pochette
//! du morceau en cours : la colonne de lumière prend la couleur de la musique.
//!
//! La lumière (pied incurvé blanc-chaud, colonne violette) est calculée une fois sur CPU dans une
//! petite texture ; le shader compose 4 « rideaux » teintés, fait dériver le champ dans le temps,
//! et choisit pour chaque cellule de 5×8 pt un glyphe `.:-=+xX` selon l'intensité.
//! Tout le travail par pixel est sur GPU : coût CPU ≈ 0, aucun upload par frame.

use eframe::egui::{self, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::Arc;

const LW: usize = 370;
const LH: usize = 310;

/// (x, scale, hue°, strength, y) — mêmes rideaux que StudioAurora.swift
const CURTAINS: [(f32, f32, f32, f32, f32); 4] =
    [(0.045, 0.91, -38.0, 0.61, 0.64), (0.77, 1.08, 0.0, 1.0, 0.64), (1.05, 0.93, 9.0, 0.61, 0.64), (0.34, 0.67, -26.0, 0.52, -0.03)];

fn light(x: f64, y: f64) -> ([f64; 3], f64) {
    let foot = 0.70 + 0.20 * x * x + 0.027 * (4.5 * x).sin();
    let d = foot - y;
    let span = 0.25 + 0.16 * d.max(0.0);
    let column = (-(x / span).powi(2) - ((d - 0.22) / 0.43).powi(2)).exp();
    let rim = (-(x / 0.64).powi(2) - (d / 0.064).powi(2)).exp();
    let haze = (-(x / 0.66).powi(2) - ((d - 0.17) / 0.46).powi(2)).exp();
    let hot = (-(x / 0.32).powi(2) - ((d - 0.055) / 0.10).powi(2)).exp();
    let pink = (-((d - 0.14) / 0.19).powi(2)).exp();
    let rgb = [
        (0.20 + 0.78 * pink + 0.64 * rim + 0.30 * hot).clamp(0.0, 1.0),
        (0.022 + 0.10 * d.max(0.0) + 0.61 * hot).clamp(0.0, 1.0),
        (0.98 - 0.55 * rim - 0.61 * hot).clamp(0.0, 1.0),
    ];
    let fade = (y * 8.0).clamp(0.0, 1.0) * ((1.0 - x.abs()) * 8.0).clamp(0.0, 1.0) * ((1.0 - y) * 9.0).clamp(0.0, 1.0);
    let a = (0.91 * column + 0.94 * rim + 0.24 * haze).min(1.0) * fade * (-((-d).max(0.0) / 0.09).powi(2)).exp();
    (rgb, a)
}

fn lamp_pixels() -> Vec<u8> {
    let mut px = vec![0u8; LW * LH * 4];
    for row in 0..LH {
        for col in 0..LW {
            let (rgb, a) = light(2.0 * (col as f64 + 0.5) / LW as f64 - 1.0, (row as f64 + 0.5) / LH as f64);
            let i = (row * LW + col) * 4;
            for k in 0..3 {
                px[i + k] = (rgb[k] * a * 255.0) as u8; // prémultiplié
            }
            px[i + 3] = (a * 255.0) as u8;
        }
    }
    px
}

const VS: &str = r#"
V_IN vec2 a_pos;
V_OUT vec2 v_uv;
void main() {
    v_uv = vec2(a_pos.x * 0.5 + 0.5, 0.5 - a_pos.y * 0.5); // y vers le bas
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const FS: &str = r#"
uniform sampler2D u_lamp;
uniform vec2 u_size;
uniform float u_phase;
uniform float u_ppp;
uniform float u_opacity;
uniform vec4 u_rect[4];
uniform vec4 u_tint[4];
V_IN vec2 v_uv;

vec3 hue(vec3 c, float cs, float sn) {
    return max(vec3(
        dot(c, vec3(0.213 + cs*0.787 - sn*0.213, 0.715 - cs*0.715 - sn*0.715, 0.072 - cs*0.072 + sn*0.928)),
        dot(c, vec3(0.213 - cs*0.213 + sn*0.143, 0.715 + cs*0.285 + sn*0.140, 0.072 - cs*0.072 - sn*0.283)),
        dot(c, vec3(0.213 - cs*0.213 - sn*0.787, 0.715 - cs*0.715 + sn*0.715, 0.072 + cs*0.928 + sn*0.072))), 0.0);
}

vec4 prints(vec2 q) {
    vec2 pt = q * u_size;
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 4; i++) {
        vec4 r = u_rect[i];
        vec2 uv = (pt - r.xy) / r.zw + 0.5;
        if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) continue;
        vec4 c = TEX(u_lamp, uv);
        c.rgb = hue(c.rgb, u_tint[i].x, u_tint[i].y);
        c *= u_tint[i].z;
        acc = c + acc * (1.0 - c.a);
    }
    return acc;
}

vec2 lightUV(vec2 p, float t) {
    vec2 drift = vec2(0.014*(sin(4.0*p.y+0.44*t)-sin(4.0*p.y)),
                      0.028*(sin(5.0*p.x-0.49*t)-sin(5.0*p.x)));
    return clamp(p + drift * smoothstep(vec2(0.0), vec2(0.12), min(p, 1.0 - p)), 0.0, 1.0);
}

float seg(vec2 p, vec2 a, vec2 b, float hw) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return clamp((hw - length(pa - ba * h)) * u_ppp + 0.5, 0.0, 1.0);
}
float dotc(vec2 p, vec2 c, float r) { return clamp((r - length(p - c)) * u_ppp + 0.5, 0.0, 1.0); }

// Glyphes Menlo 7 pt dans une cellule de 5×8 pt (ligne de base à 6.4 pt)
float glyph(float k, vec2 p) {
    if (k < 0.5) return dotc(p, vec2(2.5, 5.85), 0.55);
    if (k < 1.5) return max(dotc(p, vec2(2.5, 3.2), 0.55), dotc(p, vec2(2.5, 5.85), 0.55));
    if (k < 2.5) return seg(p, vec2(1.3, 4.4), vec2(3.7, 4.4), 0.32);
    if (k < 3.5) return max(seg(p, vec2(0.9, 3.6), vec2(4.1, 3.6), 0.3), seg(p, vec2(0.9, 5.2), vec2(4.1, 5.2), 0.3));
    if (k < 4.5) return max(seg(p, vec2(0.9, 4.4), vec2(4.1, 4.4), 0.32), seg(p, vec2(2.5, 2.8), vec2(2.5, 6.0), 0.32));
    if (k < 5.5) return max(seg(p, vec2(1.0, 2.7), vec2(4.0, 6.4), 0.33), seg(p, vec2(4.0, 2.7), vec2(1.0, 6.4), 0.33));
    return max(seg(p, vec2(0.8, 1.3), vec2(4.2, 6.4), 0.35), seg(p, vec2(4.2, 1.3), vec2(0.8, 6.4), 0.35));
}

void main() {
    vec2 p = v_uv;
    float t = u_phase;
    vec4 lamp = prints(lightUV(p, t));
    if (lamp.a <= 0.003) { O_COLOR = vec4(0.0); return; }
    vec2 cells = u_size / vec2(5.0, 8.0);
    vec2 cell = floor(p * cells);
    vec2 center = (cell + 0.5) / cells;
    vec2 local = fract(p * cells);
    float arc = p.y + 0.065*sin(5.0*p.x + 0.24*t) + 0.035*sin(10.0*p.x - 0.31*t);
    float ribbon = 0.5 + 0.5*cos(10.5*arc - 0.82*t + 1.3*p.x);
    float glint = 0.5 + 0.5*sin(18.0*arc + 3.3*p.x - 0.58*t);
    float veil = 0.5 + 0.5*cos(4.7*p.x - 2.6*p.y + 0.37*t);
    float light = 0.77 + 0.26*ribbon + 0.12*glint*glint + 0.05*veil;
    vec3 emission = vec3(light + 0.08*ribbon*glint, light, light + 0.085*(1.0-ribbon)*veil);
    float intensity = prints(lightUV(center, t)).a;
    float wave = 0.5 + 0.5*cos(10.5*(center.y + 0.065*sin(5.0*center.x + 0.24*t)) - 0.82*t + 1.3*center.x);
    float level = 6.0 * clamp(intensity * (0.65 + 0.60*wave), 0.0, 1.0);
    float index = floor(level);
    vec2 lp = local * vec2(5.0, 8.0);
    float first = glyph(index, lp);
    float next = glyph(min(6.0, index + 1.0), lp);
    float mark = mix(first, next, smoothstep(0.32, 0.68, fract(level)));
    float alpha = lamp.a;
    vec3 color = min(lamp.rgb * emission, vec3(alpha));
    O_COLOR = vec4(mix(color, vec3(alpha), 0.45*mark*smoothstep(0.25, 1.65, level)), alpha) * u_opacity;
}
"#;

pub struct Aurora {
    prog: glow::Program,
    vao: Option<glow::VertexArray>,
    vbo: glow::Buffer,
    tex: glow::Texture,
    loc: [Option<glow::UniformLocation>; 8],
}

// Les handles GL sont de simples entiers ; on ne les utilise que depuis le thread de rendu.
unsafe impl Send for Aurora {}
unsafe impl Sync for Aurora {}

#[derive(Clone, Copy)]
pub struct Params {
    pub phase: f32,
    pub full_height: bool,
    pub opacity: f32,
    /// rotation de teinte globale (degrés) appliquée à tous les rideaux
    pub shift: f32,
}

/// Teinte dominante de la lampe d'origine (violet) : décalage nul pour la palette par défaut.
pub const BASE_HUE: f32 = 266.0;

impl Aurora {
    pub fn new(gl: &glow::Context) -> Option<Self> {
        let sv = egui_glow::ShaderVersion::get(gl);
        let new = sv.is_new_shader_interface();
        let head = sv.version_declaration();
        let (vdefs, fdefs) = if new {
            ("#define V_IN in\n#define V_OUT out\n", "#define V_IN in\n#define TEX texture\n#define O_COLOR o_color\nout vec4 o_color;\n")
        } else {
            ("#define V_IN attribute\n#define V_OUT varying\n", "#define V_IN varying\n#define TEX texture2D\n#define O_COLOR gl_FragColor\n")
        };
        let prec = if matches!(sv, egui_glow::ShaderVersion::Es100 | egui_glow::ShaderVersion::Es300) { "precision highp float;\n" } else { "" };
        unsafe {
            let prog = gl.create_program().ok()?;
            let mut shaders = vec![];
            for (ty, src) in [(glow::VERTEX_SHADER, format!("{head}{prec}{vdefs}{VS}")), (glow::FRAGMENT_SHADER, format!("{head}{prec}{fdefs}{FS}"))] {
                let s = gl.create_shader(ty).ok()?;
                gl.shader_source(s, &src);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    eprintln!("aurora shader: {}", gl.get_shader_info_log(s));
                    return None;
                }
                gl.attach_shader(prog, s);
                shaders.push(s);
            }
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) {
                eprintln!("aurora link: {}", gl.get_program_info_log(prog));
                return None;
            }
            for s in shaders {
                gl.detach_shader(prog, s);
                gl.delete_shader(s);
            }
            let vao = if new { gl.create_vertex_array().ok() } else { None };
            if let Some(v) = vao {
                gl.bind_vertex_array(Some(v));
            }
            let vbo = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let tri: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
            let bytes: Vec<u8> = tri.iter().flat_map(|f| f.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            if vao.is_some() {
                gl.bind_vertex_array(None);
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            let tex = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            let px = lamp_pixels();
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                LW as i32,
                LH as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&px)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            let names = ["u_lamp", "u_size", "u_phase", "u_ppp", "u_opacity", "u_rect", "u_tint", ""];
            let loc = names.map(|n| if n.is_empty() { None } else { gl.get_uniform_location(prog, n) });
            Some(Self { prog, vao, vbo, tex, loc })
        }
    }

    fn paint(&self, gl: &glow::Context, info: &egui::PaintCallbackInfo, prm: Params) {
        let size = info.viewport.size();
        let (w, h) = (size.x, size.y);
        // géométrie des rideaux (cf. StudioAurora.prints)
        let height = if prm.full_height { h * 0.79 } else { (h * 0.48).min(w * 0.42) };
        let mut rects = [0f32; 16];
        let mut tints = [0f32; 16];
        for (i, &(x, scale, hue, strength, y)) in CURTAINS.iter().enumerate() {
            let width = (w * if prm.full_height { 0.66 } else { 0.68 }).min(height * 2.1) * scale;
            let depth = height * scale;
            let s = (i as f32).sin();
            let cy = depth * (if prm.full_height { y } else { 0.31 } + 0.035 * s);
            rects[i * 4..i * 4 + 4].copy_from_slice(&[w * (x + 0.026 * s), cy, width, depth]);
            let r = (hue + prm.shift).to_radians();
            tints[i * 4..i * 4 + 4].copy_from_slice(&[r.cos(), r.sin(), strength, 0.0]);
        }
        unsafe {
            gl.use_program(Some(self.prog));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            gl.uniform_1_i32(self.loc[0].as_ref(), 0);
            gl.uniform_2_f32(self.loc[1].as_ref(), w, h);
            gl.uniform_1_f32(self.loc[2].as_ref(), prm.phase);
            gl.uniform_1_f32(self.loc[3].as_ref(), info.pixels_per_point);
            gl.uniform_1_f32(self.loc[4].as_ref(), prm.opacity);
            gl.uniform_4_f32_slice(self.loc[5].as_ref(), &rects);
            gl.uniform_4_f32_slice(self.loc[6].as_ref(), &tints);
            match self.vao {
                Some(v) => gl.bind_vertex_array(Some(v)),
                None => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                    gl.enable_vertex_attrib_array(0);
                    gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
                }
            }
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            }
        }
    }

    /// Ajoute l'aurora dans `rect` (fond de fenêtre).
    pub fn add(self: &Arc<Self>, p: &egui::Painter, rect: Rect, prm: Params) {
        let me = self.clone();
        p.add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| me.paint(painter.gl(), &info, prm))),
        });
    }
}

/// Phase du champ : on intègre la vitesse pour qu'un changement de rythme ne fasse jamais sauter
/// le motif (cf. AuroraPlayback). Plus rapide pendant la lecture.
pub struct Clock {
    pub phase: f64,
    speed: f64,
    last: Option<std::time::Instant>,
}

impl Default for Clock {
    fn default() -> Self {
        Self { phase: 0.0, speed: 1.0, last: None }
    }
}

impl Clock {
    pub fn step(&mut self, playing: bool, running: bool) -> f32 {
        let now = std::time::Instant::now();
        if let Some(l) = self.last {
            let dt = (now - l).as_secs_f64().min(0.05);
            if running {
                let target = if playing { 1.9 } else { 1.0 };
                // détente exponentielle (~1.2 s)
                self.speed += (target - self.speed) * (1.0 - (-dt / 0.45).exp());
                self.phase += dt * self.speed;
            }
        }
        self.last = Some(now);
        self.phase as f32
    }
}
