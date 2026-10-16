//! Seeing and inspecting the page beyond the outline: OCR and visual
//! grounding (Windows' own OCR on a screenshot of the tab), picture diffs,
//! reference expiry and stored snapshots, spell checking, DevTools tracing,
//! CPU profiles and coverage, DNS, service workers and background services,
//! screen-reader mode, page media (full screen, picture-in-picture, frames)
//! and the gated `runtime.*` page-script tools.

use std::cell::RefCell;
use std::collections::BTreeMap;

use agentgo_protocol::ErrorCode;
use cef::{ImplBrowserHost, ImplRequestContext};
use resolve::Resolved;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, exec, native, page, state, tasks};

thread_local! {
    /// tab -> last screenshot taken by observe.visual_diff.
    static LAST_SHOT: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    static SNAPSHOTS: RefCell<BTreeMap<String, Value>> = const { RefCell::new(BTreeMap::new()) };
    /// tab -> path of the last trace written by debug.trace_stop.
    static TRACES: RefCell<BTreeMap<String, Value>> = const { RefCell::new(BTreeMap::new()) };
}

async fn shot(caller: &Caller, tab: &str) -> Result<String, Fail> {
    let taken = page::screenshot(caller, "observe.screenshot", &json!({"tabId": tab})).await?;
    taken["path"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Fail::internal("the screenshot was not saved"))
}

async fn scale(tab: &str) -> f64 {
    cdp::agent_eval(tab, "devicePixelRatio", 2_000)
        .await
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0)
        .unwrap_or(1.0)
}

/// Picture pixels to page (CSS) pixels.
fn to_css(value: &Value, dpr: f64) -> Value {
    let get = |key: &str| value[key].as_f64().unwrap_or(0.0) / dpr;
    let (x, y, w, h) = (get("x"), get("y"), get("width"), get("height"));
    json!({"x": x.round(), "y": y.round(), "width": w.round(), "height": h.round(), "centerX": (x + w / 2.0).round(), "centerY": (y + h / 2.0).round()})
}

async fn ocr(caller: &Caller, tab: &str) -> Reply {
    let path = shot(caller, tab).await?;
    let dpr = scale(tab).await;
    let mut out = native::helper(json!({"op": "ocr", "path": path}), 60_000).await?;
    for line in out["lines"].as_array_mut().into_iter().flatten() {
        let mut words: Vec<Value> = Vec::new();
        for word in line["words"].as_array().cloned().unwrap_or_default() {
            let mut placed = to_css(&word, dpr);
            placed["text"] = word["text"].clone();
            words.push(placed);
        }
        // The line's box is the union of its words.
        if let (Some(first), Some(last)) = (words.first(), words.last()) {
            let x = first["x"].as_f64().unwrap_or(0.0);
            let y = words
                .iter()
                .filter_map(|w| w["y"].as_f64())
                .fold(f64::MAX, f64::min);
            let right = last["x"].as_f64().unwrap_or(0.0) + last["width"].as_f64().unwrap_or(0.0);
            let bottom = words
                .iter()
                .map(|w| w["y"].as_f64().unwrap_or(0.0) + w["height"].as_f64().unwrap_or(0.0))
                .fold(0.0, f64::max);
            line["box"] = json!({"x": x, "y": y, "width": right - x, "height": bottom - y, "centerX": ((x + right) / 2.0).round(), "centerY": ((y + bottom) / 2.0).round()});
        }
        line["words"] = json!(words);
    }
    out["screenshot"] = json!(path);
    out["units"] = json!("page pixels: act.click {x, y} takes them");
    Ok(out)
}

