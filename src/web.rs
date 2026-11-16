//! Session Deezer officielle dans une WebView (WKWebView) intégrée à la fenêtre.
//!
//! - Connexion : la vraie page account.deezer.com s'affiche dans l'app (email + mot de passe,
//!   saisis chez Deezer : l'app ne voit jamais le mot de passe). Cookies gardés par WebKit.
//! - Lecture : le lecteur officiel du site (`dzPlayer`) joue les morceaux complets selon ton
//!   abonnement ; la WebView est cachée et pilotée en JS depuis l'UI egui.
//! - Bibliothèque : likes / playlists via la passerelle du site, avec ta propre session.

use eframe::egui;
use serde_json::Value;
use std::sync::mpsc::{channel, Receiver};
use wry::{dpi::{LogicalPosition, LogicalSize}, Rect, WebView, WebViewBuilder};

pub const LOGIN_URL: &str = "https://account.deezer.com/fr/login/";
pub const ENGINE_URL: &str = "https://www.deezer.com/fr/";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";

const INIT_JS: &str = r#"(()=>{if(window.__drs)return;window.__drs=1;
const send=o=>{try{window.ipc.postMessage(JSON.stringify(o))}catch(e){}};
const onDz=location.hostname==='www.deezer.com';
let tok='';
const GW='/ajax/gw-light.php?input=3&api_version=1.0&method=';
async function who(){
 if(!onDz){send({t:'page',host:location.hostname});return}
 try{const r=await fetch(GW+'deezer.getUserData&api_token=&cid='+Math.floor(Math.random()*1e9),{credentials:'include'});
  const j=await r.json();const res=j.results||{};const u=res.USER||{};tok=res.checkForm||'';
  const id=Number(u.USER_ID)||0;
  if(!id&&window.__drs_arl&&!sessionStorage.getItem('__drs_arl')){
   sessionStorage.setItem('__drs_arl','1');
   document.cookie='arl='+window.__drs_arl+'; domain=.deezer.com; path=/; max-age=31536000; secure; samesite=lax';
   location.reload();return}
  send({t:'user',id,name:u.BLOG_NAME||'',pic:u.USER_PICTURE||'',offer:res.OFFER_NAME||''});
  if(!id){setTimeout(who,3000);return}
  if(!window.__drs_arl_sent){window.__drs_arl_sent=1;
   fetch(GW+'user.getArl&api_token='+encodeURIComponent(tok),{method:'POST',credentials:'include',headers:{'Content-Type':'application/json'},body:'{}'})
    .then(r=>r.json()).then(j=>{if(typeof j.results==='string'&&j.results)send({t:'arl',v:j.results})}).catch(()=>{})}
 }catch(e){setTimeout(who,3000)}
}
window.__drs_gw=async(id,method,params)=>{
 const call=async()=>{const r=await fetch(GW+method+'&api_token='+encodeURIComponent(tok),{method:'POST',credentials:'include',headers:{'Content-Type':'application/json'},body:JSON.stringify(params)});return r.json()};
 try{if(!tok)await who();let j=await call();
  const bad=e=>e&&(Array.isArray(e)?e.length:Object.keys(e).length);
  if(bad(j.error)){await who();j=await call()}
  send({t:'gw',id,err:bad(j.error)?JSON.stringify(j.error):''});
 }catch(e){send({t:'gw',id,err:String(e)})}};
const setQ=p=>{const q=window.__drs_q;if(q)try{if(p.control.getAudioQuality()!==q)p.control.setAudioQuality(q)}catch(e){}};
window.__drs_setq=q=>{window.__drs_q=q;const p=window.dzPlayer;if(p&&p.control)setQ(p)};
window.__drs_play=(id,n)=>{const p=window.dzPlayer;n=n||0;
 if(!p||!p.play){if(n<60)setTimeout(()=>window.__drs_play(id,n+1),250);else send({t:'noplayer'});return}
 setQ(p);
 const s=String(id);p.play({type:'track',id:[s],index:0,context:{ID:s,TYPE:'track_page',CONTEXT_ID:s}});};
window.__drs_c=(f,a)=>{try{const c=window.dzPlayer.control;c[f].apply(c,a||[])}catch(e){}};
let last='';
setInterval(()=>{const p=window.dzPlayer;if(!p||!p.getSongId)return;let s;
 if(!window.__drs_dbg){window.__drs_dbg=1;try{const f=o=>{const r=[];for(const k in o){try{r.push(k+':'+typeof o[k])}catch(e){}}return r};send({t:'dbg',v:JSON.stringify({p:f(p),c:f(p.control||{}),s:[...document.scripts].map(x=>x.src).filter(Boolean)})})}catch(e){}}

 try{s={t:'st',song:Number(p.getSongId())||0,playing:!!p.isPlaying(),pos:p.getPosition()||0,dur:p.getDuration()||0,load:!!p.isLoading()};setQ(p);s.q=String(p.control.getAudioQuality()||'');s.flac=!!(p.html5&&p.html5.flac)}catch(e){return}
 const k=[s.song,s.playing,s.load,Math.round(s.dur),s.q].join();
 if(k!==last||s.playing){last=k;send(s)}},500);
if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',who);else who();
})();"#;

