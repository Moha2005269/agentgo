//! Fingerprint protection in the style of Brave's "farbling".
//!
//! Canvas (2D, offscreen and WebGL read-backs) and audio read-backs are
//! nudged by amounts people cannot see, so trackers cannot build a stable
//! identifier from them.
//!
//! * No agent connected (the person browsing): the nudge changes with every
//!   browser session and with every site, so nothing links visits together.
//! * An agent connected: the browser switches to the agent's own fingerprint,
//!   unique to this installation, kept across runs and the same on every
//!   site. It never matches the person's, so an agent's visits cannot be tied
//!   to the person's browsing, and sites see one consistent visitor (no
//!   identity rotation: policy rule 2).
//!
//! The hooks are proxies of the native functions, so `toString`, `name` and
//! `length` read exactly as the browser's own. Same-origin child frames
//! (`about:blank`, `srcdoc`) and `window.open` popups get the same
//! protection as their opener. Hardware numbers are left alone: workers
//! would report the real values and the mismatch itself identifies.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use serde_json::json;

use super::{cdp, sessions, state};

fn random_u64() -> u64 {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 8];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        return now ^ u64::from(std::process::id()).rotate_left(32);
    }
    u64::from_le_bytes(bytes)
}

/// The unique fingerprint agents browse with; made on first use, then kept.
fn persistent_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        let path = state::data_dir().join("fingerprint-seed");
        if let Some(seed) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| u64::from_str_radix(text.trim(), 16).ok())
        {
            return seed;
        }
        let seed = random_u64();
        let _ = std::fs::write(&path, format!("{seed:016x}"));
        seed
    })
}

fn session_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(random_u64)
}

/// The 32-bit base the page script derives its noise from.
fn base_seed(agent: bool) -> u32 {
    let mut hasher = DefaultHasher::new();
    if agent {
        persistent_seed().hash(&mut hasher);
    } else {
        session_seed().hash(&mut hasher);
    }
    hasher.finish() as u32
}

