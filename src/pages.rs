//! Chargement des pages (accueil, explorer, artiste, album, playlist, recherche…) et de la bibliothèque.
//! Tout tourne dans des threads workers ; les résultats sont poussés via `emit`.

use crate::api::{Api, Res};
use crate::model::*;
use serde_json::Value;

pub enum LibPart {
    Me { id: u64, name: String, img: String },
    Tracks(Vec<Track>),
    TracksDone,
    Albums(Vec<Card>),
    Artists(Vec<Card>),
    Playlists(Vec<Card>),
    Podcasts(Vec<Card>),
}

pub enum Loaded {
    Page(PageData),
    More(Vec<Track>),
    Lib(LibPart),
}

pub struct Loader<'a> {
    pub api: &'a Api,
    pub me: u64,
    pub emit: &'a (dyn Fn(Loaded) + Send + Sync),
    pub alive: &'a (dyn Fn() -> bool + Send + Sync),
}

fn lim(n: u32) -> (&'static str, String) {
    ("limit", n.to_string())
}

fn cards(kind: Kind, v: &Value) -> Vec<Card> {
    data(v).iter().filter_map(|x| Card::from_json(kind, x)).collect()
}

fn any_cards(v: &Value) -> Vec<Card> {
    data(v).iter().filter_map(Card::from_any).collect()
}

fn tracks(v: &Value, fb: Option<&AlbumFb>) -> Vec<Track> {
    data(v).iter().filter_map(|x| Track::from_json(x, fb)).collect()
}

fn sec(title: &str, cards: Vec<Card>, style: Style) -> Option<Section> {
    (!cards.is_empty()).then(|| Section { title: title.into(), cards, style })
}

