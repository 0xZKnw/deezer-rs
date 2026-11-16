//! État de l'application + logique (navigation, lecteur, bibliothèque). Le rendu est dans ui.rs.

use crate::audio::Audio;
use crate::config::Config;
use crate::model::*;
use crate::pages::{LibPart, Loaded};
use crate::ui::Images;
use crate::workers::*;
use eframe::egui::{self, Context};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::Ordering,
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Library {
    pub me_id: u64,
    pub me_name: String,
    pub me_img: String,
    pub tracks: Vec<Track>,
    pub tracks_done: bool,
    pub fav: HashSet<u64>,
    pub albums: Vec<Card>,
    pub artists: Vec<Card>,
    pub playlists: Vec<Card>,
    pub podcasts: Vec<Card>,
    pub album_ids: HashSet<u64>,
    pub artist_ids: HashSet<u64>,
    pub playlist_ids: HashSet<u64>,
}

#[derive(Default)]
pub struct CarState {
    pub off: f32,
    pub anim: Option<(f32, f32, Instant)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Col {
    Title,
    Artist,
    Album,
    Added,
    Dur,
    Pop,
}

#[derive(Default)]
pub struct PageState {
    pub data: PageData,
    pub loading: bool,
    pub err: String,
    pub view: Vec<u32>,
    pub dirty: bool,
    pub sort: Option<(Col, bool)>,
    pub filter: String,
}

impl PageState {
    pub fn rebuild_view(&mut self) {
        self.dirty = false;
        let f = self.filter.to_lowercase();
        let tr = &self.data.tracks;
        let mut v: Vec<u32> = (0..tr.len() as u32)
            .filter(|&i| {
                if f.is_empty() {
                    return true;
                }
                let t = &tr[i as usize];
                t.title.to_lowercase().contains(&f)
                    || t.artist.to_lowercase().contains(&f)
                    || t.album.to_lowercase().contains(&f)
            })
            .collect();
        if let Some((col, asc)) = self.sort {
            match col {
                Col::Title => v.sort_by_cached_key(|&i| tr[i as usize].title.to_lowercase()),
                Col::Artist => v.sort_by_cached_key(|&i| tr[i as usize].artist.to_lowercase()),
                Col::Album => v.sort_by_cached_key(|&i| tr[i as usize].album.to_lowercase()),
                Col::Added => v.sort_by_key(|&i| tr[i as usize].added),
                Col::Dur => v.sort_by_key(|&i| tr[i as usize].dur),
                Col::Pop => v.sort_by_key(|&i| tr[i as usize].rank),
            }
            if !asc {
                v.reverse();
            }
        }
        self.view = v;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

pub enum Act {
    /// vue « Now Playing » plein écran
    NowPlaying(bool),
    /// panneau des paroles (ouvre la vue plein écran si besoin)
    ToggleLyrics,
    Go(Route),
    Back,
    Forward,
    PlayList { tracks: Vec<Track>, start: usize, shuffle: bool },
    PlayContent(Kind, u64, bool),
    Flow,
    ArtistRadio(u64),
    TrackRadio(u64),
    Enqueue(Track),
    PlayNext(Track),
    Like(Track),
    Follow(Kind, u64, bool),
    AddToPlaylist { pl: u64, track: u64 },
    RemoveFromPlaylist { pl: u64, track: u64 },
    NewPlaylist(String),
    OpenNewPlaylist,
    DeletePlaylist(u64),
    Toggle,
    Next,
    Prev,
    Seek(f32),
    Shuffle,
    Repeat,
    Volume(f32),
    OpenWeb(String),
    CopyText(String),
    Sort(Col),
    JumpQueue(usize),
    ClearQueue,
    Login,
    Logout,
    RefreshLib,
    ClearImgCache,
    SaveConfig,
    GuestLogin,
    PasteLogin,
    WebLogin,
    CancelWebLogin,
}

pub struct App {
    pub cfg: Config,
    pub api: crate::api::Api,
    pub pool: Pool,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub ctx: Context,
    pub imgs: Images,

    // auth
    pub logged: bool,
    pub login_busy: bool,
    pub login_err: String,
    pub guest_input: String,

    // navigation
    pub route: Route,
    pub back: Vec<Route>,
    pub fwd: Vec<Route>,
    pub page: PageState,
    pub lib: Library,
    pub carousel: HashMap<String, CarState>,
    pub page_t0: Instant,
    pub clip_checked: bool,

    // lecteur
    pub audio: Option<Audio>,
    pub audio_err: String,
    pub queue: Vec<Track>,
    orig: Vec<Track>,
    pub qi: usize,
    pub cur: Option<Track>,
    pub loading: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f32,
    pub vol_before_mute: f32,
    want: Option<u64>,
    pcache: HashMap<u64, Arc<Vec<u8>>>,
    porder: VecDeque<u64>,
    requested: HashSet<u64>,
    pub seeking: Option<f32>,

    // session Deezer officielle (WebView) : connexion + lecteur complet
    pub web: Option<crate::web::Web>,
    pub web_want: Option<(&'static str, bool)>,
    pub web_login: bool,
    pub web_user: u64,
    pub web_offer: String,
    pub ws: WebState,
    web_started: bool,
    /// id réellement joué par le lecteur Deezer (peut différer : version alternative / remplacée)
    web_song: u64,
    /// morceau que le lecteur jouait au moment de la demande (à ignorer)
    web_prev: u64,
    gw_seq: u64,
    gw_pending: HashMap<u64, (String, bool)>,

    // UI
    pub search: String,
    pub show_queue: bool,
    pub show_settings: bool,
    pub new_playlist: Option<String>,
    pub toast: Option<(String, Instant)>,
    pub acts: Vec<Act>,
    pub last_title: String,
    pub rng: u64,
    pub aurora: Option<Arc<crate::aurora::Aurora>>,
    pub aclock: crate::aurora::Clock,
    pub pal_st: crate::ambient::State,
    /// animations continues (lueur, égaliseur) actives à cette frame
    pub live: bool,
    pub upd: crate::update::Updater,
    pub upd_t0: Option<Instant>,
    pub upd_toasted: bool,
    pub now_playing: bool,
    pub np_t0: Instant,
    pub show_lyrics: bool,
    /// paroles du morceau `.0`
    pub lyrics: Option<(u64, crate::lyrics::Lyrics)>,
    /// défilement manuel des paroles (décalage, instant) : suspend le suivi auto quelques secondes
    pub ly_manual: Option<(f32, Instant)>,
}

#[derive(Clone, Copy)]
pub struct WebState {
    pub song: u64,
    pub playing: bool,
    pub pos: f32,
    pub dur: f32,
    pub loading: bool,
    pub at: Instant,
}

impl Default for WebState {
    fn default() -> Self {
        Self { song: 0, playing: false, pos: 0.0, dur: 0.0, loading: false, at: Instant::now() }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        crate::ui::setup_style(&ctx);
        let cfg = Config::load();
        let api = crate::api::Api::new(&cfg.token);
        let (tx, rx) = channel();
        let pool = Pool::new(api.clone(), tx.clone(), ctx.clone());
        let (audio, audio_err) = match Audio::new(cfg.volume) {
            Ok(a) => (Some(a), String::new()),
            Err(e) => (None, e),
        };
        let imgs = Images::new(pool.clone(), ctx.clone());
        api.set_user(cfg.user_id);
        let logged = !cfg.token.is_empty() || cfg.user_id != 0;
        let volume = cfg.volume;
        let mut app = App {
            cfg,
            api,
            pool,
            tx,
            rx,
            ctx,
            imgs,
            logged,
            login_busy: false,
            login_err: String::new(),
            guest_input: String::new(),
            route: Route::Home,
            back: vec![],
            fwd: vec![],
            page: PageState::default(),
            lib: Library::default(),
            carousel: HashMap::new(),
            page_t0: Instant::now(),
            clip_checked: false,
            audio,
            audio_err,
            queue: vec![],
            orig: vec![],
            qi: 0,
            cur: None,
            loading: false,
            shuffle: false,
            repeat: Repeat::Off,
            volume,
            vol_before_mute: 0.8,
            want: None,
            pcache: HashMap::new(),
            porder: VecDeque::new(),
            requested: HashSet::new(),
            seeking: None,
            web: None,
            web_want: None,
            web_login: false,
            web_user: 0,
            web_offer: String::new(),
            ws: WebState::default(),
            web_started: false,
            web_song: 0,
            web_prev: 0,
            gw_seq: 0,
            gw_pending: HashMap::new(),
            search: String::new(),
            show_queue: false,
            show_settings: false,
            new_playlist: None,
            toast: None,
            acts: vec![],
            last_title: String::new(),
            rng: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(88172645463325252)
                | 1,
            aurora: cc.gl.as_ref().and_then(|gl| crate::aurora::Aurora::new(gl)).map(Arc::new),
            aclock: Default::default(),
            pal_st: Default::default(),
            live: false,
            upd: crate::update::Updater::new(cc.egui_ctx.clone()),
            upd_t0: None,
            upd_toasted: false,
            now_playing: false,
            np_t0: Instant::now(),
            show_lyrics: false,
            lyrics: None,
            ly_manual: None,
        };
        if app.cfg.web {
            // session Deezer mémorisée par WebKit : on relance le lecteur officiel en arrière-plan
            app.web_want = Some((crate::web::ENGINE_URL, false));
        }
        if app.logged {
            app.start_session();
        }
        if app.cfg.auto_update {
            app.upd_t0 = Some(Instant::now());
            app.ctx.request_repaint_after(Duration::from_secs(5));
        }
        app
    }

    pub fn start_session(&mut self) {
        self.lib = Library::default();
        self.pool.job(Job::Library);
        self.go(Route::Home, false);
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
        self.ctx.request_repaint();
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    // ---------------------------------------------------------------- navigation

    pub fn go(&mut self, r: Route, push: bool) {
        if push && r != self.route {
            self.back.push(self.route.clone());
            self.fwd.clear();
        }
        self.route = r.clone();
        self.page_t0 = Instant::now();
        let g = self.pool.gen.fetch_add(1, Ordering::Relaxed) + 1;
        self.page = PageState::default();
        match r {
            Route::FavTracks | Route::Playlists | Route::Albums | Route::Artists | Route::Podcasts => {
                self.rebuild_lib_page()
            }
            _ => {
                self.page.loading = true;
                self.pool.job(Job::Route { route: r, gen: g });
            }
        }
        self.ctx.request_repaint();
    }

    pub fn rebuild_lib_page(&mut self) {
        let lib = &self.lib;
        let mut d = PageData::default();
        let mut sort = None;
        let grid = |title: &str, cards: &Vec<Card>| Section { title: title.to_owned(), cards: cards.clone(), style: Style::Grid };
        match self.route {
            Route::FavTracks => {
                let secs: u64 = lib.tracks.iter().map(|t| t.dur as u64).sum();
                d.header = Some(Header {
                    title: "Favourite tracks".into(),
                    line: lib.me_name.clone(),
                    meta: format!("{} tracks • {}", fmt_num(lib.tracks.len() as u64), fmt_total(secs)),
                    img: "fav".into(),
                    ..Default::default()
                });
                d.tracks = lib.tracks.clone();
                d.total = d.tracks.len() as u32;
                sort = Some((Col::Added, false));
            }
            Route::Playlists => {
                d.header = Some(Header { title: "Playlists".into(), meta: format!("{} playlists", lib.playlists.len()), ..Default::default() });
                d.sections.push(grid("", &lib.playlists));
            }
            Route::Albums => {
                d.header = Some(Header { title: "Albums".into(), meta: format!("{} albums", lib.albums.len()), ..Default::default() });
                d.sections.push(grid("", &lib.albums));
            }
            Route::Artists => {
                d.header = Some(Header { title: "Artists".into(), meta: format!("{} artists", lib.artists.len()), ..Default::default() });
                d.sections.push(grid("", &lib.artists));
            }
            Route::Podcasts => {
                d.header = Some(Header { title: "Podcasts".into(), meta: format!("{} podcasts", lib.podcasts.len()), ..Default::default() });
                d.sections.push(grid("", &lib.podcasts));
            }
            _ => return,
        }
        let keep_filter = std::mem::take(&mut self.page.filter);
        let keep_sort = self.page.sort.or(sort);
        self.page = PageState { data: d, loading: false, err: String::new(), view: vec![], dirty: true, sort: keep_sort, filter: keep_filter };
    }

    pub fn is_library_route(&self) -> bool {
        matches!(self.route, Route::FavTracks | Route::Playlists | Route::Albums | Route::Artists | Route::Podcasts)
    }

    // ---------------------------------------------------------------- lecteur

    pub fn play_list(&mut self, tracks: Vec<Track>, start: usize, shuffle: bool) {
        if tracks.is_empty() {
            self.toast("Nothing to play");
            return;
        }
        self.orig = tracks.clone();
        self.queue = tracks;
        self.qi = start.min(self.queue.len() - 1);
        self.shuffle = shuffle;
        if shuffle {
            let n = self.queue.len();
            let s = if start == 0 { (self.rand() % n as u64) as usize } else { start.min(n - 1) };
            self.shuffle_rest(s);
        }
        self.play_index(self.qi);
    }

    /// Mélange la queue en gardant le morceau courant en tête.
    fn shuffle_rest(&mut self, keep: usize) {
        let n = self.queue.len();
        let first = self.queue.swap_remove(keep.min(n - 1));
        for i in (1..self.queue.len()).rev() {
            let j = (self.rand() % (i as u64 + 1)) as usize;
            self.queue.swap(i, j);
        }
        self.queue.insert(0, first);
        self.qi = 0;
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.queue.is_empty() {
            return;
        }
        if self.shuffle {
            self.orig = self.queue.clone();
            let keep = self.qi;
            self.shuffle_rest(keep);
        } else if let Some(cur) = self.queue.get(self.qi).map(|t| t.id) {
            self.queue = std::mem::take(&mut self.orig);
            self.qi = self.queue.iter().position(|t| t.id == cur).unwrap_or(0);
        }
    }

    pub fn play_index(&mut self, i: usize) {
        let n = self.queue.len();
        if n == 0 {
            return;
        }
        if self.use_web() {
            let idx = i.min(n - 1);
            self.qi = idx;
            let t = self.queue[idx].clone();
            self.cur = Some(t.clone());
            self.want = None;
            self.loading = true;
            self.web_started = false;
            self.web_prev = if self.ws.song == t.id { 0 } else { self.ws.song };
            self.web_song = t.id;
            if let Some(a) = self.audio.as_mut() {
                a.stop();
            }
            if let Some(w) = &self.web {
                w.control("setVolume", &format!("{}", self.volume));
                w.play(t.id);
            }
            self.ctx.request_repaint();
            return;
        }
        let mut idx = i.min(n - 1);
        let mut tries = 0;
        while self.queue[idx].preview.is_empty() {
            idx = (idx + 1) % n;
            tries += 1;
            if tries >= n {
                self.toast("No playable preview in this list");
                return;
            }
        }
        self.qi = idx;
        let t = self.queue[idx].clone();
        self.cur = Some(t.clone());
        if let Some(b) = self.pcache.get(&t.id).cloned() {
            self.want = None;
            self.loading = false;
            self.start_audio(b);
        } else {
            self.want = Some(t.id);
            self.loading = true;
            if let Some(a) = self.audio.as_mut() {
                a.stop();
            }
            self.request_preview(&t);
        }
        self.ctx.request_repaint();
    }

    fn request_preview(&mut self, t: &Track) {
        if t.preview.is_empty() || self.pcache.contains_key(&t.id) || !self.requested.insert(t.id) {
            return;
        }
        self.pool.job(Job::Preview { id: t.id, url: t.preview.clone() });
    }

    fn start_audio(&mut self, bytes: Arc<Vec<u8>>) {
        let r = self.audio.as_mut().map(|a| a.play(bytes));
        match r {
            Some(Err(e)) => self.toast(format!("Audio error: {e}")),
            None => self.toast("No audio output device"),
            _ => {}
        }
        // précharge le suivant
        if let Some(i) = self.next_index(false) {
            let t = self.queue[i].clone();
            self.request_preview(&t);
        }
    }

    fn next_index(&self, manual: bool) -> Option<usize> {
        let n = self.queue.len();
        if n == 0 {
            return None;
        }
        if !manual && self.repeat == Repeat::One {
            return Some(self.qi);
        }
        if self.qi + 1 < n {
            Some(self.qi + 1)
        } else if self.repeat == Repeat::All || manual && n > 1 {
            Some(0)
        } else {
            None
        }
    }

    pub fn next(&mut self, manual: bool) {
        match self.next_index(manual) {
            Some(i) => self.play_index(i),
            None => {
                if let Some(a) = self.audio.as_mut() {
                    a.stop();
                }
                self.web_pause();
            }
        }
    }

    pub fn prev(&mut self) {
        let pos = self.play_pos().0;
        if pos > Duration::from_secs(3) || self.qi == 0 {
            if self.use_web() {
                self.apply(Act::Seek(0.0));
            } else if let Some(a) = &self.audio {
                a.seek(Duration::ZERO);
            }
        } else {
            self.play_index(self.qi - 1);
        }
    }

    pub fn toggle(&mut self) {
        if self.use_web() {
            if self.cur.is_none() {
                return;
            }
            if self.ws.playing {
                self.web_pause();
                self.ws.playing = false;
            } else if self.web_started {
                if let Some(w) = &self.web {
                    w.control("play", "");
                }
                self.ws.playing = true;
                self.ws.at = Instant::now();
            } else {
                self.play_index(self.qi);
            }
            return;
        }
        let has = self.audio.as_ref().map(|a| a.has_player()).unwrap_or(false);
        if has {
            if let Some(a) = &self.audio {
                a.toggle();
            }
        } else if self.cur.is_some() {
            self.play_index(self.qi);
        }
    }

    pub fn use_web(&self) -> bool {
        self.web.is_some() && self.web_user > 0
    }

    fn web_pause(&self) {
        if let Some(w) = &self.web {
            w.control("pause", "");
        }
    }

    /// (position, durée) du morceau courant, quel que soit le moteur.
    pub fn play_pos(&self) -> (Duration, Duration) {
        if self.use_web() {
            if self.ws.song != 0 && self.ws.song == self.web_song {
                let mut p = self.ws.pos;
                if self.ws.playing {
                    p += self.ws.at.elapsed().as_secs_f32();
                }
                let d = self.ws.dur.max(0.0);
                return (Duration::from_secs_f32(p.clamp(0.0, d.max(0.0))), Duration::from_secs_f32(d));
            }
            return (Duration::ZERO, self.cur.as_ref().map(|t| Duration::from_secs(t.dur as u64)).unwrap_or_default());
        }
        self.audio.as_ref().map(|a| (a.pos(), a.len)).unwrap_or_default()
    }

    pub fn is_playing(&self) -> bool {
        if self.use_web() {
            return self.ws.playing;
        }
        self.audio.as_ref().map(|a| a.has_player() && !a.paused()).unwrap_or(false)
    }

    // ---------------------------------------------------------------- messages

    pub fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Img(url, img) => self.imgs.on_loaded(url, img),
                Msg::Preview(id, r) => {
                    self.requested.remove(&id);
                    match r {
                        Ok(b) => {
                            self.pcache.insert(id, b.clone());
                            self.porder.push_back(id);
                            while self.porder.len() > 6 {
                                if let Some(old) = self.porder.pop_front() {
                                    self.pcache.remove(&old);
                                }
                            }
                            if self.want == Some(id) {
                                self.want = None;
                                self.loading = false;
                                self.start_audio(b);
                            }
                        }
                        Err(e) => {
                            if self.want == Some(id) {
                                self.want = None;
                                self.loading = false;
                                self.toast(format!("Could not load preview: {e}"));
                            }
                        }
                    }
                }
                Msg::Page(g, l) => {
                    if g != self.pool.gen.load(Ordering::Relaxed) {
                        continue;
                    }
                    match l {
                        Loaded::Page(d) => {
                            let default_sort = d.header.as_ref().map(|h| h.kind == Some(Kind::Playlist)).unwrap_or(false);
                            let _ = default_sort;
                            self.page.data = d;
                            self.page_t0 = Instant::now();
                            self.page.loading = false;
                            self.page.dirty = true;
                        }
                        Loaded::More(v) => {
                            self.page.data.tracks.extend(v);
                            self.page.dirty = true;
                        }
                        Loaded::Lib(_) => {}
                    }
                }
                Msg::PageErr(g, e) => {
                    if g == self.pool.gen.load(Ordering::Relaxed) {
                        self.page.loading = false;
                        if !self.cfg.token.is_empty() && (e.contains("OAuth") || e.contains("token")) {
                            self.session_expired();
                        }
                        self.page.err = e;
                    }
                }
                Msg::Lib(p) => self.on_lib(p),
                Msg::Play(t, shuffle) => self.play_list(t, 0, shuffle),
                Msg::Toast(s) => self.toast(s),
                Msg::Lyrics(id, l) => self.lyrics = Some((id, l)),
                Msg::Login(r) => {
                    self.login_busy = false;
                    match r {
                        Ok(tok) => {
                            self.cfg.token = tok;
                            self.cfg.save();
                            self.api.set_token(&self.cfg.token);
                            self.logged = true;
                            self.login_err.clear();
                            self.start_session();
                        }
                        Err(e) => self.login_err = e,
                    }
                }
                Msg::Mutated { msg, refresh } => {
                    self.toast(msg);
                    if refresh {
                        self.pool.job(Job::Library);
                        self.lib.albums.clear();
                        self.lib.artists.clear();
                        self.lib.playlists.clear();
                        self.lib.podcasts.clear();
                        self.lib.tracks.clear();
                        self.lib.tracks_done = false;
                    }
                }
            }
        }
    }

    fn session_expired(&mut self) {
        self.logged = false;
        self.cfg.token.clear();
        self.cfg.user_id = 0;
        self.api.set_user(0);
        self.cfg.save();
        self.api.set_token("");
        self.login_err = "Session expired, please log in again".into();
    }

    fn on_lib(&mut self, p: LibPart) {
        match p {
            LibPart::Me { id, name, img } => {
                self.lib.me_id = id;
                self.lib.me_name = name;
                self.lib.me_img = img;
                self.pool.me.store(id, Ordering::Relaxed);
            }
            LibPart::Tracks(v) => {
                self.lib.fav.extend(v.iter().map(|t| t.id));
                self.lib.tracks.extend(v);
            }
            LibPart::TracksDone => self.lib.tracks_done = true,
            LibPart::Albums(v) => {
                self.lib.album_ids = v.iter().map(|c| c.id).collect();
                self.lib.albums = v;
            }
            LibPart::Artists(v) => {
                self.lib.artist_ids = v.iter().map(|c| c.id).collect();
                self.lib.artists = v;
            }
            LibPart::Playlists(v) => {
                self.lib.playlist_ids = v.iter().map(|c| c.id).collect();
                self.lib.playlists = v;
            }
            LibPart::Podcasts(v) => self.lib.podcasts = v,
        }
        if self.is_library_route() {
            self.rebuild_lib_page();
        }
    }

    // ---------------------------------------------------------------- actions

    pub fn apply(&mut self, a: Act) {
        match a {
            Act::Go(r) => self.go(r, true),
            Act::Back => {
                if let Some(r) = self.back.pop() {
                    self.fwd.push(self.route.clone());
                    self.go(r, false);
                }
            }
            Act::Forward => {
                if let Some(r) = self.fwd.pop() {
                    self.back.push(self.route.clone());
                    self.go(r, false);
                }
            }
            Act::PlayList { tracks, start, shuffle } => self.play_list(tracks, start, shuffle),
            Act::PlayContent(k, id, sh) => {
                if k == Kind::Track {
                    return;
                }
                self.toast("Loading…");
                self.pool.job(Job::PlayContent { kind: k, id, shuffle: sh });
            }
            Act::Flow => {
                self.toast("Starting Flow…");
                self.pool.job(Job::Flow)
            }
            Act::ArtistRadio(id) => self.pool.job(Job::ArtistRadio(id)),
            Act::TrackRadio(id) => self.pool.job(Job::TrackRadio(id)),
            Act::Enqueue(t) => {
                self.orig.push(t.clone());
                self.queue.push(t);
                if self.cur.is_none() {
                    self.play_index(0);
                }
                self.toast("Added to queue");
            }
            Act::PlayNext(t) => {
                if self.queue.is_empty() {
                    self.play_list(vec![t], 0, false);
                } else {
                    self.queue.insert(self.qi + 1, t);
                    self.toast("Will play next");
                }
            }
            Act::Like(_) | Act::Follow(..) | Act::AddToPlaylist { .. } | Act::RemoveFromPlaylist { .. } | Act::NewPlaylist(_) | Act::DeletePlaylist(_)
                if self.cfg.token.is_empty() && !self.use_web() =>
            {
                self.toast("Read-only mode: log in with your Deezer account");
            }
            Act::Like(t) => {
                let on = !self.lib.fav.contains(&t.id);
                if on {
                    self.lib.fav.insert(t.id);
                    let mut t2 = t.clone();
                    t2.added = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    self.lib.tracks.push(t2);
                } else {
                    self.lib.fav.remove(&t.id);
                    self.lib.tracks.retain(|x| x.id != t.id);
                }
                if self.route == Route::FavTracks {
                    self.rebuild_lib_page();
                }
                self.mutate(Mut::LikeTrack(t.id, on));
            }
            Act::Follow(k, id, on) => {
                let set = match k {
                    Kind::Album => Some(&mut self.lib.album_ids),
                    Kind::Artist => Some(&mut self.lib.artist_ids),
                    Kind::Playlist => Some(&mut self.lib.playlist_ids),
                    _ => None,
                };
                if let Some(s) = set {
                    if on {
                        s.insert(id);
                    } else {
                        s.remove(&id);
                    }
                }
                let m = match k {
                    Kind::Album => Mut::FollowAlbum(id, on),
                    Kind::Artist => Mut::FollowArtist(id, on),
                    Kind::Playlist => Mut::FollowPlaylist(id, on),
                    _ => return,
                };
                self.mutate(m);
            }
            Act::AddToPlaylist { pl, track } => self.mutate(Mut::AddToPlaylist { pl, track }),
            Act::RemoveFromPlaylist { pl, track } => {
                self.page.data.tracks.retain(|t| t.id != track);
                self.page.dirty = true;
                self.mutate(Mut::RemoveFromPlaylist { pl, track });
            }
            Act::OpenNewPlaylist => self.new_playlist = Some(String::new()),
            Act::NewPlaylist(name) => self.mutate(Mut::CreatePlaylist(name)),
            Act::DeletePlaylist(id) => {
                self.mutate(Mut::DeletePlaylist(id));
                self.go(Route::Playlists, true);
            }
            Act::Toggle => self.toggle(),
            Act::Next => self.next(true),
            Act::Prev => self.prev(),
            Act::Seek(f) => {
                if self.use_web() {
                    let f = f.clamp(0.0, 1.0);
                    if let Some(w) = &self.web {
                        w.control("seek", &format!("{f}"));
                    }
                    self.ws.pos = self.ws.dur * f;
                    self.ws.at = Instant::now();
                } else if let Some(a) = &self.audio {
                    let to = a.len.mul_f32(f.clamp(0.0, 1.0));
                    a.seek(to);
                }
            }
            Act::Shuffle => self.toggle_shuffle(),
            Act::NowPlaying(on) => {
                if on && !self.now_playing {
                    self.np_t0 = Instant::now();
                }
                self.now_playing = on && self.cur.is_some();
                self.ly_manual = None;
            }
            Act::ToggleLyrics => {
                if !self.now_playing {
                    self.now_playing = self.cur.is_some();
                    self.np_t0 = Instant::now();
                    self.show_lyrics = true;
                } else {
                    self.show_lyrics = !self.show_lyrics;
                }
                self.ly_manual = None;
            }
            Act::Repeat => {
                self.repeat = match self.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                }
            }
            Act::Volume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                if self.volume > 0.0 {
                    self.vol_before_mute = self.volume;
                }
                if let Some(a) = self.audio.as_mut() {
                    a.set_volume(self.volume);
                }
                if let Some(w) = &self.web {
                    w.control("setVolume", &format!("{}", self.volume));
                }
                self.cfg.volume = self.volume;
            }
            Act::OpenWeb(u) => {
                let _ = open::that(u);
            }
            Act::CopyText(t) => {
                self.ctx.copy_text(t);
                self.toast("Link copied");
            }
            Act::Sort(c) => {
                self.page.sort = match self.page.sort {
                    Some((x, asc)) if x == c => {
                        if asc { Some((c, false)) } else { None }
                    }
                    _ => Some((c, true)),
                };
                self.page.dirty = true;
            }
            Act::JumpQueue(i) => self.play_index(i),
            Act::ClearQueue => {
                self.queue.clear();
                self.orig.clear();
                self.qi = 0;
                self.cur = None;
                if let Some(a) = self.audio.as_mut() {
                    a.stop();
                }
                self.web_pause();
            }
            Act::WebLogin => {
                self.login_err.clear();
                self.web_login = true;
                match &mut self.web {
                    Some(w) => w.load(crate::web::LOGIN_URL),
                    None => self.web_want = Some((crate::web::LOGIN_URL, true)),
                }
            }
            Act::CancelWebLogin => {
                self.web_login = false;
                if self.web_user == 0 {
                    self.web = None;
                    self.web_want = None;
                }
            }
            Act::PasteLogin => {
                self.guest_input = clipboard_text();
                self.apply(Act::GuestLogin);
            }
            Act::GuestLogin => {
                if self.guest_input.trim().is_empty() {
                    self.guest_input = clipboard_text();
                }
                let txt = self.guest_input.trim().to_owned();
                // accepte "123456789" ou "https://www.deezer.com/fr/profile/123456789"
                let digits: String = match txt.rfind("profile/") {
                    Some(i) => txt[i + 8..].chars().take_while(|c| c.is_ascii_digit()).collect(),
                    None => txt.chars().filter(|c| c.is_ascii_digit()).collect(),
                };
                match digits.parse::<u64>() {
                    Ok(id) if id > 0 => {
                        self.cfg.user_id = id;
                        self.cfg.save();
                        self.api.set_user(id);
                        self.logged = true;
                        self.login_err.clear();
                        self.start_session();
                    }
                    _ => self.login_err = "Could not find a profile ID in that text".into(),
                }
            }
            Act::Login => {
                if self.login_busy {
                    return;
                }
                self.login_busy = true;
                self.login_err.clear();
                self.cfg.save();
                spawn_login(self.cfg.clone(), self.tx.clone(), self.ctx.clone());
            }
            Act::Logout => {
                if let Some(w) = self.web.take() {
                    w.control("pause", "");
                    w.clear();
                }
                self.web_user = 0;
                self.web_want = None;
                self.web_login = false;
                self.cfg.web = false;
                self.cfg.arl.clear();
                self.session_expired();
                self.login_err.clear();
                if let Some(a) = self.audio.as_mut() {
                    a.stop();
                }
                self.queue.clear();
                self.cur = None;
                self.lib = Library::default();
                self.show_settings = false;
            }
            Act::RefreshLib => {
                self.lib = Library { me_id: self.lib.me_id, me_name: self.lib.me_name.clone(), me_img: self.lib.me_img.clone(), ..Default::default() };
                self.pool.job(Job::Library);
                let r = self.route.clone();
                self.go(r, false);
            }
            Act::ClearImgCache => {
                self.imgs.clear();
                let _ = std::fs::remove_dir_all(crate::config::cache_dir().join("img"));
                let _ = std::fs::create_dir_all(crate::config::cache_dir().join("img"));
                self.toast("Image cache cleared");
            }
            Act::SaveConfig => self.cfg.save(),
        }
    }

