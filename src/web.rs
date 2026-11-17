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
// la page Deezer n'est jamais affichée (seule la page de connexion l'est) : pas de mise en page ni de rendu
if(onDz){const st=document.createElement('style');st.textContent='html{display:none!important}';(document.head||document.documentElement).appendChild(st)}
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
const noRep=p=>{try{const r=p.getRepeat?p.getRepeat():0;if(r)p.control.setRepeat(0)}catch(e){}};
const setQ=p=>{noRep(p);const q=window.__drs_q;if(q)try{if(p.control.getAudioQuality()!==q)p.control.setAudioQuality(q)}catch(e){}};
window.__drs_setq=q=>{window.__drs_q=q;const p=window.dzPlayer;if(p&&p.control)setQ(p)};
window.__drs_play=(id,n)=>{const p=window.dzPlayer;n=n||0;
 if(!p||!p.play){if(n<60)setTimeout(()=>window.__drs_play(id,n+1),250);else send({t:'noplayer'});return}
 setQ(p);
 const a=(Array.isArray(id)?id:[id]).map(String);p.play({type:'track',id:a,index:0,context:{ID:a[0],TYPE:'track_page',CONTEXT_ID:a[0]}});};
window.__drs_c=(f,a)=>{try{const c=window.dzPlayer.control;c[f].apply(c,a||[])}catch(e){}};
let last='';
setInterval(()=>{const p=window.dzPlayer;if(!p||!p.getSongId)return;let s;
 if(window.__drs_auto&&p.play){const a=window.__drs_auto;window.__drs_auto=0;window.__drs_autot=a.length>1?2:1;window.__drs_play(a)}
 if(window.__drs_autot===1&&p.getPosition()>3){window.__drs_autot=0;try{p.control.pause()}catch(e){}}
 if(window.__drs_autot===2&&p.getPosition()>3&&p.getDuration()>10){window.__drs_autot=3;window.__drs_s0=p.getSongId();try{p.control.seek((p.getDuration()-4)/p.getDuration())}catch(e){}}
 if(window.__drs_autot===3&&p.getSongId()!=window.__drs_s0&&p.getPosition()>3){window.__drs_autot=0;try{p.control.pause()}catch(e){}}
 if(!window.__drs_dbg){window.__drs_dbg=1;try{const f=o=>{const r=[];for(const k in o){try{r.push(k+':'+typeof o[k])}catch(e){}}return r};send({t:'dbg',v:JSON.stringify({p:f(p),c:f(p.control||{}),s:[...document.scripts].map(x=>x.src).filter(Boolean)})})}catch(e){}}

 try{s={t:'st',song:Number(p.getSongId())||0,playing:!!p.isPlaying(),pos:Number(p.getPosition())||0,dur:Number(p.getDuration())||0,load:!!p.isLoading()};setQ(p);s.q=String(p.control.getAudioQuality()||'');s.flac=!!(p.html5&&p.html5.flac)}catch(e){return}
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
    /// page à charger dès que la liste de blocage est prête (lecteur caché)
    pending: Option<(String, std::time::Instant)>,
    rules_on: bool,
}

/// Lecteur caché allégé : WebKit ne télécharge ni images, ni polices, ni traceurs tiers
/// (rien n'est affiché, seul le moteur audio compte). Désactivé quand la WebView est visible
/// (page de connexion), pour qu'elle reste normale.
mod light {
    use objc2::{rc::Retained, MainThreadMarker};
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// traceurs / mesure d'audience tiers (aucun domaine Deezer)
    const TRACKERS: [&str; 26] = [
        "google-analytics", "googletagmanager", "googleadservices", "doubleclick", "facebook", "fbcdn", "braze", "appboy", "sentry",
        "hotjar", "scorecardresearch", "criteo", "adsrvr", "amazon-adsystem", "tiktok", "snapchat", "pinterest", "clarity",
        "onetrust", "cookielaw", "appsflyer", "branch", "segment", "mixpanel", "amplitude", "nr-data",
    ];

    /// règles WebKit (pas d'alternative « | » dans les filtres : une règle par domaine)
    fn rules() -> String {
        let mut r = vec![r#"{"trigger":{"url-filter":".*","resource-type":["image","font","svg-document"]},"action":{"type":"block"}}"#.to_owned()];
        for d in TRACKERS {
            r.push(format!(r#"{{"trigger":{{"url-filter":"^https?://([^/]*\\.)?{d}\\.","load-type":["third-party"]}},"action":{{"type":"block"}}}}"#));
        }
        format!("[{}]", r.join(","))
    }

    thread_local!(static LIST: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) });
    pub static READY: AtomicBool = AtomicBool::new(false);

    pub fn compile(ctx: eframe::egui::Context) {
        let Some(mtm) = MainThreadMarker::new() else {
            READY.store(true, Ordering::Relaxed);
            return;
        };
        unsafe {
            let Some(store) = WKContentRuleListStore::defaultStore(mtm) else {
                READY.store(true, Ordering::Relaxed);
                return;
            };
            let blk = block2::RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
                if let (Some(e), true) = (Retained::retain(err), std::env::var_os("DEEZER_PROF").is_some()) {
                    eprintln!("content rules: {}", e.localizedDescription());
                }
                if let Some(l) = Retained::retain(list) {
                    LIST.with(|c| *c.borrow_mut() = Some(l));
                }
                READY.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            });
            store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                Some(&NSString::from_str("drs-light-2")),
                Some(&NSString::from_str(&rules())),
                Some(&blk),
            );
        }
    }

    /// true si la liste a bien été appliquée
    pub fn apply(ctrl: &WKUserContentController, on: bool) -> bool {
        unsafe {
            ctrl.removeAllContentRuleLists();
            if on {
                return LIST.with(|c| c.borrow().as_ref().map(|l| ctrl.addContentRuleList(l)).is_some());
            }
        }
        true
    }
}

