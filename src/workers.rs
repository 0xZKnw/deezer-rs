//! Pools de threads : réseau (API + previews) et images. L'UI ne bloque jamais.

use crate::api::Api;
use eframe::egui;
use crate::config::{cache_dir, Config};
use crate::model::*;
use crate::pages::{self, Loaded, Loader};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

pub enum Mut {
    LikeTrack(u64, bool),
    FollowAlbum(u64, bool),
    FollowArtist(u64, bool),
    FollowPlaylist(u64, bool),
    CreatePlaylist(String),
    AddToPlaylist { pl: u64, track: u64 },
    RemoveFromPlaylist { pl: u64, track: u64 },
    DeletePlaylist(u64),
}

pub enum Job {
    Route { route: Route, gen: u64 },
    Library,
    Preview { id: u64, url: String },
    PlayContent { kind: Kind, id: u64, shuffle: bool },
    Flow,
    ArtistRadio(u64),
    TrackRadio(u64),
    Mutate(Mut),
}

pub enum Msg {
    Img(String, Option<egui::ColorImage>),
    Preview(u64, Result<Arc<Vec<u8>>, String>),
    Page(u64, Loaded),
    PageErr(u64, String),
    Lib(pages::LibPart),
    Play(Vec<Track>, bool),
    Toast(String),
    Login(Result<String, String>),
    Mutated { msg: String, refresh: bool },
    Lyrics(u64, crate::lyrics::Lyrics),
}

#[derive(Clone)]
pub struct Pool {
    net: Sender<Job>,
    img: Sender<String>,
    pub gen: Arc<AtomicU64>,
    pub me: Arc<AtomicU64>,
}

fn fnv(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl Pool {
    pub fn new(api: Api, tx: Sender<Msg>, ctx: egui::Context) -> Self {
        let (net_tx, net_rx) = channel::<Job>();
        let (img_tx, img_rx) = channel::<String>();
        let gen = Arc::new(AtomicU64::new(0));
        let me = Arc::new(AtomicU64::new(0));
        let net_rx = Arc::new(Mutex::new(net_rx));
        let img_rx = Arc::new(Mutex::new(img_rx));

        for _ in 0..4 {
            let (api, tx, ctx, rx, gen, me) =
                (api.clone(), tx.clone(), ctx.clone(), net_rx.clone(), gen.clone(), me.clone());
            std::thread::spawn(move || net_worker(api, tx, ctx, rx, gen, me));
        }
        let dir = cache_dir().join("img");
        let _ = fs::create_dir_all(&dir);
        std::thread::spawn({
            let dir = dir.clone();
            move || trim_cache(&dir, 300 << 20, 200 << 20)
        });
        for _ in 0..3 {
            let (api, tx, ctx, rx, dir) = (api.clone(), tx.clone(), ctx.clone(), img_rx.clone(), dir.clone());
            std::thread::spawn(move || img_worker(api, tx, ctx, rx, dir));
        }
        Self { net: net_tx, img: img_tx, gen, me }
    }

    pub fn job(&self, j: Job) {
        let _ = self.net.send(j);
    }

    pub fn image(&self, url: String) {
        let _ = self.img.send(url);
    }
}

fn trim_cache(dir: &std::path::Path, max: u64, target: u64) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort();
    for (_, len, p) in files {
        if total <= target {
            break;
        }
        if fs::remove_file(p).is_ok() {
            total -= len;
        }
    }
}

fn img_worker(api: Api, tx: Sender<Msg>, ctx: egui::Context, rx: Arc<Mutex<Receiver<String>>>, dir: std::path::PathBuf) {
    loop {
        let url = match rx.lock().unwrap().recv() {
            Ok(u) => u,
            Err(_) => return,
        };
        let path = dir.join(format!("{:016x}", fnv(&url)));
        let bytes = fs::read(&path).ok().or_else(|| {
            let b = api.download(&url, 4 << 20).ok()?;
            let _ = fs::write(&path, &b);
            Some(b)
        });
        let img = bytes.and_then(|b| image::load_from_memory(&b).ok()).map(|im| {
            let rgba = im.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
        });
        let _ = tx.send(Msg::Img(url, img));
        ctx.request_repaint();
    }
}