/// Runs before the page's own scripts. `perSite` mixes the top-level site
/// into the seed (browsing); the agent's fingerprint is the same everywhere.
const SCRIPT: &str = r#"(function(base,perSite){try{
const P=location.protocol;
if(P!=='http:'&&P!=='https:'&&P!=='about:'&&P!=='blob:'&&P!=='data:')return;
const MARK=Symbol.for('@MARK@');
const mix=(a,b)=>{let h=Math.imul(a^0x9E3779B9,0x85EBCA6B)^Math.imul(b+0x7F4A7C15,0xC2B2AE35);h^=h>>>13;h=Math.imul(h,0x27D4EB2F);return(h^h>>>16)>>>0};
const SLD=['co','com','net','org','gov','edu','ac','or','ne','go','gob','nic','gv'];
const site=(h)=>{const p=h.split('.'),n=p.length;return(n>2&&p[n-1].length===2&&SLD.includes(p[n-2])?p.slice(-3):p.slice(-2)).join('.')};
let host=location.hostname;
try{const a=location.ancestorOrigins;if(a&&a.length)host=new URL(a[a.length-1]).hostname}catch(e){}
let seed=base>>>0;
if(perSite){let h=7;for(const ch of site(host))h=mix(h,ch.charCodeAt(0));seed=mix(seed,h)}
const noise=(d,x0,y0,w)=>{w=w>0?w:1;for(let i=0;i+3<d.length;i+=4){if(d[i+3]===0)continue;const p=i>>2;const h=mix(seed,Math.imul(x0+p%w,73856093)^Math.imul(y0+(p/w|0),19349663));if((h&63)===0)d[i+(h>>>6)%3]^=1}};
const strings=new WeakMap();
const protect=(w)=>{
try{if(w[MARK])return;void w.document;Object.defineProperty(w,MARK,{value:1})}catch(e){return}
const toStr=w.Function.prototype.toString;
const clean=(e)=>{try{if(e&&typeof e.stack==='string'&&/<anonymous>:\d+:\d+/.test(e.stack)){const st=e.stack.split('\n').filter(l=>!/<anonymous>:\d+:\d+/.test(l)).join('\n');Object.defineProperty(e,'stack',{value:st,writable:true,configurable:true})}}catch(x){}return e};
const wrap=(orig,apply)=>{const p=new Proxy(orig,{apply:(t,s,a)=>{try{return apply(t,s,a)}catch(e){throw clean(e)}}});strings.set(p,toStr.call(orig));return p};
const method=(o,name,apply)=>{try{const d=Object.getOwnPropertyDescriptor(o,name);if(!d||typeof d.value!=='function')return;d.value=wrap(d.value,apply);Object.defineProperty(o,name,d)}catch(e){}};
const accessor=(o,name,kind,after)=>{try{const d=Object.getOwnPropertyDescriptor(o,name);if(!d||!d[kind])return;d[kind]=wrap(d[kind],(t,s,a)=>{const r=Reflect.apply(t,s,a);try{after(r)}catch(e){}return r});Object.defineProperty(o,name,d)}catch(e){}};
method(w.Function.prototype,'toString',(t,s,a)=>{const v=strings.get(s);return v!==undefined?v:Reflect.apply(t,s,a)});
try{if(w.Navigator&&!('globalPrivacyControl' in w.Navigator.prototype)){const g=Object.getOwnPropertyDescriptor({get globalPrivacyControl(){return true}},'globalPrivacyControl').get;strings.set(g,'function get globalPrivacyControl() { [native code] }');Object.defineProperty(w.Navigator.prototype,'globalPrivacyControl',{get:g,configurable:true,enumerable:true})}}catch(e){}
for(const C of [w.CanvasRenderingContext2D,w.OffscreenCanvasRenderingContext2D])if(C)method(C.prototype,'getImageData',(t,s,a)=>{const img=Reflect.apply(t,s,a);try{noise(img.data,a[0]|0,a[1]|0,img.width)}catch(e){}return img});
const copyOf=(c,make)=>{try{const W=c.width,H=c.height;if(!W||!H||W*H>16777216)return null;const k=make(W,H),x=k.getContext('2d');x.drawImage(c,0,0);x.putImageData(x.getImageData(0,0,W,H),0,0);return k}catch(e){return null}};
const element=(W,H)=>{const k=w.document.createElement('canvas');k.width=W;k.height=H;return k};
if(w.HTMLCanvasElement)for(const n of ['toDataURL','toBlob'])method(w.HTMLCanvasElement.prototype,n,(t,s,a)=>Reflect.apply(t,copyOf(s,element)||s,a));
if(w.OffscreenCanvas)method(w.OffscreenCanvas.prototype,'convertToBlob',(t,s,a)=>Reflect.apply(t,copyOf(s,(W,H)=>new w.OffscreenCanvas(W,H))||s,a));
for(const G of [w.WebGLRenderingContext,w.WebGL2RenderingContext])if(G)method(G.prototype,'readPixels',(t,s,a)=>{const r=Reflect.apply(t,s,a);try{const b=a[6];if(ArrayBuffer.isView(b)&&b.BYTES_PER_ELEMENT===1&&b.length)noise(b,a[0]|0,a[1]|0,a[2]|0)}catch(e){}return r});
if(w.AudioBuffer){const seen=new WeakSet();
method(w.AudioBuffer.prototype,'getChannelData',(t,s,a)=>{const d=Reflect.apply(t,s,a);if(!seen.has(d)){seen.add(d);for(let i=0;i<d.length;i+=97)d[i]+=((mix(seed,i)&255)-128)*1e-8}return d});
method(w.AudioBuffer.prototype,'copyFromChannel',(t,s,a)=>{try{s.getChannelData(a[1]|0)}catch(e){}return Reflect.apply(t,s,a)})}
if(w.AnalyserNode)method(w.AnalyserNode.prototype,'getFloatFrequencyData',(t,s,a)=>{const r=Reflect.apply(t,s,a);try{const v=a[0];for(let i=0;i<v.length;i+=7)v[i]+=((mix(seed,i)&255)-128)*1e-6}catch(e){}return r});
let frames=0;
const check=()=>{try{const f=w.frames,n=f.length;if(n===frames)return;frames=n;for(let i=0;i<n;i++){try{protect(f[i])}catch(e){}}}catch(e){}};
const after=(t,s,a)=>{const r=Reflect.apply(t,s,a);check();return r};
if(w.HTMLIFrameElement){const I=w.HTMLIFrameElement.prototype;accessor(I,'contentWindow','get',(r)=>{if(r)protect(r)});accessor(I,'contentDocument','get',(r)=>{if(r)protect(r.defaultView)})}
for(const n of ['appendChild','insertBefore','replaceChild'])method(w.Node.prototype,n,after);
for(const n of ['append','prepend','after','before','replaceWith','replaceChildren','insertAdjacentElement','insertAdjacentHTML'])method(w.Element.prototype,n,after);
for(const n of ['innerHTML','outerHTML'])accessor(w.Element.prototype,n,'set',check);
for(const n of ['write','writeln'])method(w.Document.prototype,n,after);
method(w,'open',(t,s,a)=>{const r=Reflect.apply(t,s,a);try{if(r)protect(r)}catch(e){}return r});
};
protect(window);
}catch(e){}})"#;