async fn diff(before: &str, after: &str, dpr: f64) -> Reply {
    let mut out = native::helper(
        json!({"op": "diff", "before": before, "after": after}),
        60_000,
    )
    .await?;
    let regions: Vec<Value> = out["regions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| to_css(r, dpr))
        .collect();
    out["regions"] = json!(regions);
    out["before"] = json!(before);
    out["after"] = json!(after);
    Ok(out)
}

/// The element a media or runtime call means: ref, or find {by, ...}.
async fn element(caller: &Caller, tab: &str, params: &Value) -> Result<String, Fail> {
    let mut target = params.clone();
    target["tabId"] = json!(tab);
    tasks::resolve(caller, &mut target).await?;
    let id = target["ref"]
        .as_u64()
        .or_else(|| {
            target["ref"]
                .as_str()
                .and_then(|s| s.trim_matches(['[', ']']).parse().ok())
        })
        .ok_or_else(|| Fail::params("name the element: ref, or find {by, ...}"))?;
    cdp::agent_object(tab, &format!("__ag.element({id})"))
        .await
        .map_err(|e| Fail::new(ErrorCode::TargetStale, e))
}

/// Runs a function on an element as if the person had just clicked
/// (full screen and picture-in-picture need that gesture).
async fn on_element(tab: &str, object: &str, function: &str) -> Reply {
    let out = cdp::call(tab, "Runtime.callFunctionOn", json!({
        "objectId": object, "functionDeclaration": function, "userGesture": true, "awaitPromise": true, "returnByValue": true,
    })).await.map_err(Fail::internal)?;
    if let Some(error) = out["exceptionDetails"]["exception"]["description"].as_str() {
        return Err(Fail::new(
            ErrorCode::TargetBlocked,
            error.lines().next().unwrap_or(error).to_string(),
        ));
    }
    Ok(out["result"]["value"].clone())
}

fn evaluate_allowed(caller: &Caller, method: &str) -> Result<(), Fail> {
    if caller.is_shell()
        || state::control_mode() == "full"
        || state::option_enabled("allowEvaluate")
    {
        return Ok(());
    }
    Err(Fail::new(
        ErrorCode::PolicyDenied,
        format!("{method} runs page script: it needs full-access mode or --allow-evaluate"),
    ))
}

mod resolve {
    use super::exec;
    use cef::*;

    wrap_resolve_callback! {
        pub struct Resolved {
            done: exec::Slot<(bool, Vec<String>)>,
        }

        impl ResolveCallback {
            fn on_resolve_completed(&self, result: Errorcode, resolved_ips: Option<&mut CefStringList>) {
                let ok = *result.as_ref() == sys::cef_errorcode_t::ERR_NONE;
                let ips = resolved_ips.map(|list| list.clone().into_iter().collect()).unwrap_or_default();
                self.done.fill((ok, ips));
            }
        }
    }
}

async fn read_stream(tab: &str, handle: &str) -> Result<Vec<u8>, Fail> {
    let mut data = Vec::new();
    loop {
        let chunk = cdp::call(tab, "IO.read", json!({"handle": handle, "size": 1_048_576}))
            .await
            .map_err(Fail::internal)?;
        let text = chunk["data"].as_str().unwrap_or("");
        if chunk["base64Encoded"] == true {
            data.extend(page::decode_base64(text));
        } else {
            data.extend(text.as_bytes());
        }
        if chunk["eof"] == true || data.len() > 200_000_000 {
            break;
        }
    }
    let _ = cdp::call(tab, "IO.close", json!({"handle": handle})).await;
    Ok(data)
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "observe.ocr"
            | "ground.visual"
            | "observe.visual_diff"
            | "observe.paint_regions"
            | "state.ref_expire"
            | "state.snapshot_store"
            | "state.snapshot_load"
            | "edit.spellcheck"
            | "debug.trace_start"
            | "debug.trace_stop"
            | "debug.trace_export"
            | "debug.profile_cpu"
            | "debug.coverage"
            | "network.dns_status"
            | "network.service_worker_events"
            | "network.background_tasks"
            | "a11y.screen_reader_mode"
            | "media.fullscreen"
            | "media.picture_in_picture"
            | "media.capture_frame"
            | "dialog.fullscreen"
            | "runtime.call_function"
            | "runtime.add_init_script"
            | "runtime.remove_init_script"
            | "runtime.add_binding"
            | "runtime.remove_binding"
            | "runtime.message"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    sessions::check_handoff(caller, &tab)?;
    match method {
        "observe.ocr" => ocr(caller, &tab).await,
        "ground.visual" => {
            let text = params["text"]
                .as_str()
                .ok_or_else(|| Fail::params("text (what to find on screen) is required"))?
                .to_lowercase();
            let read = ocr(caller, &tab).await?;
            let mut matches = Vec::new();
            for line in read["lines"].as_array().cloned().unwrap_or_default() {
                let words = line["words"].as_array().cloned().unwrap_or_default();
                let joined = line["text"].as_str().unwrap_or("").to_lowercase();
                if !joined.contains(&text) {
                    continue;
                }
                // The words that make up the match, else the whole line.
                let want: Vec<&str> = text.split_whitespace().collect();
                let start = (0..words.len()).find(|&i| {
                    want.iter().enumerate().all(|(k, w)| {
                        words
                            .get(i + k)
                            .and_then(|word| word["text"].as_str())
                            .is_some_and(|t| t.to_lowercase().contains(w))
                    })
                });
                let hit = match start {
                    Some(i) => {
                        let part = &words[i..(i + want.len()).min(words.len())];
                        let x = part[0]["x"].as_f64().unwrap_or(0.0);
                        let y = part
                            .iter()
                            .filter_map(|w| w["y"].as_f64())
                            .fold(f64::MAX, f64::min);
                        let last = &part[part.len() - 1];
                        let right = last["x"].as_f64().unwrap_or(0.0)
                            + last["width"].as_f64().unwrap_or(0.0);
                        let bottom = part
                            .iter()
                            .map(|w| {
                                w["y"].as_f64().unwrap_or(0.0) + w["height"].as_f64().unwrap_or(0.0)
                            })
                            .fold(0.0, f64::max);
                        json!({"x": ((x + right) / 2.0).round(), "y": ((y + bottom) / 2.0).round(), "box": {"x": x, "y": y, "width": right - x, "height": bottom - y}})
                    }
                    None => {
                        json!({"x": line["box"]["centerX"], "y": line["box"]["centerY"], "box": line["box"]})
                    }
                };
                matches.push(
                    json!({"text": line["text"], "x": hit["x"], "y": hit["y"], "box": hit["box"]}),
                );
            }
            if matches.is_empty() {
                return Err(Fail::new(
                    ErrorCode::TargetStale,
                    format!("{text:?} is not visible on screen"),
                )
                .with(json!({"seen": read["text"]})));
            }
            Ok(
                json!({"x": matches[0]["x"], "y": matches[0]["y"], "matches": matches, "next": "act.click {x, y}"}),
            )
        }
        "observe.visual_diff" => {
            let dpr = scale(&tab).await;
            let after = shot(caller, &tab).await?;
            let before = match params["before"].as_str() {
                Some(path) => path.to_string(),
                None => match LAST_SHOT.with(|s| s.borrow_mut().insert(tab.clone(), after.clone()))
                {
                    Some(previous) => previous,
                    None => {
                        return Ok(
                            json!({"baseline": after, "note": "first picture of this tab; call again to see what changed"}),
                        )
                    }
                },
            };
            LAST_SHOT.with(|s| s.borrow_mut().insert(tab.clone(), after.clone()));
            diff(&before, &after, dpr).await
        }
        "observe.paint_regions" => {
            let dpr = scale(&tab).await;
            let first = shot(caller, &tab).await?;
            exec::sleep(params["ms"].as_u64().unwrap_or(600).clamp(100, 5_000)).await;
            let second = shot(caller, &tab).await?;
            let mut out = diff(&first, &second, dpr).await?;
            out["note"] = json!("regions that repainted between the two pictures (animations, videos, live content)");
            Ok(out)
        }
        "state.ref_expire" => {
            let out = page::agent(
                &tab,
                "expire",
                json!([params.get("refs").cloned().unwrap_or(Value::Null)]),
                4_000,
            )
            .await?;
            Ok(
                json!({"tabId": tab, "expired": out["expired"], "next": "observe.page gives fresh references"}),
            )
        }
        "state.snapshot_store" => {
            let name = params["name"]
                .as_str()
                .unwrap_or("last")
                .chars()
                .take(64)
                .collect::<String>();
            let view = page::observe_page(caller, &json!({"tabId": tab, "max": 40_000})).await?;
            let snapshot = json!({"name": name, "url": state::tab_url(&tab), "page": view["page"], "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())});
            SNAPSHOTS.with(|s| {
                s.borrow_mut()
                    .insert(format!("{}/{name}", caller.session), snapshot)
            });
            Ok(
                json!({"stored": name, "url": state::tab_url(&tab), "lines": view["page"].as_str().map_or(0, |p| p.lines().count())}),
            )
        }
        "state.snapshot_load" => {
            let name = params["name"].as_str().unwrap_or("last");
            let stored = SNAPSHOTS
                .with(|s| {
                    s.borrow()
                        .get(&format!("{}/{name}", caller.session))
                        .cloned()
                })
                .ok_or_else(|| Fail::params(format!("no snapshot named {name}")))?;
            let mut out = stored.clone();
            if params["compare"] != false {
                let now = page::observe_page(caller, &json!({"tabId": tab, "max": 40_000})).await?;
                let old: Vec<&str> = stored["page"].as_str().unwrap_or("").lines().collect();
                let new: Vec<&str> = now["page"].as_str().unwrap_or("").lines().collect();
                let strip = |l: &str| {
                    l.trim_start_matches(|c: char| {
                        c == '[' || c.is_ascii_digit() || c == ']' || c == ' '
                    })
                    .to_string()
                };
                let old_set: std::collections::HashSet<String> =
                    old.iter().map(|l| strip(l)).collect();
                let new_set: std::collections::HashSet<String> =
                    new.iter().map(|l| strip(l)).collect();
                out["added"] = json!(new
                    .iter()
                    .filter(|l| !old_set.contains(&strip(l)))
                    .take(80)
                    .collect::<Vec<_>>());
                out["removed"] = json!(old
                    .iter()
                    .filter(|l| !new_set.contains(&strip(l)))
                    .take(80)
                    .collect::<Vec<_>>());
                out["sameUrl"] =
                    json!(stored["url"].as_str() == Some(state::tab_url(&tab).as_str()));
            }
            Ok(out)
        }
        "edit.spellcheck" => {
            let text = match params["text"].as_str() {
                Some(text) => text.to_string(),
                None => {
                    let object = element(caller, &tab, params).await?;
                    on_element(&tab, &object, "function(){return this.isContentEditable ? this.innerText : (this.value ?? this.textContent)}").await?.as_str().unwrap_or("").to_string()
                }
            };
            native::helper(json!({"op": "spellcheck", "text": text.chars().take(20_000).collect::<String>(), "language": params["language"]}), 60_000).await
        }
        "debug.trace_start" => {
            let categories = params["categories"].as_array().cloned().unwrap_or_else(|| {
                vec![
                    json!("devtools.timeline"),
                    json!("v8.execute"),
                    json!("blink.user_timing"),
                    json!("loading"),
                ]
            });
            let _ = cdp::events(&tab, &["trace-complete"], 10, true);
            cdp::call(&tab, "Tracing.start", json!({"transferMode": "ReturnAsStream", "traceConfig": {"includedCategories": categories}})).await.map_err(Fail::internal)?;
            Ok(json!({"tracing": true, "categories": categories}))
        }
        "debug.trace_stop" => {
            cdp::call(&tab, "Tracing.end", json!({}))
                .await
                .map_err(Fail::internal)?;
            let started = std::time::Instant::now();
            let stream = loop {
                if let Some(done) = cdp::events(&tab, &["trace-complete"], 1, true).pop() {
                    break done["stream"].as_str().map(str::to_string);
                }
                if started.elapsed().as_secs() > 30 {
                    return Err(Fail::new(ErrorCode::Timeout, "the trace did not finish"));
                }
                exec::sleep(100).await;
            };
            let stream = stream.ok_or_else(|| Fail::internal("the trace has no data"))?;
            let data = read_stream(&tab, &stream).await?;
            let parsed: Value = serde_json::from_slice(&data).unwrap_or(Value::Null);
            let events = parsed["traceEvents"]
                .as_array()
                .or(parsed.as_array())
                .map_or(0, Vec::len);
            let mut saved = page::save_artifact(caller, "trace", "json", &data)?;
            saved["events"] = json!(events);
            TRACES.with(|t| t.borrow_mut().insert(tab.clone(), saved.clone()));
            Ok(saved)
        }
        "debug.trace_export" => {
            let trace = TRACES
                .with(|t| t.borrow().get(&tab).cloned())
                .ok_or_else(|| {
                    Fail::params("no trace yet: debug.trace_start, then debug.trace_stop")
                })?;
            let bytes = std::fs::read(trace["path"].as_str().unwrap_or(""))
                .map_err(|e| Fail::internal(e.to_string()))?;
            let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let mut names: BTreeMap<String, u64> = BTreeMap::new();
            for event in parsed["traceEvents"]
                .as_array()
                .or(parsed.as_array())
                .into_iter()
                .flatten()
            {
                *names
                    .entry(event["name"].as_str().unwrap_or("?").to_string())
                    .or_default() += 1;
            }
            let mut top: Vec<(String, u64)> = names.into_iter().collect();
            top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            Ok(
                json!({"path": trace["path"], "events": trace["events"], "top": top.into_iter().take(15).map(|(name, n)| json!({"name": name, "count": n})).collect::<Vec<_>>(), "open": "chrome://tracing or https://ui.perfetto.dev reads this file"}),
            )
        }
        "debug.profile_cpu" => {
            let ms = params["ms"].as_u64().unwrap_or(1_000).clamp(100, 30_000);
            cdp::call(&tab, "Profiler.enable", json!({}))
                .await
                .map_err(Fail::internal)?;
            cdp::call(&tab, "Profiler.start", json!({}))
                .await
                .map_err(Fail::internal)?;
            exec::sleep(ms).await;
            let profile = cdp::call(&tab, "Profiler.stop", json!({}))
                .await
                .map_err(Fail::internal)?["profile"]
                .clone();
            let mut by_function: BTreeMap<String, u64> = BTreeMap::new();
            let mut samples = 0u64;
            for node in profile["nodes"].as_array().into_iter().flatten() {
                let hits = node["hitCount"].as_u64().unwrap_or(0);
                samples += hits;
                let frame = &node["callFrame"];
                let name = format!(
                    "{} {}:{}",
                    frame["functionName"]
                        .as_str()
                        .filter(|n| !n.is_empty())
                        .unwrap_or("(anonymous)"),
                    frame["url"].as_str().unwrap_or(""),
                    frame["lineNumber"]
                );
                *by_function.entry(name).or_default() += hits;
            }
            let mut top: Vec<(String, u64)> =
                by_function.into_iter().filter(|(_, h)| *h > 0).collect();
            top.sort_by_key(|(_, h)| std::cmp::Reverse(*h));
            let saved =
                page::save_artifact(caller, "cpu", "cpuprofile", profile.to_string().as_bytes())?;
            Ok(
                json!({"ms": ms, "samples": samples, "path": saved["path"], "top": top.into_iter().take(12).map(|(f, h)| json!({"function": f, "samples": h})).collect::<Vec<_>>()}),
            )
        }
        "debug.coverage" => {
            let ms = params["ms"].as_u64().unwrap_or(1_000).clamp(0, 30_000);
            cdp::call(&tab, "Profiler.enable", json!({}))
                .await
                .map_err(Fail::internal)?;
            cdp::call(
                &tab,
                "Profiler.startPreciseCoverage",
                json!({"callCount": true, "detailed": false}),
            )
            .await
            .map_err(Fail::internal)?;
            let _ = cdp::call(&tab, "DOM.enable", json!({})).await;
            let css = cdp::call(&tab, "CSS.enable", json!({})).await.is_ok()
                && cdp::call(&tab, "CSS.startRuleUsageTracking", json!({}))
                    .await
                    .is_ok();
            exec::sleep(ms).await;
            let js = cdp::call(&tab, "Profiler.takePreciseCoverage", json!({}))
                .await
                .map_err(Fail::internal)?;
            let _ = cdp::call(&tab, "Profiler.stopPreciseCoverage", json!({})).await;
            let scripts: Vec<Value> = js["result"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s["url"].as_str().is_some_and(|u| !u.is_empty()))
                .map(|script| {
                    let functions = script["functions"].as_array().cloned().unwrap_or_default();
                    let used = functions
                        .iter()
                        .filter(|f| f["ranges"][0]["count"].as_u64().unwrap_or(0) > 0)
                        .count();
                    json!({"url": script["url"], "functions": functions.len(), "used": used})
                })
                .collect();
            let rules = if css {
                let usage = cdp::call(&tab, "CSS.stopRuleUsageTracking", json!({}))
                    .await
                    .map_err(Fail::internal)?;
                let all = usage["ruleUsage"].as_array().cloned().unwrap_or_default();
                json!({"rules": all.len(), "used": all.iter().filter(|r| r["used"] == true).count()})
            } else {
                Value::Null
            };
            Ok(json!({"scripts": scripts, "css": rules, "ms": ms}))
        }
        "network.dns_status" => {
            let host = params["host"]
                .as_str()
                .map(str::to_string)
                .or_else(|| {
                    state::tab_url(&tab)
                        .split("://")
                        .nth(1)
                        .and_then(|r| r.split(['/', ':']).next())
                        .map(str::to_string)
                })
                .ok_or_else(|| Fail::params("host is required"))?;
            let context = state::content_context()
                .ok_or_else(|| Fail::internal("the profile is not ready"))?;
            let slot = exec::Slot::default();
            let mut callback = Resolved::new(slot.clone());
            let started = std::time::Instant::now();
            context.resolve_host(
                Some(&cef::CefString::from(format!("https://{host}").as_str())),
                Some(&mut callback),
            );
            let (ok, addresses) = slot
                .wait(15_000)
                .await
                .ok_or_else(|| Fail::new(ErrorCode::Timeout, "the name did not resolve in time"))?;
            Ok(
                json!({"host": host, "resolved": ok, "addresses": addresses, "ms": started.elapsed().as_millis() as u64}),
            )
        }
        "network.service_worker_events" => {
            cdp::call(&tab, "ServiceWorker.enable", json!({}))
                .await
                .map_err(Fail::internal)?;
            exec::sleep(params["ms"].as_u64().unwrap_or(400).clamp(0, 10_000)).await;
            Ok(json!({"events": cdp::events(&tab, &["sw"], 100, params["clear"] == true)}))
        }
        "network.background_tasks" => {
            for service in [
                "backgroundFetch",
                "backgroundSync",
                "pushMessaging",
                "notifications",
                "paymentHandler",
                "periodicBackgroundSync",
            ] {
                let _ = cdp::call(
                    &tab,
                    "BackgroundService.startObserving",
                    json!({"service": service}),
                )
                .await;
            }
            exec::sleep(params["ms"].as_u64().unwrap_or(300).clamp(0, 10_000)).await;
            Ok(
                json!({"events": cdp::events(&tab, &["background"], 100, params["clear"] == true), "observing": ["backgroundFetch", "backgroundSync", "pushMessaging", "notifications", "paymentHandler", "periodicBackgroundSync"]}),
            )
        }
        "a11y.screen_reader_mode" => {
            let on = params["on"] != false;
            for id in state::content_tab_ids() {
                if let Some(host) = state::content_host(&id) {
                    host.set_accessibility_state(if on {
                        cef::State::ENABLED
                    } else {
                        cef::State::DEFAULT
                    });
                }
            }
            Ok(
                json!({"screenReaderMode": on, "note": "Chromium builds its full accessibility tree for every tab, as when a screen reader runs"}),
            )
        }
        "media.fullscreen" | "media.picture_in_picture" => {
            if params["exit"] == true {
                let script = if method == "media.fullscreen" {
                    "document.fullscreenElement ? document.exitFullscreen().then(() => true) : false"
                } else {
                    "document.pictureInPictureElement ? document.exitPictureInPicture().then(() => true) : false"
                };
                let left = cdp::page_eval(&tab, script, 5_000)
                    .await
                    .map_err(Fail::internal)?;
                return Ok(json!({"exited": left}));
            }
            let object = element(caller, &tab, params).await?;
            let function = if method == "media.fullscreen" {
                "function(){return this.requestFullscreen().then(() => !!document.fullscreenElement)}"
            } else {
                "function(){if (!(this instanceof HTMLVideoElement)) throw new Error('picture-in-picture needs a video element'); return this.requestPictureInPicture().then(() => true)}"
            };
            let shown = on_element(&tab, &object, function).await?;
            Ok(json!({"shown": shown, "exit": format!("{method} {{exit: true}}")}))
        }
        "media.capture_frame" => {
            let object = element(caller, &tab, params).await?;
            let data = on_element(&tab, &object, "function(){const w = this.videoWidth || this.naturalWidth || this.width, h = this.videoHeight || this.naturalHeight || this.height; if (!w || !h) throw new Error('nothing is showing yet'); const c = document.createElement('canvas'); c.width = w; c.height = h; c.getContext('2d').drawImage(this, 0, 0, w, h); return [c.toDataURL('image/png'), w, h, this.currentTime ?? null]}").await?;
            let url = data[0].as_str().unwrap_or("");
            let bytes = page::decode_base64(url.split_once(',').map_or("", |(_, b)| b));
            let mut saved = page::save_artifact(caller, "frame", "png", &bytes)?;
            saved["width"] = data[1].clone();
            saved["height"] = data[2].clone();
            saved["time"] = data[3].clone();
            Ok(saved)
        }
        "dialog.fullscreen" => {
            let state_now = cdp::page_eval(&tab, "!!document.fullscreenElement", 3_000)
                .await
                .map_err(Fail::internal)?;
            if params["exit"] == true && state_now == true {
                cdp::page_eval(&tab, "document.exitFullscreen().then(() => true)", 5_000)
                    .await
                    .map_err(Fail::internal)?;
                return Ok(json!({"fullscreen": false, "exited": true}));
            }
            Ok(
                json!({"fullscreen": state_now, "next": "dialog.fullscreen {exit: true} leaves the page's full screen"}),
            )
        }
        "runtime.call_function" => {
            evaluate_allowed(caller, method)?;
            let function = params["function"].as_str().ok_or_else(|| {
                Fail::params("function (its source, e.g. \"(a, b) => a + b\") is required")
            })?;
            let args = params.get("args").cloned().unwrap_or(json!([]));
            if params.get("ref").is_some() || params.get("find").is_some() {
                let object = element(caller, &tab, params).await?;
                let out = cdp::call(&tab, "Runtime.callFunctionOn", json!({"objectId": object, "functionDeclaration": format!("function(...a){{return ({function}).call(this, this, ...a)}}"), "arguments": args.as_array().into_iter().flatten().map(|a| json!({"value": a})).collect::<Vec<_>>(), "awaitPromise": true, "returnByValue": true})).await.map_err(Fail::internal)?;
                if let Some(error) = out["exceptionDetails"]["exception"]["description"].as_str() {
                    return Err(Fail::params(error.to_string()));
                }
                return Ok(
                    json!({"value": out["result"]["value"], "world": "AgentGo's isolated world, with the element as first argument"}),
                );
            }
            let value = cdp::page_eval(&tab, &format!("({function})(...{args})"), 10_000)
                .await
                .map_err(Fail::params)?;
            Ok(json!({"value": value}))
        }
        "runtime.add_init_script" => {
            evaluate_allowed(caller, method)?;
            let source = params["source"]
                .as_str()
                .ok_or_else(|| Fail::params("source is required"))?;
            let out = cdp::call(
                &tab,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": source}),
            )
            .await
            .map_err(Fail::internal)?;
            Ok(
                json!({"identifier": out["identifier"], "runs": "before every new document of this tab"}),
            )
        }
        "runtime.remove_init_script" => {
            let id = params["identifier"].as_str().ok_or_else(|| {
                Fail::params("identifier (from runtime.add_init_script) is required")
            })?;
            cdp::call(
                &tab,
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({"identifier": id}),
            )
            .await
            .map_err(Fail::params)?;
            Ok(json!({"removed": id}))
        }
        "runtime.add_binding" | "runtime.remove_binding" => {
            evaluate_allowed(caller, method)?;
            let name = params["name"]
                .as_str()
                .filter(|n| {
                    !n.is_empty()
                        && n.len() <= 64
                        && !n.starts_with("__ag")
                        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
                .ok_or_else(|| Fail::params("name: letters, digits and _ (not __ag...)"))?;
            let domain = if method == "runtime.add_binding" {
                "Runtime.addBinding"
            } else {
                "Runtime.removeBinding"
            };
            cdp::call(&tab, domain, json!({"name": name}))
                .await
                .map_err(Fail::internal)?;
            Ok(
                json!({"binding": name, "added": method == "runtime.add_binding", "read": "runtime.message returns what the page sent with window.<name>(text)"}),
            )
        }
        "runtime.message" => {
            if let Some(data) = params.get("data") {
                evaluate_allowed(caller, method)?;
                cdp::page_eval(&tab, &format!("window.dispatchEvent(new MessageEvent('message', {{data: {data}, origin: 'agentgo'}})), true"), 5_000).await.map_err(Fail::internal)?;
            }
            Ok(
                json!({"sent": params.get("data").is_some(), "received": cdp::events(&tab, &["binding"], 100, true)}),
            )
        }
        _ => Err(Fail::internal(format!(
            "{method} is not an inspection command"
        ))),
    }
}
