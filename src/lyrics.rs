//! Paroles Deezer (API GraphQL `pipe.deezer.com`, celle du lecteur web officiel).
//!
//! Nécessite la session Deezer (cookie « arl ») : on l'échange contre un jeton JWT court
//! (`auth.deezer.com`), réutilisé quelques minutes, puis on demande les paroles du morceau.
//! Appels bloquants : toujours depuis un thread à part, jamais depuis l'UI.

use serde_json::{json, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum Lyrics {
    Loading,
    /// lignes synchronisées : (début en ms, texte)
    Synced(Vec<(u32, String)>),
    Plain(Vec<String>),
    None,
    /// pas de session Deezer complète
    NeedLogin,
    Err(String),
}

static JWT: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(12))).http_status_as_error(false).build())
}

fn jwt(a: &ureq::Agent, arl: &str, fresh: bool) -> Result<String, String> {
    if !fresh {
        if let Some((j, t)) = JWT.lock().ok().and_then(|g| g.clone()) {
            if t.elapsed() < Duration::from_secs(240) {
                return Ok(j);
            }
        }
    }
    let mut r = a
        .post("https://auth.deezer.com/login/arl?jo=p&rto=c&i=c")
        .header("Cookie", &format!("arl={arl}"))
        .send_empty()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let j = v["jwt"].as_str().unwrap_or("").to_owned();
    if j.is_empty() {
        return Err("Deezer session expired".into());
    }
    if let Ok(mut g) = JWT.lock() {
        *g = Some((j.clone(), Instant::now()));
    }
    Ok(j)
}

const QUERY: &str = "query GetLyrics($trackId: String!) { track(trackId: $trackId) { id lyrics { id text synchronizedLines { line milliseconds } } } }";

pub fn fetch(arl: &str, track: u64) -> Lyrics {
    if arl.is_empty() {
        return Lyrics::NeedLogin;
    }
    let a = agent();
    let body = json!({ "operationName": "GetLyrics", "variables": { "trackId": track.to_string() }, "query": QUERY }).to_string();
    for fresh in [false, true] {
        let tok = match jwt(&a, arl, fresh) {
            Ok(t) => t,
            Err(e) => return Lyrics::Err(e),
        };
        let res = a
            .post("https://pipe.deezer.com/api")
            .header("Authorization", &format!("Bearer {tok}"))
            .header("Content-Type", "application/json")
            .send(body.as_str());
        let mut r = match res {
            Ok(r) => r,
            Err(e) => return Lyrics::Err(e.to_string()),
        };
        if r.status() == 401 && !fresh {
            continue; // jeton expiré : on en redemande un
        }
        let txt = r.body_mut().with_config().limit(2 << 20).read_to_string().unwrap_or_default();
        let v: Value = match serde_json::from_str(&txt) {
            Ok(v) => v,
            Err(_) => return Lyrics::Err("Unexpected answer from Deezer".into()),
        };
        let l = &v["data"]["track"]["lyrics"];
        if l.is_null() {
            let unauth = v["errors"].as_array().is_some_and(|e| e.iter().any(|x| x["type"].as_str().is_some_and(|t| t.contains("Auth"))));
            if unauth && !fresh {
                continue;
            }
            return Lyrics::None;
        }
        let sync: Vec<(u32, String)> = l["synchronizedLines"]
            .as_array()
            .map(|a| a.iter().map(|x| (x["milliseconds"].as_u64().unwrap_or(0) as u32, x["line"].as_str().unwrap_or("").trim().to_owned())).collect())
            .unwrap_or_default();
        if sync.iter().any(|(_, s)| !s.is_empty()) {
            return Lyrics::Synced(sync);
        }
        let text: Vec<String> = l["text"].as_str().unwrap_or("").lines().map(|s| s.trim().to_owned()).collect();
        return if text.iter().any(|s| !s.is_empty()) { Lyrics::Plain(text) } else { Lyrics::None };
    }
    Lyrics::Err("Deezer session expired".into())
}

/// Index de la ligne en cours (dernière ligne commencée).
pub fn current(lines: &[(u32, String)], pos_ms: u32) -> Option<usize> {
    match lines.partition_point(|(t, _)| *t <= pos_ms) {
        0 => None,
        n => Some(n - 1),
    }
}
