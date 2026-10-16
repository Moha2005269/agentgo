//! Fingerprint protection in the style of Brave's "farbling".
//!
//! Canvas, WebGL read-backs, audio and a few hardware numbers are nudged by
//! amounts people cannot see, so trackers cannot build a stable identifier
//! from them.
//!
//! * No agent connected (the person browsing): the nudge changes with every
//!   browser session and with every site, so nothing links visits together.
//! * An agent connected: the browser stops randomising and uses one unique
//!   fingerprint that is created once and kept for good. Sites then see the
//!   same, consistent visitor on every run, which keeps logins and
//!   bot-defences from tripping on a changing identity.

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

/// Runs before the page's own scripts. `per_site` mixes the site into the
/// seed (browsing); the agent's fingerprint is the same everywhere.
const SCRIPT: &str = r#"(function(base,perSite){try{
if(location.protocol!=='http:'&&location.protocol!=='https:')return;
if(window.__agentgoFp)return;
Object.defineProperty(window,'__agentgoFp',{value:1});
const mix=(a,b)=>{let h=Math.imul(a^0x9E3779B9,0x85EBCA6B)^Math.imul(b+0x7F4A7C15,0xC2B2AE35);h^=h>>>13;h=Math.imul(h,0x27D4EB2F);return(h^h>>>16)>>>0};
let seed=base;
if(perSite){let h=7;for(const ch of location.hostname.split('.').slice(-2).join('.'))h=mix(h,ch.charCodeAt(0));seed=mix(base,h)}
const noisePixels=(d)=>{for(let i=0;i<d.length;i+=4){const h=mix(seed,i);if((h&63)===0){const c=(h>>>6)%3;d[i+c]^=1}}};
const mark=(fn,orig)=>{try{Object.defineProperty(fn,'toString',{value:()=>Function.prototype.toString.call(orig)})}catch(e){}return fn};
try{const orig=CanvasRenderingContext2D.prototype.getImageData;
CanvasRenderingContext2D.prototype.getImageData=mark(function(){const img=orig.apply(this,arguments);try{noisePixels(img.data)}catch(e){}return img},orig)}catch(e){}
const farbled=(canvas)=>{try{const w=canvas.width,h=canvas.height;if(!w||!h)return null;const copy=document.createElement('canvas');copy.width=w;copy.height=h;const c=copy.getContext('2d');c.drawImage(canvas,0,0);const img=CanvasRenderingContext2D.prototype.getImageData.call(c,0,0,w,h);c.putImageData(img,0,0);return copy}catch(e){return null}};
try{const orig=HTMLCanvasElement.prototype.toDataURL;
HTMLCanvasElement.prototype.toDataURL=mark(function(){const copy=farbled(this);return orig.apply(copy||this,arguments)},orig)}catch(e){}
try{const orig=HTMLCanvasElement.prototype.toBlob;
HTMLCanvasElement.prototype.toBlob=mark(function(){const copy=farbled(this);return orig.apply(copy||this,arguments)},orig)}catch(e){}
for(const Ctx of [window.WebGLRenderingContext,window.WebGL2RenderingContext]){if(!Ctx)continue;try{const orig=Ctx.prototype.readPixels;
Ctx.prototype.readPixels=mark(function(){orig.apply(this,arguments);const buf=arguments[6];if(buf&&buf.length&&buf instanceof Uint8Array)noisePixels(buf)},orig)}catch(e){}}
try{const orig=AudioBuffer.prototype.getChannelData;const done=new WeakSet();
AudioBuffer.prototype.getChannelData=mark(function(){const data=orig.apply(this,arguments);if(!done.has(data)){done.add(data);for(let i=0;i<data.length;i+=97){data[i]+=((mix(seed,i)&255)-128)*1e-8}}return data},orig)}catch(e){}
try{const orig=AnalyserNode.prototype.getFloatFrequencyData;
AnalyserNode.prototype.getFloatFrequencyData=mark(function(a){orig.apply(this,arguments);for(let i=0;i<a.length;i+=7)a[i]+=((mix(seed,i)&255)-128)*1e-6},orig)}catch(e){}
const pick=(list,salt)=>list[mix(seed,salt)%list.length];
try{Object.defineProperty(Navigator.prototype,'hardwareConcurrency',{get:mark(function(){return pick([4,6,8,12],1)},Object.getOwnPropertyDescriptor(Navigator.prototype,'hardwareConcurrency').get),configurable:true,enumerable:true})}catch(e){}
try{Object.defineProperty(Navigator.prototype,'deviceMemory',{get:mark(function(){return pick([4,8],2)},Object.getOwnPropertyDescriptor(Navigator.prototype,'deviceMemory').get),configurable:true,enumerable:true})}catch(e){}
}catch(e){}})"#;

thread_local! {
    /// DevTools identifier of the script installed in each tab, and the mode
    /// (agent or not) it was built for.
    static INSTALLED: std::cell::RefCell<(HashMap<String, String>, Option<bool>)> =
        std::cell::RefCell::new((HashMap::new(), None));
}

/// Installs (or replaces) the fingerprint script of one tab so it runs before
/// every new document. Called when a tab is created.
pub async fn install(tab: String) {
    let agent = sessions::agent_present();
    let previous = INSTALLED.with(|i| i.borrow_mut().0.remove(&tab));
    if let Some(identifier) = previous {
        let _ = cdp::call(
            &tab,
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({"identifier": identifier}),
        )
        .await;
    }
    if state::settings_snapshot()["fingerprintProtection"] == false {
        return;
    }
    let source = format!("{SCRIPT}({},{});", base_seed(agent), !agent);
    if let Ok(out) = cdp::call(
        &tab,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": source}),
    )
    .await
    {
        if let Some(identifier) = out["identifier"].as_str() {
            INSTALLED.with(|i| {
                let mut i = i.borrow_mut();
                i.0.insert(tab, identifier.to_string());
                i.1 = Some(agent);
            });
        }
    }
}

/// When an agent connects or the last one leaves, every tab switches to the
/// matching fingerprint for its next page load.
pub fn refresh_all() {
    let agent = sessions::agent_present();
    let changed = INSTALLED.with(|i| i.borrow().1.is_some_and(|mode| mode != agent));
    if !changed {
        return;
    }
    for tab in cdp::tab_ids() {
        super::exec::spawn(install(tab));
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
