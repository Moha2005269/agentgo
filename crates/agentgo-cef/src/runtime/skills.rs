//! Local skills: named, parameterized step lists kept in AgentGo's data
//! folder (`skills/<name>/skill.json`), made by hand, installed from a folder
//! or compiled from a workflow recording. A skill is data, not code: its
//! steps run through the task engine and the normal dispatcher, so every
//! gate applies. Also the recording checks `workflow.diff`, `verify` and
//! `approve`.

use std::path::PathBuf;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Map, Value};

use super::sessions::{Caller, Fail, Reply};
use super::{dispatch, recorder, state, tasks};

fn root() -> PathBuf {
    state::data_dir().join("skills")
}

fn checked(name: &str) -> Result<&str, Fail> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Fail::params(
            "a skill name is 1 to 64 letters, digits, - or _",
        ));
    }
    Ok(name)
}

fn file(name: &str) -> Result<PathBuf, Fail> {
    Ok(root().join(checked(name)?).join("skill.json"))
}

fn name_of(params: &Value) -> Result<String, Fail> {
    params["name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Fail::params("name is required"))
}

fn load(name: &str) -> Result<Value, Fail> {
    let bytes = std::fs::read(file(name)?)
        .map_err(|_| Fail::params(format!("there is no skill named {name}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| Fail::params(format!("skill {name} is not valid JSON: {e}")))
}

fn store(skill: &Value) -> Result<(), Fail> {
    let path = file(skill["name"].as_str().unwrap_or(""))?;
    std::fs::create_dir_all(path.parent().expect("skill folder"))
        .map_err(|e| Fail::internal(e.to_string()))?;
    std::fs::write(&path, serde_json::to_vec_pretty(skill).unwrap_or_default())
        .map_err(|e| Fail::internal(e.to_string()))
}

/// `{{name}}` placeholders used in a value.
fn placeholders(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let mut rest = text.as_str();
            while let Some(start) = rest.find("{{") {
                let Some(end) = rest[start..].find("}}") else {
                    break;
                };
                let name = rest[start + 2..start + end].trim().to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
                rest = &rest[start + end + 2..];
            }
        }
        Value::Array(items) => items.iter().for_each(|v| placeholders(v, out)),
        Value::Object(map) => map.values().for_each(|v| placeholders(v, out)),
        _ => {}
    }
}

/// Fills placeholders; a value that is only one placeholder keeps its type.
fn fill(value: &Value, values: &Map<String, Value>) -> Value {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with("{{")
                && trimmed.ends_with("}}")
                && trimmed.matches("{{").count() == 1
            {
                if let Some(v) = values.get(trimmed[2..trimmed.len() - 2].trim()) {
                    return v.clone();
                }
            }
            let mut out = text.clone();
            for (key, v) in values {
                let with = v.as_str().map_or(v.to_string(), str::to_string);
                out = out.replace(&format!("{{{{{key}}}}}", key = key), &with);
            }
            Value::String(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, values)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), fill(v, values)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Replaces literal string values with placeholders (`{literal: param}`).
fn parameterize(value: &Value, swaps: &Map<String, Value>) -> Value {
    match value {
        Value::String(text) => swaps
            .get(text)
            .and_then(Value::as_str)
            .map_or_else(|| value.clone(), |param| json!(format!("{{{{{param}}}}}"))),
        Value::Array(items) => Value::Array(items.iter().map(|v| parameterize(v, swaps)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), parameterize(v, swaps)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn problems(skill: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let steps = skill.get("steps").cloned().unwrap_or(json!([]));
    if steps.as_array().is_none_or(Vec::is_empty) {
        out.push("the skill has no steps".to_string());
    } else if let Err(fail) = tasks::validate(&steps) {
        out.push(fail.message);
    }
    let mut used = Vec::new();
    placeholders(&steps, &mut used);
    let declared: Vec<String> = skill["params"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for name in &used {
        if !declared.contains(name) {
            out.push(format!(
                "placeholder {{{{{name}}}}} is not declared in params"
            ));
        }
    }
    for name in &declared {
        if !used.contains(name) {
            out.push(format!("param {name} is declared but never used"));
        }
    }
    out
}

/// A clean skill record from what a caller gave.
fn shape(given: &Value, name: &str) -> Value {
    let steps: Vec<Value> = given["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|s| json!({"method": s["method"], "params": s.get("params").cloned().unwrap_or(json!({}))}))
        .collect();
    json!({
        "name": name,
        "version": given["version"].as_str().unwrap_or("0.1.0"),
        "description": given["description"].as_str().unwrap_or(""),
        "domain": given["domain"],
        "params": given.get("params").filter(|p| p.is_object()).cloned().unwrap_or(json!({})),
        "steps": steps,
        "expect": given["expect"],
        "source": given["source"].as_str().unwrap_or("local"),
        "enabled": given["enabled"] != false,
    })
}

fn bump(version: &str) -> String {
    let mut parts: Vec<u64> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(3, 0);
    parts[2] += 1;
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

fn fnv(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64)
                .wrapping_mul(0x100000001b3))
    )
}

/// What the person sees in the skill browser and workflow editor.
pub fn list() -> Value {
    let mut skills = Vec::new();
    for entry in std::fs::read_dir(root()).into_iter().flatten().flatten() {
        if let Ok(skill) = load(&entry.file_name().to_string_lossy()) {
            skills.push(json!({
                "name": skill["name"], "version": skill["version"], "description": skill["description"],
                "domain": skill["domain"], "enabled": skill["enabled"], "steps": skill["steps"].as_array().map_or(0, Vec::len),
                "params": skill["params"], "source": skill["source"],
            }));
        }
    }
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(recorder::dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        let disabled = path.extension().is_some_and(|e| e == "disabled");
        if !disabled && path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let text = std::fs::read(&path).unwrap_or_default();
        workflows.push(json!({
            "name": name,
            "steps": String::from_utf8_lossy(&text).lines().count(),
            "disabled": disabled,
            "approved": approved(&name, &text),
        }));
    }
    json!({"skills": skills, "workflows": workflows})
}

fn approval_file(name: &str) -> Result<PathBuf, Fail> {
    Ok(recorder::file_of(name)?.with_extension("approved"))
}

fn approved(name: &str, content: &[u8]) -> bool {
    approval_file(name)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|hash| hash.trim() == fnv(content))
}

fn recording(name: &str) -> Result<Vec<Value>, Fail> {
    let text = std::fs::read_to_string(recorder::file_of(name)?)
        .map_err(|_| Fail::params(format!("there is no recording named {name}")))?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// `button "Add to cart" required` -> role button, name Add to cart.
fn locator(line: &str) -> Option<Value> {
    let role = line
        .split_whitespace()
        .next()?
        .split('(')
        .next()?
        .to_string();
    let start = line.find('"')?;
    let end = line[start + 1..].find('"')?;
    Some(json!({"by": "role", "role": role, "name": &line[start + 1..start + 1 + end]}))
}

async fn call(caller: &Caller, method: &str, params: Value) -> Reply {
    Box::pin(dispatch::run(caller, method, params)).await
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "skill.install_local"
            | "skill.remove_local"
            | "skill.enable"
            | "skill.disable"
            | "skill.update"
            | "skill.validate"
            | "skill.example"
            | "skill.parameterize"
            | "skill.permission_requirements"
            | "skill.test"
            | "skill.compile"
            | "skill.domain_adapter"
            | "workflow.diff"
            | "workflow.verify"
            | "workflow.approve"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "skill.install_local" => {
            let given = match params["path"].as_str() {
                Some(folder) => {
                    let path = PathBuf::from(folder).join("skill.json");
                    let bytes = std::fs::read(&path)
                        .map_err(|_| Fail::params(format!("{} not found", path.display())))?;
                    serde_json::from_slice::<Value>(&bytes)
                        .map_err(|e| Fail::params(format!("skill.json: {e}")))?
                }
                None => params.clone(),
            };
            let name = given["name"]
                .as_str()
                .ok_or_else(|| Fail::params("the skill needs a name"))?
                .to_string();
            checked(&name)?;
            let skill = shape(&given, &name);
            let found = problems(&skill);
            if !found.is_empty() {
                return Err(
                    Fail::params(format!("the skill is not valid: {}", found.join("; ")))
                        .with(json!({"problems": found})),
                );
            }
            if let Some(folder) = params["path"].as_str() {
                // Notes and examples next to skill.json come along.
                for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                    let target = root().join(&name).join(entry.file_name());
                    if entry.path().is_file() && entry.file_name() != "skill.json" {
                        let _ = std::fs::create_dir_all(root().join(&name));
                        let _ = std::fs::copy(entry.path(), target);
                    }
                }
            }
            store(&skill)?;
            Ok(
                json!({"installed": name, "version": skill["version"], "steps": skill["steps"].as_array().map_or(0, Vec::len)}),
            )
        }
        "skill.remove_local" => {
            let name = name_of(params)?;
            load(&name)?;
            std::fs::remove_dir_all(root().join(checked(&name)?))
                .map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"removed": name}))
        }
        "skill.enable" | "skill.disable" => {
            let mut skill = load(&name_of(params)?)?;
            skill["enabled"] = json!(method == "skill.enable");
            store(&skill)?;
            Ok(json!({"name": skill["name"], "enabled": skill["enabled"]}))
        }
        "skill.update" => {
            let name = name_of(params)?;
            let old = load(&name)?;
            let mut merged = old.clone();
            for key in [
                "description",
                "domain",
                "params",
                "steps",
                "expect",
                "enabled",
            ] {
                if let Some(value) = params.get(key) {
                    merged[key] = value.clone();
                }
            }
            merged["version"] = json!(params["version"].as_str().map_or_else(
                || bump(old["version"].as_str().unwrap_or("0.1.0")),
                str::to_string
            ));
            let skill = shape(&merged, &name);
            let found = problems(&skill);
            if !found.is_empty() {
                return Err(Fail::params(format!(
                    "the update is not valid: {}",
                    found.join("; ")
                )));
            }
            store(&skill)?;
            Ok(json!({"name": name, "version": skill["version"]}))
        }
        "skill.validate" => {
            let skill = match params.get("skill") {
                Some(given) => shape(given, given["name"].as_str().unwrap_or("draft")),
                None => load(&name_of(params)?)?,
            };
            let found = problems(&skill);
            Ok(json!({"name": skill["name"], "valid": found.is_empty(), "problems": found}))
        }
        "skill.example" => {
            let skill = load(&name_of(params)?)?;
            let examples = skill["params"].as_object().cloned().unwrap_or_default();
            Ok(json!({
                "call": {"method": "skill.test", "params": {"name": skill["name"], "params": examples}},
                "steps": fill(&skill["steps"], &examples),
                "description": skill["description"],
            }))
        }
        "skill.parameterize" => {
            let mut skill = load(&name_of(params)?)?;
            let swaps = params["values"]
                .as_object()
                .cloned()
                .ok_or_else(|| Fail::params("values: {\"literal text\": \"paramName\"}"))?;
            skill["steps"] = parameterize(&skill["steps"], &swaps);
            for (literal, param) in &swaps {
                if let Some(param) = param.as_str() {
                    skill["params"][param] = json!(literal);
                }
            }
            skill["version"] = json!(bump(skill["version"].as_str().unwrap_or("0.1.0")));
            store(&skill)?;
            Ok(json!({"name": skill["name"], "params": skill["params"], "steps": skill["steps"]}))
        }
        "skill.permission_requirements" => {
            let skill = load(&name_of(params)?)?;
            let steps = tasks::validate(&skill["steps"])?;
            let mut settings = Vec::new();
            let mut approvals = Vec::new();
            for step in &steps {
                let m = step["method"].as_str().unwrap_or("");
                let need = |list: &mut Vec<&str>, item| {
                    if !list.contains(&item) {
                        list.push(item)
                    }
                };
                if m.starts_with("native.") {
                    need(&mut settings, "nativeControl");
                    need(&mut approvals, "each action on another program");
                }
                if m.starts_with("download.") || m == "act.download" {
                    need(&mut settings, "downloadsEnabled");
                }
                if m.starts_with("runtime.") {
                    need(&mut settings, "allowEvaluate");
                }
                if m.starts_with("form.submit") || m == "act.submit" {
                    need(&mut approvals, "final form submissions (approval mode)");
                }
                if m.starts_with("print.to_printer") {
                    need(&mut approvals, "printing");
                }
            }
            Ok(json!({
                "name": skill["name"],
                "acting": steps.iter().filter(|s| s["readOnly"] == false).map(|s| &s["method"]).collect::<Vec<_>>(),
                "risks": steps.iter().map(|s| json!({"method": s["method"], "risk": s["risk"]})).collect::<Vec<_>>(),
                "settings": settings,
                "approvals": approvals,
                "domain": skill["domain"],
                "note": "purchases and payments always ask the person, whatever the skill",
            }))
        }
        "skill.test" => {
            let skill = load(&name_of(params)?)?;
            if skill["enabled"] == false {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    format!("skill {} is disabled", skill["name"]),
                ));
            }
            let mut values = skill["params"].as_object().cloned().unwrap_or_default();
            values.extend(params["params"].as_object().cloned().unwrap_or_default());
            let mut run = json!({"goal": format!("skill {}", skill["name"].as_str().unwrap_or("")), "steps": fill(&skill["steps"], &values)});
            if let Some(tab) = params.get("tabId") {
                run["tabId"] = tab.clone();
            }
            let task = call(caller, "agent.run", run).await?;
            let mut passed = task["status"] == "done";
            let mut verified = Value::Null;
            if passed && skill["expect"].is_object() {
                let mut check = fill(&skill["expect"], &values);
                if let Some(tab) = params.get("tabId") {
                    check["tabId"] = tab.clone();
                }
                verified = call(caller, "recovery.false_completion", check).await?;
                passed = verified["verified"] == true;
            }
            Ok(json!({"name": skill["name"], "passed": passed, "verified": verified, "task": task}))
        }
        "skill.compile" => {
            let workflow = params["workflow"]
                .as_str()
                .ok_or_else(|| Fail::params("workflow (a recording name) is required"))?;
            let name = params["name"].as_str().unwrap_or(workflow).to_string();
            let mut steps = Vec::new();
            for step in recording(workflow)? {
                let method = step["method"].as_str().unwrap_or("");
                if step["ok"] != true
                    || method.starts_with("decision.")
                    || step["params"]["redacted"] == true
                {
                    continue;
                }
                let mut params = step["params"].clone();
                if let Some(map) = params.as_object_mut() {
                    map.remove("idempotencyKey");
                    map.remove("approvalId");
                    if let Some(find) = step["target"].as_str().and_then(locator) {
                        map.remove("ref");
                        map.insert("find".into(), find);
                    }
                }
                steps.push(json!({"method": method, "params": params}));
            }
            let skill = shape(
                &json!({"steps": steps, "description": params["description"].as_str().unwrap_or(&format!("compiled from recording {workflow}")), "source": format!("workflow:{workflow}"), "domain": params["domain"]}),
                &name,
            );
            let found = problems(&skill);
            if !found.is_empty() {
                return Err(Fail::params(format!(
                    "the recording does not make a valid skill: {}",
                    found.join("; ")
                )));
            }
            store(&skill)?;
            Ok(json!({"name": name, "steps": skill["steps"], "from": workflow}))
        }
        "skill.domain_adapter" => {
            let domain = params["domain"]
                .as_str()
                .ok_or_else(|| Fail::params("domain is required"))?
                .to_lowercase();
            if let Some(name) = params["name"].as_str() {
                let mut skill = load(name)?;
                skill["domain"] = json!(domain);
                store(&skill)?;
                return Ok(json!({"name": name, "domain": domain}));
            }
            let skills: Vec<Value> = list()["skills"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| {
                    s["domain"]
                        .as_str()
                        .is_some_and(|d| domain == d || domain.ends_with(&format!(".{d}")))
                })
                .collect();
            Ok(json!({"domain": domain, "skills": skills}))
        }
        "workflow.diff" => {
            let side = |key: &str| -> Result<Vec<Value>, Fail> {
                let name = params[key]
                    .as_str()
                    .ok_or_else(|| Fail::params("a and b (recording names) are required"))?;
                Ok(recording(name)?
                    .into_iter()
                    .filter(|s| s["ok"] == true)
                    .map(|s| json!({"method": s["method"], "target": s["target"]}))
                    .collect())
            };
            let (a, b) = (side("a")?, side("b")?);
            let mut changes = Vec::new();
            for index in 0..a.len().max(b.len()) {
                if a.get(index) != b.get(index) {
                    changes.push(json!({"index": index, "a": a.get(index), "b": b.get(index)}));
                }
            }
            Ok(
                json!({"same": changes.is_empty(), "changes": changes, "stepsA": a.len(), "stepsB": b.len()}),
            )
        }
        "workflow.verify" => {
            let name = name_of(params)?;
            let steps = recording(&name)?;
            let content = std::fs::read(recorder::file_of(&name)?).unwrap_or_default();
            let outline = call(
                caller,
                "observe.page",
                json!({"tabId": params.get("tabId"), "max": 20000}),
            )
            .await
            .ok();
            let outline = outline
                .as_ref()
                .and_then(|o| o["page"].as_str())
                .unwrap_or("");
            let checks: Vec<Value> = steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let method = step["method"].as_str().unwrap_or("");
                    let works = tasks::validate(&json!([{"method": method}])).is_ok();
                    let target = step["target"].as_str();
                    json!({
                        "index": index, "method": method, "works": works,
                        "target": target,
                        "onPage": target.map(|line| recorder::find_ref(outline, line).is_some()),
                    })
                })
                .collect();
            let broken = checks.iter().filter(|c| c["works"] == false).count();
            Ok(
                json!({"name": name, "steps": checks, "broken": broken, "approved": approved(&name, &content)}),
            )
        }
        "workflow.approve" => {
            if !caller.is_shell() {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "approving a workflow is the person's decision",
                ));
            }
            let name = name_of(params)?;
            let content = std::fs::read(recorder::file_of(&name)?)
                .map_err(|_| Fail::params(format!("there is no recording named {name}")))?;
            std::fs::write(approval_file(&name)?, fnv(&content))
                .map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"name": name, "approved": true}))
        }
        _ => Err(Fail::internal(format!("{method} is not a skill command"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_fill_and_parameterize() {
        let steps = json!([{"method": "act.fill", "params": {"text": "Ada", "n": 1}}]);
        let swaps = json!({"Ada": "name"});
        let made = parameterize(&steps, swaps.as_object().unwrap());
        assert_eq!(made[0]["params"]["text"], "{{name}}");
        let mut used = Vec::new();
        placeholders(&made, &mut used);
        assert_eq!(used, vec!["name"]);
        let values = json!({"name": "Grace"});
        assert_eq!(
            fill(&made, values.as_object().unwrap())[0]["params"]["text"],
            "Grace"
        );
        let typed = json!({"count": 3});
        assert_eq!(
            fill(&json!("{{count}}"), typed.as_object().unwrap()),
            json!(3)
        );
        assert_eq!(
            fill(&json!("n={{count}}"), typed.as_object().unwrap()),
            json!("n=3")
        );
    }

    #[test]
    fn locator_from_outline_line() {
        assert_eq!(
            locator("button \"Count clicks\"").unwrap()["name"],
            "Count clicks"
        );
        assert_eq!(
            locator("textbox(email) \"Email\" required").unwrap()["role"],
            "textbox"
        );
        assert_eq!(bump("0.1.9"), "0.1.10");
    }
}
