//! Configuration persistante (app id / secret / token OAuth / volume).
//! Stockée dans ~/Library/Application Support/deezer-rs/config.json (droits 0600).

use serde_json::{json, Value};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub app_id: String,
    pub secret: String,
    pub token: String,
    pub port: u16,
    pub volume: f32,
    pub user_id: u64,
    /// session Deezer officielle (WebView) active
    pub web: bool,
    /// aurora ASCII animée (teintée par la pochette)
    pub aurora: bool,
    /// vérifier les mises à jour au lancement
    pub auto_update: bool,
    /// copie du cookie de session Deezer (« arl ») pour rester connecté (fichier en 0600)
    pub arl: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { app_id: String::new(), secret: String::new(), token: String::new(), port: 8765, volume: 0.8, user_id: 0, web: false, aurora: true, auto_update: true, arl: String::new() }
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/deezer-rs")
    } else {
        home().join(".config/deezer-rs")
    }
}

pub fn cache_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Caches/deezer-rs")
    } else {
        home().join(".cache/deezer-rs")
    }
}

impl Config {
    fn path() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Self {
        let mut c = Self::default();
        if let Ok(txt) = fs::read_to_string(Self::path()) {
            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                c.app_id = v["app_id"].as_str().unwrap_or("").into();
                c.secret = v["secret"].as_str().unwrap_or("").into();
                c.token = v["token"].as_str().unwrap_or("").into();
                c.user_id = v["user_id"].as_u64().unwrap_or(0);
                c.web = v["web"].as_bool().unwrap_or(false);
                c.aurora = v["aurora"].as_bool().unwrap_or(true);
                c.auto_update = v["auto_update"].as_bool().unwrap_or(true);
                c.arl = v["arl"].as_str().unwrap_or("").into();
                if let Some(p) = v["port"].as_u64() {
                    c.port = p as u16;
                }
                if let Some(x) = v["volume"].as_f64() {
                    c.volume = (x as f32).clamp(0.0, 1.0);
                }
            }
        }
        c
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = fs::create_dir_all(&dir);
        let v = json!({
            "app_id": self.app_id, "secret": self.secret, "token": self.token,
            "port": self.port, "volume": self.volume, "user_id": self.user_id, "web": self.web, "aurora": self.aurora, "auto_update": self.auto_update, "arl": self.arl,
        });
        let tmp = dir.join("config.json.tmp");
        if fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap()).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
            }
            let _ = fs::rename(&tmp, Self::path());
        }
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}/callback", self.port)
    }
}
