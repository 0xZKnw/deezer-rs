// Lecteur Deezer léger — Rust + egui (glow). Client non officiel.
#![allow(deprecated)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod audio;
mod aurora;
mod ambient;
mod auth;
mod config;
mod icons;
mod lyrics;
mod model;
mod pages;
mod recent;
mod ui;
mod update;
mod web;
mod workers;

use eframe::egui;

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Deezer")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([960.0, 600.0])
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false)
        // icône vide : eframe n'impose pas son logo egui au Dock, macOS prend celle du bundle
        .with_icon(egui::IconData::default());
    let options = eframe::NativeOptions {
        viewport,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native("deezer-rs", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