/// Exécute des closures en parallèle (threads scopés) et garde l'ordre.
fn par<T: Send>(jobs: Vec<Box<dyn FnOnce() -> T + Send + '_>>) -> Vec<T> {
    std::thread::scope(|s| {
        let hs: Vec<_> = jobs.into_iter().map(|j| s.spawn(j)).collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

impl<'a> Loader<'a> {
    fn get(&self, p: &str, q: &[(&str, String)]) -> Res<Value> {
        self.api.get(p, q)
    }

    /// Charge une liste de morceaux paginée : 1ère page -> `Page`, le reste -> `More`.
    fn paged_tracks(
        &self,
        path: &str,
        extra: &[(&str, String)],
        page: u32,
        fb: Option<&AlbumFb>,
        mut base: PageData,
    ) -> Res<()> {
        let mut q: Vec<(&str, String)> = extra.to_vec();
        q.push(lim(page));
        q.push(("index", "0".into()));
        let v = self.get(path, &q)?;
        let total = n(&v, "total") as u32;
        let mut got = tracks(&v, fb);
        base.total = total.max(got.len() as u32);
        base.tracks = std::mem::take(&mut got);
        let mut idx = data(&v).len() as u32;
        let first_len = base.tracks.len();
        (self.emit)(Loaded::Page(base));
        if first_len == 0 {
            return Ok(());
        }
        while idx < total && (self.alive)() {
            let mut q: Vec<(&str, String)> = extra.to_vec();
            q.push(lim(page));
            q.push(("index", idx.to_string()));
            let v = self.get(path, &q)?;
            let got = tracks(&v, fb);
            let raw = data(&v).len() as u32;
            if raw == 0 {
                break;
            }
            idx += raw;
            (self.emit)(Loaded::More(got));
        }
        Ok(())
    }

    pub fn library(&self) {
        let me = self.api.me_path();
        let emit = |p| (self.emit)(Loaded::Lib(p));
        if let Ok(me) = self.get(&me, &[]) {
            emit(LibPart::Me { id: n(&me, "id"), name: s(&me, "name"), img: s(&me, "picture_medium") });
        }
        // Tout en parallèle sauf les favoris (pagination séquentielle).
        std::thread::scope(|sc| {
            sc.spawn(|| {
                if let Ok(v) = self.get(&format!("{}/albums", me), &[lim(1000)]) {
                    emit(LibPart::Albums(cards(Kind::Album, &v)));
                }
            });
            sc.spawn(|| {
                if let Ok(v) = self.get(&format!("{}/artists", me), &[lim(1000)]) {
                    emit(LibPart::Artists(cards(Kind::Artist, &v)));
                }
            });
            sc.spawn(|| {
                if let Ok(v) = self.get(&format!("{}/playlists", me), &[lim(1000)]) {
                    emit(LibPart::Playlists(cards(Kind::Playlist, &v)));
                }
            });
            sc.spawn(|| {
                if let Ok(v) = self.get(&format!("{}/podcasts", me), &[lim(500)]) {
                    emit(LibPart::Podcasts(cards(Kind::Podcast, &v)));
                }
            });
            let mut idx = 0u32;
            loop {
                let Ok(v) = self.get(&format!("{}/tracks", me), &[lim(500), ("index", idx.to_string())]) else { break };
                let raw = data(&v).len() as u32;
                emit(LibPart::Tracks(tracks(&v, None)));
                idx += raw;
                if raw == 0 || idx >= n(&v, "total") as u32 {
                    break;
                }
            }
            emit(LibPart::TracksDone);
        });
    }

    pub fn route(&self, r: &Route) -> Res<()> {
        match r {
            Route::Home => self.home(),
            Route::Explore => self.explore(),
            Route::Search(q) => self.search(q),
            Route::Artist(id) => self.artist(*id),
            Route::Album(id) => self.album(*id),
            Route::Playlist(id) => self.playlist(*id),
            Route::Genre(id) => self.genre(*id),
            Route::Radio(id) => self.radio(*id),
            Route::Podcast(id) => self.podcast(*id),
            // pages bibliothèque : rendues depuis le cache local
            _ => Ok(()),
        }
    }

    fn home_guest(&self) -> Res<()> {
        let me = self.api.me_path();
        let g = |p: String, extra: Vec<(&'static str, String)>| move || -> Option<Value> { self.get(&p, &extra).ok() };
        let r = par(vec![
            Box::new(g(format!("{me}/flow"), vec![lim(20)])),
            Box::new(g("/chart/0/tracks".into(), vec![lim(20)])),
            Box::new(g("/editorial/0/releases".into(), vec![lim(20)])),
            Box::new(g("/chart/0/albums".into(), vec![lim(20)])),
            Box::new(g("/chart/0/artists".into(), vec![lim(20)])),
            Box::new(g("/chart/0/playlists".into(), vec![lim(20)])),
        ]);
        let get = |i: usize| r.get(i).and_then(|x| x.as_ref());
        let mut sections = vec![];
        if let Some(v) = get(0) {
            let cs = tracks(v, None).iter().map(Card::from_track).collect();
            sections.extend(sec("Flow for you", cs, Style::Carousel));
        }
        if let Some(v) = get(2) {
            sections.extend(sec("New releases", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(1) {
            let cs = tracks(v, None).iter().map(Card::from_track).collect();
            sections.extend(sec("Top tracks right now", cs, Style::Carousel));
        }
        if let Some(v) = get(3) {
            sections.extend(sec("Top albums", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(4) {
            sections.extend(sec("Top artists", cards(Kind::Artist, v), Style::Carousel));
        }
        if let Some(v) = get(5) {
            sections.extend(sec("Top playlists", cards(Kind::Playlist, v), Style::Carousel));
        }
        (self.emit)(Loaded::Page(PageData { sections, ..Default::default() }));
        Ok(())
    }

    fn home(&self) -> Res<()> {
        if !self.api.has_token() {
            return self.home_guest();
        }
        let g = |p: &'static str, extra: Vec<(&'static str, String)>| {
            move || -> Option<Value> { self.get(p, &extra).ok() }
        };
        let r = par(vec![
            Box::new(g("/user/me/history", vec![lim(30)])),
            Box::new(g("/user/me/recommendations/playlists", vec![lim(12)])),
            Box::new(g("/user/me/recommendations/radios", vec![lim(12)])),
            Box::new(g("/user/me/recommendations/albums", vec![lim(20)])),
            Box::new(g("/user/me/recommendations/artists", vec![lim(20)])),
            Box::new(g("/user/me/recommendations/releases", vec![lim(20)])),
            Box::new(g("/user/me/recommendations/tracks", vec![lim(20)])),
            Box::new(g("/chart/0/tracks", vec![lim(20)])),
        ]);
        let get = |i: usize| r.get(i).and_then(|x| x.as_ref());
        let mut sections = vec![];

        // Récemment écoutés : dédoublonnés par album
        if let Some(v) = get(0) {
            let mut seen = std::collections::HashSet::new();
            let cs: Vec<Card> = tracks(v, None)
                .into_iter()
                .filter(|t| seen.insert(t.album_id))
                .take(12)
                .map(|t| Card::from_track(&t))
                .collect();
            sections.extend(sec("Recently played", cs, Style::Carousel));
        }
        let mut mixes = vec![];
        if let Some(v) = get(2) {
            mixes.extend(cards(Kind::Radio, v));
        }
        if let Some(v) = get(1) {
            mixes.extend(cards(Kind::Playlist, v));
        }
        mixes.truncate(12);
        sections.extend(sec("Mixes inspired by...", mixes, Style::Tiles));
        if let Some(v) = get(6) {
            let cs = tracks(v, None).iter().map(Card::from_track).collect();
            sections.extend(sec("Tracks for you", cs, Style::Carousel));
        }
        if let Some(v) = get(3) {
            sections.extend(sec("Albums for you", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(4) {
            sections.extend(sec("Artists you may like", cards(Kind::Artist, v), Style::Carousel));
        }
        if let Some(v) = get(5) {
            sections.extend(sec("New releases for you", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(7) {
            let cs = tracks(v, None).iter().map(Card::from_track).collect();
            sections.extend(sec("Top tracks right now", cs, Style::Carousel));
        }
        (self.emit)(Loaded::Page(PageData { sections, ..Default::default() }));
        Ok(())
    }

    fn explore(&self) -> Res<()> {
        let g = |p: &'static str, extra: Vec<(&'static str, String)>| {
            move || -> Option<Value> { self.get(p, &extra).ok() }
        };
        let r = par(vec![
            Box::new(g("/editorial/0/releases", vec![lim(30)])),
            Box::new(g("/chart/0/tracks", vec![lim(30)])),
            Box::new(g("/chart/0/albums", vec![lim(30)])),
            Box::new(g("/chart/0/artists", vec![lim(30)])),
            Box::new(g("/chart/0/playlists", vec![lim(30)])),
            Box::new(g("/chart/0/podcasts", vec![lim(30)])),
            Box::new(g("/genre", vec![])),
            Box::new(g("/radio", vec![lim(100)])),
        ]);
        let get = |i: usize| r.get(i).and_then(|x| x.as_ref());
        let mut sections = vec![];
        if let Some(v) = get(6) {
            let mut cs = cards(Kind::Genre, v);
            cs.retain(|c| c.id != 0); // "All"
            sections.extend(sec("Genres", cs, Style::Carousel));
        }
        if let Some(v) = get(0) {
            sections.extend(sec("New releases", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(1) {
            let cs = tracks(v, None).iter().map(Card::from_track).collect();
            sections.extend(sec("Top tracks", cs, Style::Carousel));
        }
        if let Some(v) = get(2) {
            sections.extend(sec("Top albums", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(3) {
            sections.extend(sec("Top artists", cards(Kind::Artist, v), Style::Carousel));
        }
        if let Some(v) = get(4) {
            sections.extend(sec("Top playlists", cards(Kind::Playlist, v), Style::Carousel));
        }
        if let Some(v) = get(5) {
            sections.extend(sec("Top podcasts", cards(Kind::Podcast, v), Style::Carousel));
        }
        if let Some(v) = get(7) {
            sections.extend(sec("Radios", cards(Kind::Radio, v), Style::Carousel));
        }
        (self.emit)(Loaded::Page(PageData { sections, ..Default::default() }));
        Ok(())
    }

    fn search(&self, q: &str) -> Res<()> {
        let qs = |extra: u32| vec![("q", q.to_owned()), lim(extra)];
        let r = par(vec![
            Box::new(|| self.get("/search/track", &qs(50)).ok()),
            Box::new(|| self.get("/search/album", &qs(30)).ok()),
            Box::new(|| self.get("/search/artist", &qs(30)).ok()),
            Box::new(|| self.get("/search/playlist", &qs(30)).ok()),
            Box::new(|| self.get("/search/podcast", &qs(30)).ok()),
        ]);
        let get = |i: usize| r.get(i).and_then(|x| x.as_ref());
        let mut p = PageData {
            header: Some(Header { title: format!("Results for “{q}”"), ..Default::default() }),
            tracks_title: "Tracks".into(),
            preview: 5,
            ..Default::default()
        };
        if let Some(v) = get(0) {
            p.tracks = tracks(v, None);
            p.total = p.tracks.len() as u32;
        }
        if let Some(v) = get(2) {
            p.sections.extend(sec("Artists", cards(Kind::Artist, v), Style::Carousel));
        }
        if let Some(v) = get(1) {
            p.sections.extend(sec("Albums", cards(Kind::Album, v), Style::Carousel));
        }
        if let Some(v) = get(3) {
            p.sections.extend(sec("Playlists", cards(Kind::Playlist, v), Style::Carousel));
        }
        if let Some(v) = get(4) {
            p.sections.extend(sec("Podcasts", cards(Kind::Podcast, v), Style::Carousel));
        }
        (self.emit)(Loaded::Page(p));
        Ok(())
    }

    fn artist(&self, id: u64) -> Res<()> {
        let a = self.get(&format!("/artist/{id}"), &[])?;
        let base = format!("/artist/{id}");
        let r = par(vec![
            Box::new(|| self.get(&format!("{base}/top"), &[lim(50)]).ok()),
            Box::new(|| self.get(&format!("{base}/albums"), &[lim(200)]).ok()),
            Box::new(|| self.get(&format!("{base}/related"), &[lim(30)]).ok()),
            Box::new(|| self.get(&format!("{base}/playlists"), &[lim(30)]).ok()),
        ]);
        let get = |i: usize| r.get(i).and_then(|x| x.as_ref());
        let fans = n(&a, "nb_fan");
        let mut p = PageData {
            header: Some(Header {
                kind: Some(Kind::Artist),
                id,
                title: s(&a, "name"),
                meta: format!("{} fans • {} albums", fmt_num(fans), n(&a, "nb_album")),
                img: s(&a, "picture_big"),
                round: true,
                ..Default::default()
            }),
            tracks_title: "Popular tracks".into(),
            preview: 5,
            ..Default::default()
        };
        if let Some(v) = get(0) {
            p.tracks = tracks(v, None);
            p.total = p.tracks.len() as u32;
        }
        if let Some(v) = get(1) {
            // discographie découpée comme Deezer : albums, singles & EP, compilations (récent d'abord)
            let pick = |types: &[&str]| {
                let mut out: Vec<Card> = data(v)
                    .iter()
                    .filter(|x| types.contains(&x["record_type"].as_str().unwrap_or("album")))
                    .filter_map(|x| Card::from_json(Kind::Album, x))
                    .collect();
                out.sort_by(|a, b| b.extra_sort().cmp(&a.extra_sort()));
                out
            };
            p.sections.extend(sec("Albums", pick(&["album"]), Style::Carousel));
            p.sections.extend(sec("Singles & EPs", pick(&["single", "ep"]), Style::Carousel));
            p.sections.extend(sec("Compilations", pick(&["compile", "compilation"]), Style::Carousel));
        }
        if let Some(v) = get(3) {
            p.sections.extend(sec("Playlists featuring", cards(Kind::Playlist, v), Style::Carousel));
        }
        if let Some(v) = get(2) {
            p.sections.extend(sec("Fans also like", cards(Kind::Artist, v), Style::Carousel));
        }
        (self.emit)(Loaded::Page(p));
        Ok(())
    }

    fn album(&self, id: u64) -> Res<()> {
        let a = self.get(&format!("/album/{id}"), &[])?;
        let fb = AlbumFb { album: s(&a, "title"), album_id: id, img: s(&a, "cover_medium") };
        let aid = n(&a["artist"], "id");
        let header = Header {
            kind: Some(Kind::Album),
            id,
            title: s(&a, "title"),
            line: s(&a["artist"], "name"),
            line_id: aid,
            meta: format!(
                "{} • {} tracks • {}{}",
                fmt_date_iso(&s(&a, "release_date")),
                n(&a, "nb_tracks"),
                fmt_total(n(&a, "duration")),
                {
                    let l = s(&a, "label");
                    if l.is_empty() { String::new() } else { format!(" • {l}") }
                }
            ),
            img: s(&a, "cover_big"),
            ..Default::default()
        };
        let mut base = PageData { header: Some(header), tracks_title: "Tracks".into(), ..Default::default() };
        if aid != 0 {
            if let Ok(v) = self.get(&format!("/artist/{aid}/albums"), &[lim(50)]) {
                let mut cs = cards(Kind::Album, &v);
                cs.retain(|c| c.id != id);
                base.sections.extend(sec("More from this artist", cs, Style::Carousel));
            }
        }
        self.paged_tracks(&format!("/album/{id}/tracks"), &[], 500, Some(&fb), base)
    }

    fn playlist(&self, id: u64) -> Res<()> {
        let a = self.get(&format!("/playlist/{id}"), &[])?;
        let header = Header {
            kind: Some(Kind::Playlist),
            id,
            title: s(&a, "title"),
            line: s(&a["creator"], "name"),
            meta: format!("{} tracks • {} • {} fans", n(&a, "nb_tracks"), fmt_total(n(&a, "duration")), fmt_num(n(&a, "fans"))),
            desc: s(&a, "description"),
            img: s(&a, "picture_big"),
            owner_id: n(&a["creator"], "id"),
            ..Default::default()
        };
        let base = PageData { header: Some(header), tracks_title: "Tracks".into(), ..Default::default() };
        self.paged_tracks(&format!("/playlist/{id}/tracks"), &[], 300, None, base)
    }

    fn genre(&self, id: u64) -> Res<()> {
        let g = self.get(&format!("/genre/{id}"), &[])?;
        let r = par(vec![
            Box::new(|| self.get(&format!("/genre/{id}/artists"), &[]).ok()),
            Box::new(|| self.get(&format!("/genre/{id}/radios"), &[]).ok()),
        ]);
        let mut p = PageData {
            header: Some(Header {
                kind: Some(Kind::Genre),
                id,
                title: s(&g, "name"),
                img: s(&g, "picture_big"),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Some(Some(v)) = r.first() {
            p.sections.extend(sec("Artists", cards(Kind::Artist, v), Style::Carousel));
        }
        if let Some(Some(v)) = r.get(1) {
            p.sections.extend(sec("Radios", cards(Kind::Radio, v), Style::Carousel));
        }
        (self.emit)(Loaded::Page(p));
        Ok(())
    }

    fn radio(&self, id: u64) -> Res<()> {
        let r = self.get(&format!("/radio/{id}"), &[])?;
        let base = PageData {
            header: Some(Header {
                kind: Some(Kind::Radio),
                id,
                title: s(&r, "title"),
                desc: s(&r, "description"),
                img: s(&r, "picture_big"),
                ..Default::default()
            }),
            tracks_title: "Tracks".into(),
            ..Default::default()
        };
        self.paged_tracks(&format!("/radio/{id}/tracks"), &[], 100, None, base)
    }

    fn podcast(&self, id: u64) -> Res<()> {
        let r = self.get(&format!("/podcast/{id}"), &[])?;
        let fb = AlbumFb { album: s(&r, "title"), album_id: 0, img: s(&r, "picture_medium") };
        let base = PageData {
            header: Some(Header {
                kind: Some(Kind::Podcast),
                id,
                title: s(&r, "title"),
                desc: s(&r, "description"),
                img: s(&r, "picture_big"),
                ..Default::default()
            }),
            tracks_title: "Episodes".into(),
            ..Default::default()
        };
        self.paged_tracks(&format!("/podcast/{id}/episodes"), &[], 100, Some(&fb), base)
    }
}

impl Card {
    /// Clé de tri chronologique pour la discographie ("13/11/2026" -> "20261113").
    pub fn extra_sort(&self) -> String {
        let p: Vec<&str> = self.extra.split('/').collect();
        if p.len() == 3 { format!("{}{}{}", p[2], p[1], p[0]) } else { String::new() }
    }
}

/// Morceaux à jouer pour une action "play" sur un contenu (flow, radio d'artiste, etc.).
pub fn fetch_play_tracks(api: &Api, kind: Kind, id: u64) -> Res<Vec<Track>> {
    let (path, q): (String, Vec<(&str, String)>) = match kind {
        Kind::Track => return Ok(vec![]),
        Kind::Album => (format!("/album/{id}/tracks"), vec![lim(500)]),
        Kind::Playlist => (format!("/playlist/{id}/tracks"), vec![lim(500)]),
        Kind::Artist => (format!("/artist/{id}/top"), vec![lim(100)]),
        Kind::Radio => (format!("/radio/{id}/tracks"), vec![lim(100)]),
        Kind::Podcast => (format!("/podcast/{id}/episodes"), vec![lim(100)]),
        Kind::Genre => (format!("/genre/{id}/radios"), vec![]),
    };
    let v = api.get(&path, &q)?;
    if kind == Kind::Genre {
        // un genre -> la première radio
        let rid = data(&v).first().map(|r| n(r, "id")).unwrap_or(0);
        return fetch_play_tracks(api, Kind::Radio, rid);
    }
    let fb = if kind == Kind::Album {
        api.get(&format!("/album/{id}"), &[]).ok().map(|a| AlbumFb {
            album: s(&a, "title"),
            album_id: id,
            img: s(&a, "cover_medium"),
        })
    } else {
        None
    };
    Ok(tracks(&v, fb.as_ref()))
}

pub fn fetch_flow(api: &Api) -> Res<Vec<Track>> {
    let v = api.get(&format!("{}/flow", api.me_path()), &[lim(25)])?;
    Ok(tracks(&v, None))
}

pub fn fetch_artist_radio(api: &Api, id: u64) -> Res<Vec<Track>> {
    let v = api.get(&format!("/artist/{id}/radio"), &[lim(50)])?;
    Ok(tracks(&v, None))
}

pub fn fetch_track_radio(api: &Api, id: u64) -> Res<Vec<Track>> {
    let v = api.get(&format!("/track/{id}/radio"), &[lim(50)])?;
    Ok(tracks(&v, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn run(route: Route) -> Vec<PageData> {
        let api = Api::new("");
        let out = Mutex::new(vec![]);
        let emit = |l: Loaded| {
            if let Loaded::Page(p) = l {
                out.lock().unwrap().push(p);
            }
        };
        let alive = || true;
        let ld = Loader { api: &api, me: 0, emit: &emit, alive: &alive };
        ld.route(&route).expect("route ok");
        out.into_inner().unwrap()
    }

    #[test]
    fn public_pages_parse() {
        let s = &run(Route::Search("daft punk".into()))[0];
        assert!(!s.tracks.is_empty() && !s.tracks[0].preview.is_empty(), "search tracks");
        assert!(!s.sections.is_empty());
        let e = &run(Route::Explore)[0];
        assert!(e.sections.len() >= 4, "explore sections: {}", e.sections.len());
        let a = &run(Route::Artist(27))[0];
        assert!(a.header.as_ref().unwrap().title.contains("Daft"));
        assert!(!a.tracks.is_empty() && a.sections.len() >= 2);
        let al = &run(Route::Album(302127))[0];
        assert!(al.tracks.len() >= 10, "album tracks {}", al.tracks.len());
        assert!(!al.tracks[0].img.is_empty(), "album fallback cover");
        let g = &run(Route::Genre(132))[0];
        assert!(!g.sections.is_empty());
        println!("img size url: {}", model_img(&s.tracks[0].img));
    }

    fn model_img(u: &str) -> String {
        img_size(u, 96)
    }

    #[test]
    fn preview_plays_decodes() {
        let api = Api::new("");
        let s = api.get("/search/track", &[("q", "daft punk".into()), lim(1)]).unwrap();
        let t = Track::from_json(&data(&s)[0], None).unwrap();
        let bytes = crate::audio::clean_mp3(api.download(&t.preview, 4 << 20).expect("download preview"));
        println!("len={} head={:02x?} url={}", bytes.len(), &bytes[..16.min(bytes.len())], t.preview);
        assert!(bytes.len() > 100_000);
        let len = bytes.len() as u64;
        let dec = rodio::Decoder::new(std::io::Cursor::new(bytes)).expect("decode mp3");
        let _ = len;
        use rodio::Source;
        println!("preview duration: {:?}", dec.total_duration());
    }
}