pub enum WebMsg {
    /// id = 0 : pas connecté
    User { id: u64, name: String, pic: String, offer: String },
    State { song: u64, playing: bool, pos: f32, dur: f32, loading: bool, quality: String, flac: bool },
    Gw { id: u64, err: String },
    /// page hors www.deezer.com (page de connexion)
    Page,
    NoPlayer,
    /// cookie de session Deezer, pour rester connecté entre deux lancements
    Arl(String),
}

pub struct Web {
    view: WebView,
    rx: Receiver<WebMsg>,
    pub visible: bool,
    bounds: egui::Rect,
}

fn parse(s: &str) -> Option<WebMsg> {
    let v: Value = serde_json::from_str(s).ok()?;
    let st = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    Some(match v["t"].as_str()? {
        "user" => WebMsg::User { id: v["id"].as_u64().unwrap_or(0), name: st("name"), pic: st("pic"), offer: st("offer") },
        "st" => WebMsg::State {
            song: v["song"].as_u64().unwrap_or(0),
            playing: v["playing"].as_bool().unwrap_or(false),
            pos: v["pos"].as_f64().unwrap_or(0.0) as f32,
            dur: v["dur"].as_f64().unwrap_or(0.0) as f32,
            loading: v["load"].as_bool().unwrap_or(false),
            quality: st("q"),
            flac: v["flac"].as_bool().unwrap_or(false),
        },
        "gw" => WebMsg::Gw { id: v["id"].as_u64().unwrap_or(0), err: st("err") },
        "page" => WebMsg::Page,
        "noplayer" => WebMsg::NoPlayer,
        "arl" => WebMsg::Arl(st("v")),
        "dbg" => {
            if std::env::var_os("DEEZER_PROF").is_some() {
                eprintln!("web dbg: {}", st("v"));
            }
            return None;
        }
        _ => return None,
    })
}

fn wry_rect(r: egui::Rect) -> Rect {
    Rect {
        position: LogicalPosition::new(r.min.x as f64, r.min.y as f64).into(),
        size: LogicalSize::new(r.width().max(1.0) as f64, r.height().max(1.0) as f64).into(),
    }
}

impl Web {
    /// `arl` : cookie de session sauvegardé, réinjecté avant le premier chargement pour rester connecté.
    pub fn new(frame: &eframe::Frame, ctx: egui::Context, url: &str, visible: bool, bounds: egui::Rect, arl: &str, quality: &str) -> Result<Self, String> {
        let (tx, rx) = channel();
        // session sauvegardée : le script de page la remet en cookie si WebKit l'a perdue
        let arl: String = arl.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let q = if crate::config::QUALITIES.contains(&quality) { quality } else { "" };
        let init = format!("window.__drs_arl={};window.__drs_q={:?};{INIT_JS}", serde_json::to_string(&arl).unwrap_or_default(), q);
        let view = WebViewBuilder::new()
            .with_url(url)
            .with_user_agent(UA)
            .with_initialization_script(&init)
            .with_autoplay(true)
            .with_visible(visible)
            .with_bounds(wry_rect(bounds))
            .with_accept_first_mouse(true)
            .with_back_forward_navigation_gestures(true)
            .with_ipc_handler(move |req| {
                if let Some(m) = parse(req.body()) {
                    let _ = tx.send(m);
                    ctx.request_repaint();
                }
            })
            .build_as_child(frame)
            .map_err(|e| e.to_string())?;
        Ok(Self { view, rx, visible, bounds })
    }

    pub fn try_recv(&self) -> Option<WebMsg> {
        self.rx.try_recv().ok()
    }

    pub fn show(&mut self, on: bool) {
        if self.visible != on {
            self.visible = on;
            let _ = self.view.set_visible(on);
            if on {
                let _ = self.view.focus();
            } else {
                let _ = self.view.focus_parent();
            }
        }
    }

    pub fn set_bounds(&mut self, r: egui::Rect) {
        if r != self.bounds {
            self.bounds = r;
            let _ = self.view.set_bounds(wry_rect(r));
        }
    }

    pub fn load(&self, url: &str) {
        let _ = self.view.load_url(url);
    }

    fn js(&self, s: &str) {
        let _ = self.view.evaluate_script(s);
    }

    pub fn play(&self, track: u64) {
        self.js(&format!("window.__drs_play&&window.__drs_play({track})"));
    }

    /// Qualité audio du lecteur officiel (s'applique au prochain chargement de morceau).
    pub fn set_quality(&self, q: &str) {
        // "" = réglage du compte : on cesse simplement d'imposer une qualité
        if q.is_empty() || crate::config::QUALITIES.contains(&q) {
            self.js(&format!("window.__drs_setq&&window.__drs_setq('{q}')"));
        }
    }

    pub fn control(&self, f: &str, args: &str) {
        self.js(&format!("window.__drs_c&&window.__drs_c('{f}',[{args}])"));
    }

    pub fn gw(&self, id: u64, method: &str, params: &Value) {
        self.js(&format!("window.__drs_gw&&window.__drs_gw({id},'{method}',{params})"));
    }

    /// Déconnexion : efface cookies / stockage WebKit de la session Deezer.
    pub fn clear(&self) {
        let _ = self.view.clear_all_browsing_data();
    }
}
