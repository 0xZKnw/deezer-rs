//! OAuth Deezer (flux "authorization code") avec petit serveur HTTP local pour le callback.
//!
//! 1. ouvre https://connect.deezer.com/oauth/auth.php dans ton navigateur
//! 2. Deezer redirige vers http://localhost:<port>/callback?code=...&state=...
//! 3. on échange le code contre un access_token (perms offline_access => n'expire pas)

use crate::config::Config;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

pub const PERMS: &str =
    "basic_access,email,offline_access,manage_library,manage_community,delete_library,listening_history";

fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                o.push(v);
                i += 3;
                continue;
            }
        }
        o.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

fn rand_state() -> String {
    // pas besoin de crypto forte : valeur imprévisible suffisante pour un callback local
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let pid = std::process::id() as u128;
    format!("{:x}", t ^ (pid << 64) ^ 0x9e37_79b9_7f4a_7c15)
}

fn reply(mut s: TcpStream, body: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = s.write_all(resp.as_bytes());
}

fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| pct_decode(v))
    })
}

/// Bloquant (à lancer dans un thread). Retourne le token d'accès.
pub fn login(cfg: &Config) -> Result<String, String> {
    if cfg.app_id.trim().is_empty() {
        return Err("Application ID required (developers.deezer.com/myapps)".into());
    }
    // Sans secret : flux implicite (token dans le fragment d'URL, relayé par une page JS locale).
    let implicit = cfg.secret.trim().is_empty();
    let listener =
        TcpListener::bind(("127.0.0.1", cfg.port)).map_err(|e| format!("Port {} indisponible: {e}", cfg.port))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let state = rand_state();
    let url = format!(
        "https://connect.deezer.com/oauth/auth.php?app_id={}&redirect_uri={}&perms={}&state={}&response_type={}",
        pct_encode(cfg.app_id.trim()),
        pct_encode(&cfg.redirect_uri()),
        PERMS,
        state,
        if implicit { "token" } else { "code" }
    );
    open::that(&url).map_err(|e| format!("Impossible d'ouvrir le navigateur: {e}"))?;

    let start = Instant::now();
    let code = loop {
        if start.elapsed() > Duration::from_secs(300) {
            return Err("Délai dépassé (5 min)".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = req.lines().next().unwrap_or("");
                let path = line.split_whitespace().nth(1).unwrap_or("");
                if implicit {
                    if let Some(q) = path.strip_prefix("/token?") {
                        if let Some(t) = param(q, "access_token") {
                            reply(stream, "ok");
                            return Ok(t);
                        }
                    }
                    if path.contains("error_reason") {
                        reply(stream, "<h3>Login refused. You can close this tab.</h3>");
                        return Err("Login refused".into());
                    }
                    if path.starts_with("/callback") {
                        reply(stream, "<body style='background:#121216;color:#fff;font-family:sans-serif;text-align:center;padding-top:20vh'><h2>Finishing…</h2><script>const h=location.hash.slice(1);if(h.includes('access_token')){fetch('/token?'+h).then(()=>{document.body.innerHTML='<h2>Connected to Deezer ✔</h2><p>You can close this tab and go back to the app.</p>'})}else{document.body.innerHTML='<h2>No token received</h2>'}</script></body>");
                    } else {
                        reply(stream, "…");
                    }
                    continue;
                }
                let Some(query) = path.strip_prefix("/callback?") else {
                    reply(stream, "<h3>…</h3>");
                    continue;
                };
                if let Some(err) = param(query, "error_reason") {
                    reply(stream, "<h3>Connexion refusée. Tu peux fermer cet onglet.</h3>");
                    return Err(format!("Refusé: {err}"));
                }
                if param(query, "state").as_deref() != Some(state.as_str()) {
                    reply(stream, "<h3>État invalide.</h3>");
                    continue;
                }
                if let Some(c) = param(query, "code") {
                    reply(stream, "<body style='background:#121216;color:#fff;font-family:sans-serif;text-align:center;padding-top:20vh'><h2>Connecté à Deezer ✔</h2><p>Tu peux fermer cet onglet et retourner dans l'app.</p></body>");
                    break c;
                }
                reply(stream, "<h3>Code manquant.</h3>");
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };

    let url = format!(
        "https://connect.deezer.com/oauth/access_token.php?app_id={}&secret={}&code={}&output=json",
        pct_encode(&cfg.app_id),
        pct_encode(&cfg.secret),
        pct_encode(&code)
    );
    let body = ureq::get(&url)
        .call()
        .map_err(|e| e.to_string())?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(t) = v["access_token"].as_str() {
            return Ok(t.to_owned());
        }
    }
    // ancien format: access_token=xxx&expires=0
    if let Some(t) = param(&body, "access_token") {
        return Ok(t);
    }
    Err(format!("Échange du code refusé: {}", body.chars().take(200).collect::<String>()))
}
