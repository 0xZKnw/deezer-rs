//! Modèles légers extraits des JSON Deezer (pas de serde derive : moins de compile, moins de RAM).

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
    Podcast,
    Radio,
    Genre,
}

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub artist_id: u64,
    pub album: String,
    pub album_id: u64,
    pub img: String,
    pub dur: u32,
    pub preview: String,
    pub explicit: bool,
    pub rank: u32,
    pub added: u64,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub kind: Kind,
    pub id: u64,
    pub title: String,
    pub sub: String,
    pub extra: String,
    pub img: String,
    pub explicit: bool,
    pub owner: u64,
    pub track: Option<Box<Track>>,
}

pub fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_owned()
}

pub fn n(v: &Value, k: &str) -> u64 {
    v[k].as_u64().or_else(|| v[k].as_str().and_then(|x| x.parse().ok())).unwrap_or(0)
}

pub fn data(v: &Value) -> &[Value] {
    v["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

pub fn fmt_num(x: u64) -> String {
    let s = x.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn fmt_dur(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

pub fn fmt_total(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

/// Date unix -> "13/11/2026"
pub fn fmt_date_ts(ts: u64) -> String {
    if ts == 0 {
        return String::new();
    }
    let days = (ts / 86400) as i64;
    // algorithme civil (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:02}/{:02}/{}", d, m, y)
}

/// "2026-11-13" -> "13/11/2026"
pub fn fmt_date_iso(d: &str) -> String {
    let p: Vec<&str> = d.split('-').collect();
    if p.len() == 3 {
        format!("{}/{}/{}", p[2], p[1], p[0])
    } else {
        d.to_owned()
    }
}

/// Redimensionne l'URL d'une image du CDN Deezer ("…/250x250-000000-80-0-0.jpg" -> "…/120x120-…").
pub fn img_size(url: &str, px: u32) -> String {
    if let Some((base, file)) = url.rsplit_once('/') {
        if let Some((dim, rest)) = file.split_once('-') {
            if dim.contains('x') && dim.chars().all(|c| c.is_ascii_digit() || c == 'x') {
                return format!("{base}/{px}x{px}-{rest}");
            }
        }
    }
    url.to_owned()
}

#[derive(Clone, Default)]
pub struct AlbumFb {
    pub album: String,
    pub album_id: u64,
    pub img: String,
}

impl Track {
    pub fn from_json(v: &Value, fb: Option<&AlbumFb>) -> Option<Track> {
        let id = n(v, "id");
        if id == 0 {
            return None;
        }
        let alb = &v["album"];
        let mut img = s(alb, "cover_medium");
        let mut album = s(alb, "title");
        let mut album_id = n(alb, "id");
        if let Some(f) = fb {
            if img.is_empty() {
                img = f.img.clone();
            }
            if album.is_empty() {
                album = f.album.clone();
            }
            if album_id == 0 {
                album_id = f.album_id;
            }
        }
        if img.is_empty() {
            img = s(&v["artist"], "picture_medium");
        }
        Some(Track {
            id,
            title: s(v, "title"),
            artist: s(&v["artist"], "name"),
            artist_id: n(&v["artist"], "id"),
            album,
            album_id,
            img,
            dur: n(v, "duration") as u32,
            preview: s(v, "preview"),
            explicit: v["explicit_lyrics"].as_bool().unwrap_or(false),
            rank: n(v, "rank") as u32,
            added: n(v, "time_add").max(n(v, "timestamp")),
        })
    }
}

impl Card {
    pub fn from_track(t: &Track) -> Card {
        Card {
            kind: Kind::Track,
            id: t.id,
            title: t.title.clone(),
            sub: t.artist.clone(),
            extra: String::new(),
            img: t.img.clone(),
            explicit: t.explicit,
            owner: 0,
            track: Some(Box::new(t.clone())),
        }
    }

    pub fn from_json(kind: Kind, v: &Value) -> Option<Card> {
        let id = n(v, "id");
        if id == 0 {
            return None;
        }
        let (title, sub, extra, img) = match kind {
            Kind::Album => (
                s(v, "title"),
                s(&v["artist"], "name"),
                fmt_date_iso(&s(v, "release_date")),
                s(v, "cover_medium"),
            ),
            Kind::Artist => {
                let fans = n(v, "nb_fan");
                (
                    s(v, "name"),
                    if fans > 0 { format!("{} fans", fmt_num(fans)) } else { String::new() },
                    String::new(),
                    s(v, "picture_medium"),
                )
            }
            Kind::Playlist => {
                let nb = n(v, "nb_tracks");
                let by = {
                    let c = s(&v["creator"], "name");
                    if c.is_empty() { s(&v["user"], "name") } else { c }
                };
                (
                    s(v, "title"),
                    if !by.is_empty() { format!("By {by}") } else { String::new() },
                    if nb > 0 { format!("{nb} tracks") } else { String::new() },
                    s(v, "picture_medium"),
                )
            }
            Kind::Podcast => (s(v, "title"), String::new(), String::new(), s(v, "picture_medium")),
            Kind::Radio => (s(v, "title"), String::new(), String::new(), s(v, "picture_medium")),
            Kind::Genre => (s(v, "name"), String::new(), String::new(), s(v, "picture_medium")),
            Kind::Track => return Track::from_json(v, None).map(|t| Card::from_track(&t)),
        };
        Some(Card {
            kind,
            id,
            title,
            sub,
            extra,
            img,
            explicit: v["explicit_lyrics"].as_bool().unwrap_or(false),
            owner: {
                let o = n(&v["creator"], "id");
                if o == 0 { n(&v["user"], "id") } else { o }
            },
            track: None,
        })
    }

    /// Détecte le type via le champ "type" (listes hétérogènes : recommandations, flow…).
    pub fn from_any(v: &Value) -> Option<Card> {
        let k = match v["type"].as_str()? {
            "track" => Kind::Track,
            "album" => Kind::Album,
            "artist" => Kind::Artist,
            "playlist" => Kind::Playlist,
            "podcast" => Kind::Podcast,
            "radio" => Kind::Radio,
            _ => return None,
        };
        Card::from_json(k, v)
    }
}

/// Mise en page d'une section d'accueil.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Carousel,
    Tiles,
    Grid,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub title: String,
    pub cards: Vec<Card>,
    pub style: Style,
}

#[derive(Clone, Debug, Default)]
pub struct Header {
    pub kind: Option<Kind>,
    pub id: u64,
    pub title: String,
    pub line: String,
    pub line_id: u64,
    pub meta: String,
    pub desc: String,
    pub img: String,
    pub round: bool,
    pub owner_id: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PageData {
    pub header: Option<Header>,
    pub tracks_title: String,
    pub tracks: Vec<Track>,
    pub total: u32,
    pub sections: Vec<Section>,
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Route {
    Home,
    Explore,
    FavTracks,
    Playlists,
    Albums,
    Artists,
    Podcasts,
    Search(String),
    Artist(u64),
    Album(u64),
    Playlist(u64),
    Genre(u64),
    Radio(u64),
    Podcast(u64),
}