/// What one tab has installed.
#[derive(Default)]
struct TabFp {
    /// DevTools identifier of the registered script.
    identifier: Option<String>,
    /// The mode it was built for: Some(true) agent, Some(false) person,
    /// None when protection is off or nothing is installed yet.
    mode: Option<bool>,
    /// An install is running for this tab.
    busy: bool,
    /// The wanted mode changed while it ran: run again.
    dirty: bool,
    /// The current document has been protected at least once.
    applied: bool,
}

thread_local! {
    /// The protection script last installed in each tab.
    static SOURCES: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
    static TABS: std::cell::RefCell<HashMap<String, TabFp>> =
        std::cell::RefCell::new(HashMap::new());
}

fn enabled() -> bool {
    state::settings_snapshot()["fingerprintProtection"] != false
}

/// Installs (or replaces) the fingerprint script of one tab so it runs before
/// every new document, in the mode that matches whether an agent is
/// connected. Installs of one tab never overlap: a change that arrives while
/// one runs makes it run again with the newest mode.
pub async fn install(tab: String) {
    let start = TABS.with(|t| {
        let mut t = t.borrow_mut();
        let entry = t.entry(tab.clone()).or_default();
        if entry.busy {
            entry.dirty = true;
            false
        } else {
            entry.busy = true;
            true
        }
    });
    if !start {
        return;
    }
    loop {
        let agent = sessions::agent_present();
        let on = enabled();
        let previous = TABS.with(|t| {
            let mut t = t.borrow_mut();
            let entry = t.entry(tab.clone()).or_default();
            entry.dirty = false;
            entry.mode = None;
            entry.identifier.take()
        });
        if let Some(identifier) = previous {
            let _ = cdp::call(
                &tab,
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({"identifier": identifier}),
            )
            .await;
        }
        if on {
            let source = format!(
                "{}({},{});",
                SCRIPT.replace("@MARK@", &format!("{}.fp", super::adblock::page_marker())),
                base_seed(agent),
                !agent
            );
            SOURCES.with(|s| s.borrow_mut().insert(tab.clone(), source.clone()));
            let identifier = cdp::call(
                &tab,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": source}),
            )
            .await
            .ok()
            .and_then(|out| out["identifier"].as_str().map(str::to_string));
            let first = TABS.with(|t| {
                let mut t = t.borrow_mut();
                let entry = t.entry(tab.clone()).or_default();
                entry.identifier = identifier.clone();
                entry.mode = identifier.as_ref().map(|_| agent);
                !std::mem::replace(&mut entry.applied, true)
            });
            // A tab's first page can start loading before the script above is
            // registered: protect that document now as well. The script skips
            // a document it already protected.
            if first {
                let _ = cdp::call(
                    &tab,
                    "Runtime.evaluate",
                    json!({"expression": source, "returnByValue": true}),
                )
                .await;
            }
        } else {
            SOURCES.with(|s| s.borrow_mut().remove(&tab));
        }
        let again = TABS.with(|t| {
            let mut t = t.borrow_mut();
            let entry = t.entry(tab.clone()).or_default();
            let stale =
                entry.dirty || enabled() != on || (on && sessions::agent_present() != agent);
            if !stale {
                entry.busy = false;
            }
            stale
        });
        if !again {
            break;
        }
    }
}

/// Applies the tab's protection to one execution context (a document that
/// may have started before the script was registered). The script itself
/// skips a document it already protected.
pub async fn ensure(tab: String, context: i64) {
    let Some(source) = SOURCES.with(|s| s.borrow().get(&tab).cloned()) else {
        return;
    };
    let _ = cdp::call(
        &tab,
        "Runtime.evaluate",
        json!({"expression": source, "contextId": context, "returnByValue": true}),
    )
    .await;
}

/// When an agent connects or the last one leaves (or protection is switched
/// on or off), every tab switches to the matching fingerprint for its next
/// page load.
pub fn refresh_all() {
    let agent = sessions::agent_present();
    let on = enabled();
    for tab in cdp::tab_ids() {
        let stale = TABS.with(|t| {
            t.borrow().get(&tab).is_none_or(|entry| {
                entry.busy
                    || entry.mode != on.then_some(agent)
                    || (!on && entry.identifier.is_some())
            })
        });
        if stale {
            super::exec::spawn(install(tab));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browsing_base_is_stable_within_a_session() {
        assert_eq!(base_seed(false), base_seed(false));
    }
}
