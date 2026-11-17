//! Historique local « Recently played » (albums / playlists / artistes lancés).
//! Petit fichier JSON dans le dossier de config : sert quand il n'y a pas de token OAuth
//! (connexion arl), car `/user/me/history` exige ce token.

use crate::model::{Card, Kind};
use serde_json::{json, Value};
use std::fs;

const MAX: usize = 20;

fn path() -> std::path::PathBuf {
    crate::config::config_dir().join("recent.json")
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Album => "album",
        Kind::Artist => "artist",
        Kind::Playlist => "playlist",
        Kind::Podcast => "podcast",
        Kind::Radio => "radio",
        Kind::Genre => "genre",
        Kind::Track => "track",
    }
}

fn kind_of(s: &str) -> Option<Kind> {
    Some(match s {
        "album" => Kind::Album,
        "artist" => Kind::Artist,
        "playlist" => Kind::Playlist,
        "podcast" => Kind::Podcast,
        "radio" => Kind::Radio,
        _ => return None,
    })
}

pub fn load() -> Vec<Card> {
    let Ok(txt) = fs::read_to_string(path()) else { return vec![] };
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&txt) else { return vec![] };
    a.iter()
        .filter_map(|v| {
            Some(Card {
                kind: kind_of(v["k"].as_str()?)?,
                id: v["id"].as_u64()?,
                title: v["t"].as_str()?.to_owned(),
                sub: v["s"].as_str().unwrap_or("").to_owned(),
                extra: String::new(),
                img: v["i"].as_str().unwrap_or("").to_owned(),
                explicit: false,
                owner: 0,
                track: None,
            })
        })
        .take(MAX)
        .collect()
}

/// Met `c` en tête (dédoublonné). Renvoie false si rien n'a changé (déjà en tête).
pub fn push(list: &mut Vec<Card>, c: Card) -> bool {
    if c.id == 0 || c.kind == Kind::Track || c.kind == Kind::Genre || c.title.is_empty() {
        return false;
    }
    if list.first().is_some_and(|f| f.kind == c.kind && f.id == c.id) {
        return false;
    }
    list.retain(|x| !(x.kind == c.kind && x.id == c.id));
    list.insert(0, Card { extra: String::new(), track: None, ..c });
    list.truncate(MAX);
    let a: Vec<Value> = list.iter().map(|c| json!({"k": kind_str(c.kind), "id": c.id, "t": c.title, "s": c.sub, "i": c.img})).collect();
    let dir = crate::config::config_dir();
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join("recent.json.tmp");
    if fs::write(&tmp, serde_json::to_vec(&a).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, path());
    }
    true
}
