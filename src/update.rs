//! Mises à jour automatiques depuis les releases GitHub publiques du projet.
//!
//! 1. `check()` interroge l'API GitHub (`releases/latest`), compare la version au binaire.
//! 2. `download()` récupère `Deezer-macos-universal.zip`, vérifie son SHA-256 (empreinte publiée
//!    par GitHub), le décompresse avec `ditto`.
//! 3. `install_and_restart()` remplace le bundle `Deezer.app` en place puis relance l'app.
//!
//! Aucune donnée personnelle n'est envoyée : une simple requête anonyme à api.github.com.

use eframe::egui;
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

/// Dépôt public des releases (propriétaire/nom).
pub const REPO: &str = "0xZKnw/deezer-rs";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSET: &str = "Deezer-macos-universal.zip";
const UA: &str = concat!("deezer-rs-updater/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available { ver: String, url: String, size: u64, digest: String, notes: String },
    Downloading { ver: String, done: u64, size: u64 },
    Ready { ver: String, app: PathBuf },
    Error(String),
}

#[derive(Clone)]
pub struct Updater {
    pub state: Arc<Mutex<State>>,
    ctx: egui::Context,
}

fn parse_ver(s: &str) -> Vec<u64> {
    s.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn newer(remote: &str, local: &str) -> bool {
    parse_ver(remote) > parse_ver(local)
}

/// Bundle `.app` qui contient l'exécutable courant (None si lancé via `cargo run`).
pub fn current_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(600))).build())
}

impl Updater {
    pub fn new(ctx: egui::Context) -> Self {
        Self { state: Arc::new(Mutex::new(State::Idle)), ctx }
    }

    pub fn get(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Idle)
    }

    fn set(&self, s: State) {
        if let Ok(mut g) = self.state.lock() {
            *g = s;
        }
        self.ctx.request_repaint();
    }

    pub fn busy(&self) -> bool {
        matches!(self.get(), State::Checking | State::Downloading { .. })
    }

    pub fn check(&self) {
        if self.busy() {
            return;
        }
        self.set(State::Checking);
        let me = self.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<State, String> {
                let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
                let mut resp = agent()
                    .get(&url)
                    .header("User-Agent", UA)
                    .header("Accept", "application/vnd.github+json")
                    .call()
                    .map_err(|e| match e {
                        ureq::Error::StatusCode(404) => "No release published yet".to_string(),
                        e => format!("GitHub unreachable ({e})"),
                    })?;
                let txt = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
                let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
                let tag = v["tag_name"].as_str().unwrap_or("").to_owned();
                if tag.is_empty() || !newer(&tag, VERSION) {
                    return Ok(State::UpToDate);
                }
                let asset = v["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(ASSET))).ok_or("Release has no macOS build")?;
                Ok(State::Available {
                    ver: tag.trim_start_matches('v').to_owned(),
                    url: asset["browser_download_url"].as_str().unwrap_or("").to_owned(),
                    size: asset["size"].as_u64().unwrap_or(0),
                    digest: asset["digest"].as_str().unwrap_or("").trim_start_matches("sha256:").to_owned(),
                    notes: v["body"].as_str().unwrap_or("").chars().take(600).collect(),
                })
            })();
            me.set(r.unwrap_or_else(State::Error));
        });
    }

    pub fn download(&self) {
        let State::Available { ver, url, size, digest, .. } = self.get() else { return };
        if !url.starts_with("https://github.com/") && !url.starts_with("https://objects.githubusercontent.com/") {
            self.set(State::Error("Unexpected download URL".into()));
            return;
        }
        self.set(State::Downloading { ver: ver.clone(), done: 0, size });
        let me = self.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<PathBuf, String> {
                let dir = crate::config::cache_dir().join("update");
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let zip = dir.join(ASSET);
                let mut resp = agent().get(&url).header("User-Agent", UA).call().map_err(|e| format!("Download failed ({e})"))?;
                let mut rd = resp.body_mut().with_config().limit(400 << 20).reader();
                let mut out = Vec::with_capacity(size as usize);
                let mut buf = vec![0u8; 256 << 10];
                let mut last = 0u64;
                loop {
                    let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    out.extend_from_slice(&buf[..n]);
                    let done = out.len() as u64;
                    if done - last > 512 << 10 {
                        last = done;
                        me.set(State::Downloading { ver: ver.clone(), done, size });
                    }
                }
                std::fs::write(&zip, &out).map_err(|e| e.to_string())?;
                drop(out);
                if !digest.is_empty() {
                    let o = Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(&zip).output().map_err(|e| e.to_string())?;
                    let got = String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or("").to_lowercase();
                    if got != digest.to_lowercase() {
                        return Err("Checksum mismatch, update discarded".into());
                    }
                }
                let st = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&dir).status().map_err(|e| e.to_string())?;
                if !st.success() {
                    return Err("Could not unpack the update".into());
                }
                let _ = std::fs::remove_file(&zip);
                let app = dir.join("Deezer.app");
                if !app.join("Contents/MacOS/Deezer").exists() {
                    return Err("Update archive is malformed".into());
                }
                Ok(app)
            })();
            me.set(match r {
                Ok(app) => State::Ready { ver, app },
                Err(e) => State::Error(e),
            });
        });
    }

    /// Remplace le bundle courant par la nouvelle version et relance. Ne revient pas en cas de succès.
    pub fn install_and_restart(&self) -> Result<(), String> {
        let State::Ready { app: new, .. } = self.get() else { return Err("Nothing to install".into()) };
        let cur = current_bundle().ok_or("Run the app from Deezer.app to install updates")?;
        swap(&new, &cur)?;
        // relance après la sortie de ce processus
        Command::new("/bin/sh").arg("-c").arg("sleep 1; /usr/bin/open \"$0\"").arg(&cur).spawn().map_err(|e| e.to_string())?;
        std::process::exit(0);
    }
}

fn swap(new: &Path, cur: &Path) -> Result<(), String> {
    let old = cur.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(cur, &old).map_err(|e| format!("Cannot replace the app ({e})"))?;
    let moved = std::fs::rename(new, cur).is_ok()
        || Command::new("/usr/bin/ditto").arg(new).arg(cur).status().map(|s| s.success()).unwrap_or(false);
    if !moved {
        let _ = std::fs::rename(&old, cur);
        return Err("Cannot install the update".into());
    }
    let _ = std::fs::remove_dir_all(&old);
    // téléchargé hors navigateur : pas de quarantaine, mais on nettoie par sécurité
    let _ = Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(cur).status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::newer;
    #[test]
    fn versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }
}
