//! The VPN was removed: it lost connections more often than it helped. The
//! settings document keeps a `vpn` key that is always off.

use serde_json::{json, Value};

pub fn start(_settings: &Value) {}

pub fn apply(_settings: &Value) {}

pub fn decorate(settings: &mut Value) {
    if let Some(map) = settings.as_object_mut() {
        map.insert("vpn".into(), json!(false));
        map.insert("vpnState".into(), json!("off"));
    }
}