    // ---------------------------------------------------------------- session web Deezer

    /// Écriture dans la bibliothèque : API OAuth si token, sinon passerelle du site (session web).
    fn mutate(&mut self, m: Mut) {
        if !self.cfg.token.is_empty() || !self.use_web() {
            self.pool.job(Job::Mutate(m));
            return;
        }
        use serde_json::json;
        let s = |x: u64| x.to_string();
        let (method, params, msg, refresh) = match m {
            Mut::LikeTrack(id, on) => (
                if on { "favorite_song.add" } else { "favorite_song.remove" },
                json!({"SNG_ID": s(id)}),
                if on { "Added to Favourite tracks" } else { "Removed from Favourite tracks" },
                false,
            ),
            Mut::FollowAlbum(id, on) => (
                if on { "album.addFavorite" } else { "album.deleteFavorite" },
                json!({"ALB_ID": s(id)}),
                if on { "Album added to your library" } else { "Album removed" },
                true,
            ),
            Mut::FollowArtist(id, on) => (
                if on { "artist.addFavorite" } else { "artist.deleteFavorite" },
                json!({"ART_ID": s(id)}),
                if on { "Artist followed" } else { "Artist unfollowed" },
                true,
            ),
            Mut::FollowPlaylist(id, on) => (
                if on { "playlist.addFavorite" } else { "playlist.deleteFavorite" },
                if on { json!({"PARENT_PLAYLIST_ID": s(id)}) } else { json!({"PLAYLIST_ID": s(id)}) },
                if on { "Playlist added to your library" } else { "Playlist removed" },
                true,
            ),
            Mut::CreatePlaylist(title) => {
                ("playlist.create", json!({"title": title, "status": 0, "description": "", "songs": []}), "Playlist created", true)
            }
            Mut::AddToPlaylist { pl, track } => (
                "playlist.addSongs",
                json!({"playlist_id": s(pl), "songs": [[s(track), 0]], "offset": -1}),
                "Added to playlist",
                true,
            ),
            Mut::RemoveFromPlaylist { pl, track } => (
                "playlist.deleteSongs",
                json!({"playlist_id": s(pl), "songs": [[s(track), 0]]}),
                "Removed from playlist",
                false,
            ),
            Mut::DeletePlaylist(pl) => ("playlist.delete", json!({"playlist_id": s(pl)}), "Playlist deleted", true),
        };
        self.gw_seq += 1;
        let id = self.gw_seq;
        self.gw_pending.insert(id, (msg.to_owned(), refresh));
        if let Some(w) = &self.web {
            w.gw(id, method, &params);
        }
    }