fn net_worker(
    api: Api,
    tx: Sender<Msg>,
    ctx: egui::Context,
    rx: Arc<Mutex<Receiver<Job>>>,
    gen: Arc<AtomicU64>,
    me: Arc<AtomicU64>,
) {
    let send = |m: Msg| {
        let _ = tx.send(m);
        ctx.request_repaint();
    };
    loop {
        let job = match rx.lock().unwrap().recv() {
            Ok(j) => j,
            Err(_) => return,
        };
        match job {
            Job::Route { route, gen: g } => {
                if gen.load(Ordering::Relaxed) != g {
                    continue; // l'utilisateur a déjà navigué ailleurs
                }
                let emit = |l: Loaded| send(Msg::Page(g, l));
                let alive = || gen.load(Ordering::Relaxed) == g;
                let ld = Loader { api: &api, me: me.load(Ordering::Relaxed), emit: &emit, alive: &alive };
                if let Err(e) = ld.route(&route) {
                    send(Msg::PageErr(g, e));
                }
            }
            Job::Library => {
                let emit = |l: Loaded| {
                    if let Loaded::Lib(p) = l {
                        send(Msg::Lib(p));
                    }
                };
                let alive = || true;
                let ld = Loader { api: &api, me: 0, emit: &emit, alive: &alive };
                ld.library();
            }
            Job::Preview { id, url } => {
                let mut r = api.download(&url, 4 << 20);
                if r.is_err() {
                    // URL signée expirée : on redemande une URL de preview fraîche
                    if let Ok(v) = api.get(&format!("/track/{id}"), &[]) {
                        let fresh = s(&v, "preview");
                        if !fresh.is_empty() {
                            r = api.download(&fresh, 4 << 20);
                        }
                    }
                }
                let r = r.map(|b| Arc::new(crate::audio::clean_mp3(b)));
                send(Msg::Preview(id, r));
            }
            Job::PlayContent { kind, id, shuffle } => match pages::fetch_play_tracks(&api, kind, id) {
                Ok(t) => send(Msg::Play(t, shuffle)),
                Err(e) => send(Msg::Toast(e)),
            },
            Job::Flow => match pages::fetch_flow(&api) {
                Ok(t) => send(Msg::Play(t, false)),
                Err(e) => send(Msg::Toast(e)),
            },
            Job::ArtistRadio(id) => match pages::fetch_artist_radio(&api, id) {
                Ok(t) => send(Msg::Play(t, false)),
                Err(e) => send(Msg::Toast(e)),
            },
            Job::TrackRadio(id) => match pages::fetch_track_radio(&api, id) {
                Ok(t) => send(Msg::Play(t, false)),
                Err(e) => send(Msg::Toast(e)),
            },
            Job::Mutate(m) => {
                let (r, ok_msg, refresh) = match m {
                    Mut::LikeTrack(id, on) => (
                        if on {
                            api.post("/user/me/tracks", &[("track_id", id.to_string())])
                        } else {
                            api.delete("/user/me/tracks", &[("track_id", id.to_string())])
                        },
                        if on { "Added to Favourite tracks" } else { "Removed from Favourite tracks" },
                        false,
                    ),
                    Mut::FollowAlbum(id, on) => (
                        if on {
                            api.post("/user/me/albums", &[("album_id", id.to_string())])
                        } else {
                            api.delete("/user/me/albums", &[("album_id", id.to_string())])
                        },
                        if on { "Album added to your library" } else { "Album removed" },
                        true,
                    ),
                    Mut::FollowArtist(id, on) => (
                        if on {
                            api.post("/user/me/artists", &[("artist_id", id.to_string())])
                        } else {
                            api.delete("/user/me/artists", &[("artist_id", id.to_string())])
                        },
                        if on { "Artist followed" } else { "Artist unfollowed" },
                        true,
                    ),
                    Mut::FollowPlaylist(id, on) => (
                        if on {
                            api.post("/user/me/playlists", &[("playlist_id", id.to_string())])
                        } else {
                            api.delete("/user/me/playlists", &[("playlist_id", id.to_string())])
                        },
                        if on { "Playlist added to your library" } else { "Playlist removed" },
                        true,
                    ),
                    Mut::CreatePlaylist(title) => {
                        (api.post("/user/me/playlists", &[("title", title)]), "Playlist created", true)
                    }
                    Mut::AddToPlaylist { pl, track } => (
                        api.post(&format!("/playlist/{pl}/tracks"), &[("songs", track.to_string())]),
                        "Added to playlist",
                        true,
                    ),
                    Mut::RemoveFromPlaylist { pl, track } => (
                        api.delete(&format!("/playlist/{pl}/tracks"), &[("songs", track.to_string())]),
                        "Removed from playlist",
                        false,
                    ),
                    Mut::DeletePlaylist(pl) => (api.delete(&format!("/playlist/{pl}"), &[]), "Playlist deleted", true),
                };
                match r {
                    Ok(_) => send(Msg::Mutated { msg: ok_msg.into(), refresh }),
                    Err(e) => send(Msg::Toast(format!("Error: {e}"))),
                }
            }
        }
    }
}

#[allow(dead_code)]
pub fn spawn_login(cfg: Config, tx: Sender<Msg>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let r = crate::auth::login(&cfg);
        let _ = tx.send(Msg::Login(r));
        ctx.request_repaint();
    });
}