fn parse(s: &str) -> Option<WebMsg> {
    let v: Value = serde_json::from_str(s).ok()?;
    let st = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    Some(match v["t"].as_str()? {
        "user" => WebMsg::User { id: v["id"].as_u64().unwrap_or(0), name: st("name"), pic: st("pic"), offer: st("offer") },
        "st" if std::env::var_os("DEEZER_PROF").is_some() && { eprintln!("web st: {s}"); false } => return None,
        "st" => WebMsg::State {
            song: v["song"].as_u64().unwrap_or(0),
            playing: v["playing"].as_bool().unwrap_or(false),
            // Deezer donne parfois la durée en texte ("137") après un enchaînement automatique
            pos: num(&v["pos"]),
            dur: num(&v["dur"]),
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
        // test automatique (profilage seulement) : DEEZER_AUTOPLAY=<id> joue 3 s puis met en pause
        // DEEZER_AUTOPLAY=<id>,<id> : enchaîne vers le 2e titre (test de la suite de file)
        let auto: Vec<u64> = std::env::var("DEEZER_AUTOPLAY").ok().filter(|_| std::env::var_os("DEEZER_PROF").is_some()).map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
        let auto = if auto.is_empty() { "0".to_owned() } else { format!("{auto:?}") };
        let init = format!("window.__drs_arl={};window.__drs_q={:?};window.__drs_auto={auto};{INIT_JS}", serde_json::to_string(&arl).unwrap_or_default(), q);
        light::compile(ctx.clone());
        // lecteur caché : on attend la liste de blocage (quelques ms) avant de charger le site
        let pending = (!visible).then(|| (url.to_owned(), std::time::Instant::now()));
        let view = WebViewBuilder::new()
            .with_url(if visible { url } else { "about:blank" })
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
        Ok(Self { view, rx, visible, bounds, pending, rules_on: false })
    }

    pub fn try_recv(&mut self) -> Option<WebMsg> {
        self.sync_rules();
        self.rx.try_recv().ok()
    }

    fn sync_rules(&mut self) {
        use std::sync::atomic::Ordering;
        let ready = light::READY.load(Ordering::Relaxed);
        let want = !self.visible && ready;
        if want != self.rules_on {
            use wry::WebViewExtMacOS;
            self.rules_on = light::apply(&self.view.manager(), want) && want;
        }
        if let Some((url, t0)) = &self.pending {
            if ready || t0.elapsed() > std::time::Duration::from_secs(2) {
                let url = url.clone();
                self.pending = None;
                self.load(&url);
            }
        }
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

    /// Lance `ids[0]` ; les suivants forment la suite du lecteur Deezer (sinon il enchaîne
    /// sur ses propres recommandations à la fin du morceau).
    pub fn play(&self, ids: &[u64]) {
        self.js(&format!("window.__drs_play&&window.__drs_play({ids:?})"));
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

fn num(v: &Value) -> f32 {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|x: &f64| x.is_finite()).unwrap_or(0.0) as f32
}