    fn pump_web(&mut self) {
        use crate::web::WebMsg;
        loop {
            let Some(m) = self.web.as_ref().and_then(|w| w.try_recv()) else { break };
            match m {
                WebMsg::User { id, name, offer, .. } => {
                    if std::env::var_os("DRS_DEBUG").is_some() {
                        eprintln!("web user id={id} login_flow={}", self.web_login);
                    }
                    if id > 0 {
                        let first = self.web_user != id;
                        self.web_user = id;
                        self.web_offer = offer.clone();
                        if self.web_login || !self.cfg.web || self.cfg.user_id != id || !self.logged {
                            self.web_login = false;
                            self.cfg.web = true;
                            self.cfg.user_id = id;
                            self.cfg.save();
                            self.api.set_user(id);
                            self.logged = true;
                            self.login_err.clear();
                            self.start_session();
                        }
                        if let Some(w) = self.web.as_mut() {
                            w.show(false);
                        }
                        if first {
                            let o = if offer.is_empty() { String::new() } else { format!(" ({offer})") };
                            self.toast(format!("Connected as {name}{o}"));
                        }
                    } else if !self.web_login {
                        // session expirée / déconnecté côté Deezer : on réaffiche la page de connexion
                        if !self.cfg.arl.is_empty() {
                            self.cfg.arl.clear();
                            self.cfg.save();
                        }
                        self.web_user = 0;
                        self.web_login = true;
                        if let Some(w) = &self.web {
                            w.load(crate::web::LOGIN_URL);
                        }
                    }
                }
                WebMsg::State { song, playing, pos, dur, loading } => {
                    let prev = std::mem::replace(
                        &mut self.ws,
                        WebState { song, playing, pos, dur, loading, at: Instant::now() },
                    );
                    if self.cur.is_none() || !self.use_web() {
                        continue;
                    }
                    // Deezer remplace parfois le titre demandé par une autre version (autre id) :
                    // le premier morceau lancé après notre demande est le nôtre.
                    if !self.web_started && song != 0 && song != self.web_song && song != self.web_prev && (playing || loading) {
                        self.web_song = song;
                    }
                    let cur = self.web_song;
                    let mut ended = false;
                    if song == cur {
                        if playing {
                            self.web_started = true;
                            self.loading = false;
                        }
                        let near_end = prev.song == cur && prev.dur > 0.0 && prev.pos >= prev.dur - 1.5;
                        if self.web_started && !playing && !loading && dur > 0.0 && (pos >= dur - 1.0 || near_end && pos < 1.0) {
                            ended = true;
                        }
                    } else if self.web_started && song != 0 {
                        // le lecteur Deezer est passé à autre chose (fin de notre morceau)
                        ended = true;
                    }
                    if ended {
                        self.web_started = false;
                        self.next(false);
                    }
                }
                WebMsg::Gw { id, err } => {
                    if let Some((msg, refresh)) = self.gw_pending.remove(&id) {
                        if err.is_empty() {
                            let _ = self.tx.send(Msg::Mutated { msg, refresh });
                        } else {
                            self.toast(format!("Deezer error: {}", err.chars().take(120).collect::<String>()));
                        }
                    }
                }
                WebMsg::Page => {}
                WebMsg::Arl(v) => {
                    // copie de la session : plus besoin de se reconnecter au prochain lancement
                    if !v.is_empty() && v != self.cfg.arl {
                        self.cfg.arl = v;
                        self.cfg.save();
                    }
                }
                WebMsg::NoPlayer => {
                    self.loading = false;
                    self.toast("Deezer player not ready, retrying…");
                    if let Some(w) = &self.web {
                        w.load(crate::web::ENGINE_URL);
                    }
                }
            }
        }
    }

