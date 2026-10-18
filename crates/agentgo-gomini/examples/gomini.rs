//! Manual check of a GoMini installation, outside the browser.
//!
//! ```text
//! cargo run -p agentgo-gomini --example gomini -- install [root]
//! cargo run -p agentgo-gomini --example gomini -- choose [root] < record.json
//! cargo run -p agentgo-gomini --example gomini -- pack [root] < record.json
//! ```
//!
//! `install` downloads (or reuses) everything and runs the self-test;
//! `choose` and `pack` read one `decision.record` answer (with optional
//! `text`, `recent`, `executor` fields) and print the request, its token ids
//! and, for `choose`, GoMini's answer.

use std::io::Read;
use std::sync::atomic::AtomicBool;

use agentgo_gomini::{install, pack, Layout, Server};
use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("install");
    let root = args
        .get(2)
        .map(std::path::PathBuf::from)
        .or_else(Layout::default_root)
        .ok_or("no root")?;
    let layout = Layout::new(root);
    if command == "install" {
        let cancel = AtomicBool::new(false);
        let found = install::install(&layout, &cancel, &|p| {
            eprintln!("{:>8} {:>5.1}% {}", p.stage, p.done as f64 * 100.0 / p.total.max(1) as f64, p.text)
        })?;
        println!("{}", found.to_json());
        return Ok(());
    }
    let installed = install::installed(&layout).ok_or("not installed; run install first")?;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
    let record: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    let executor = pack::Executor {
        cancelled: record["executor"]["cancelled"] == true,
        attempts_remaining: record["executor"]["attempts_remaining"].as_u64().unwrap_or(3),
    };
    let recent: Vec<Value> = record["recent"].as_array().cloned().unwrap_or_default();
    let (request, _) =
        pack::from_record(&record, record["text"].as_str().unwrap_or(""), &recent, &executor, record["field_readback"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))?;
    let server = Server::start(&layout, &installed)?;
    let packed = pack::check(&server, &request)?;
    let mut out = json!({"body": pack::body(&request), "ids": packed.ids});
    if command == "choose" {
        let choice = agentgo_gomini::choose(&server, &request)?;
        out["choice"] = json!(choice.choice);
        out["top"] = json!(choice.top_probability);
        out["probabilities"] = json!(choice.probabilities);
    }
    println!("{out}");
    Ok(())
}
