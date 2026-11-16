//! Client HTTP pour l'API publique Deezer (https://api.deezer.com), avec token OAuth.
//! Bloquant (ureq) : appelé uniquement depuis des threads workers, jamais depuis l'UI.

use serde_json::Value;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

pub type Res<T> = Result<T, String>;

const BASE: &str = "https://api.deezer.com";

#[derive(Clone)]
pub struct Api {
    agent: ureq::Agent,
    token: Arc<RwLock<String>>,
    user: Arc<RwLock<u64>>,
}

#[derive(Clone, Copy)]
pub enum Method {
    Get,
    Post,
    Delete,
}

pub type Q<'a> = &'a [(&'a str, String)];

impl Api {
    pub fn new(token: &str) -> Self {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .build();
        Self { agent: ureq::Agent::new_with_config(cfg), token: Arc::new(RwLock::new(token.to_owned())), user: Arc::new(RwLock::new(0)) }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    pub fn set_token(&self, t: &str) {
        *self.token.write().unwrap() = t.to_owned();
    }

    pub fn set_user(&self, id: u64) {
        *self.user.write().unwrap() = id;
    }

    /// "/user/me" avec token OAuth, sinon "/user/<id>" (profil public, lecture seule).
    pub fn me_path(&self) -> String {
        if self.has_token() {
            "/user/me".into()
        } else {
            format!("/user/{}", self.user.read().unwrap())
        }
    }

    pub fn has_token(&self) -> bool {
        !self.token.read().unwrap().is_empty()
    }

    pub fn request(&self, method: Method, path: &str, q: Q) -> Res<Value> {
        let url = format!("{BASE}{path}");
        let tok = self.token.read().unwrap().clone();
        let mut last = String::new();
        for attempt in 0..3 {
            let r = match method {
                Method::Get => {
                    let mut b = self.agent.get(&url);
                    for (k, v) in q {
                        b = b.query(*k, v.as_str());
                    }
                    if !tok.is_empty() {
                        b = b.query("access_token", tok.as_str());
                    }
                    b.call()
                }
                Method::Delete => {
                    let mut b = self.agent.delete(&url);
                    for (k, v) in q {
                        b = b.query(*k, v.as_str());
                    }
                    if !tok.is_empty() {
                        b = b.query("access_token", tok.as_str());
                    }
                    b.call()
                }
                Method::Post => {
                    let mut b = self.agent.post(&url);
                    for (k, v) in q {
                        b = b.query(*k, v.as_str());
                    }
                    if !tok.is_empty() {
                        b = b.query("access_token", tok.as_str());
                    }
                    b.send_empty()
                }
            };
            match r {
                Ok(resp) => {
                    let txt = resp.into_body().read_to_string().map_err(|e| e.to_string())?;
                    let v: Value = serde_json::from_str(&txt).unwrap_or(Value::Bool(txt.trim() == "true"));
                    if let Some(err) = v.get("error") {
                        let code = err["code"].as_u64().unwrap_or(0);
                        // 4 = quota dépassé : on attend un peu puis on réessaie
                        if code == 4 && attempt < 2 {
                            std::thread::sleep(Duration::from_millis(900));
                            continue;
                        }
                        return Err(format!(
                            "{} ({})",
                            err["message"].as_str().unwrap_or("Deezer API error"),
                            err["type"].as_str().unwrap_or("?")
                        ));
                    }
                    return Ok(v);
                }
                Err(e) => {
                    last = e.to_string();
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        }
        Err(last)
    }

    pub fn get(&self, path: &str, q: Q) -> Res<Value> {
        self.request(Method::Get, path, q)
    }

    pub fn post(&self, path: &str, q: Q) -> Res<Value> {
        self.request(Method::Post, path, q)
    }

    pub fn delete(&self, path: &str, q: Q) -> Res<Value> {
        self.request(Method::Delete, path, q)
    }

    /// Télécharge des octets bruts (pochettes, previews MP3).
    pub fn download(&self, url: &str, max: u64) -> Res<Vec<u8>> {
        let resp = self.agent.get(url).call().map_err(|e| e.to_string())?;
        resp.into_body().with_config().limit(max).read_to_vec().map_err(|e| e.to_string())
    }
}