    /// Appelé à chaque frame (même fenêtre cachée) : messages, fin de morceau, repaint adaptatif.
    pub fn tick(&mut self) {
        self.pump();
        self.pump_web();
        // paroles : chargées à la demande pour le morceau en cours (vue Now Playing + paroles ouvertes)
        if self.now_playing && self.show_lyrics {
            if let Some(id) = self.cur.as_ref().map(|t| t.id) {
                if self.lyrics.as_ref().map(|l| l.0) != Some(id) {
                    self.lyrics = Some((id, crate::lyrics::Lyrics::Loading));
                    self.ly_manual = None;
                    let (arl, tx, ctx) = (if self.use_web() { self.cfg.arl.clone() } else { String::new() }, self.tx.clone(), self.ctx.clone());
                    std::thread::spawn(move || {
                        let l = crate::lyrics::fetch(&arl, id);
                        let _ = tx.send(Msg::Lyrics(id, l));
                        ctx.request_repaint();
                    });
                }
            }
        }
        if self.cur.is_none() {
            self.now_playing = false;
        }
        // mise à jour : vérification discrète quelques secondes après le lancement
        if self.upd_t0.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
            self.upd_t0 = None;
            self.upd.check();
        }
        if !self.upd_toasted {
            if let crate::update::State::Available { ver, .. } = self.upd.get() {
                self.upd_toasted = true;
                self.toast(format!("Update {ver} available — Settings to install"));
            }
        }
        if self.use_web() {
        } else if let Some(a) = &self.audio {
            if a.has_player() && a.finished() {
                self.next(false);
            }
        }
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(300));
            }
        }
        if self.is_playing() || self.loading {
            let focused = self.ctx.input(|i| i.focused);
            self.ctx.request_repaint_after(Duration::from_millis(if focused { 250 } else { 1000 }));
        }
        // titre de fenêtre
        let title = match &self.cur {
            Some(t) if self.is_playing() => format!("{} • {}", t.title, t.artist),
            _ => "Deezer".into(),
        };
        if title != self.last_title {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

/// Lit le presse-papiers (macOS : pbpaste). Vide si indisponible.
pub fn clipboard_text() -> String {
    std::process::Command::new("pbpaste")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().chars().take(300).collect())
        .unwrap_or_default()
}
