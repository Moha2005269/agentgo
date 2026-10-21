//! MCP server (stdio, JSON-RPC 2.0) over the AgentGo protocol.
//!
//! The tool set is deliberately small and its results are compact text, so
//! an agent spends few tokens on tool definitions and on every observation.
//! Each tool maps onto ordinary protocol commands: the adapter adds no
//! privilege and removes no policy gate. `browser_call` reaches every other
//! command.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::client::{Client, ClientError};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// The tool that hands a subgoal to GoMini, listed only while it is active.
pub fn delegate_tool() -> Value {
    json!({
        "name": "browser_delegate",
        "description": "GoMini, the local model, is on: hand it a routine run of steps and it takes them itself (fill the values you supply, click, check, select, scroll, wait, retry) without you reading every page: opening tabs, folders or sections until something shows, a set of boxes, a long form. Not worth it for one click. Give subgoal (one goal, named as the page names things), values (every text to type, by name), completion (how to tell it is done), constraints. Returns status done (GoMini's claim: glance at evidence), needs_help (read the page and give a narrower subgoal, or do the step yourself) or stopped. Payments, sign-ins, final submissions and deletes always come back to you.",
        "inputSchema": {"type": "object", "properties": {
            "subgoal": {"type": "string"},
            "values": {"type": "object", "additionalProperties": {"type": "string"}},
            "completion": {"type": "string"},
            "constraints": {"type": "array", "items": {"type": "string"}},
            "maxSteps": {"type": "integer", "description": "1-12, default 8"},
            "tabId": {"type": "string"}
        }, "required": ["subgoal"]},
    })
}

/// Tool definitions advertised to the MCP host.
pub fn tools() -> Value {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required});
    json!([
        {
            "name": "browser_open",
            "description": "Go to a URL (or search Google with plain words), wait for the load, and return the page as a compact outline: headings plus interactive elements as `[ref] role \"name\"`. Use the refs with browser_act.",
            "inputSchema": object(json!({
                "url": {"type": "string", "description": "URL, or search words"},
                "view": {"type": "string", "enum": ["page", "text", "none"], "description": "page = outline with refs (default); text = readable text"}
            }), &["url"]),
        },
        {
            "name": "browser_read",
            "description": "Read the current page. view: page (outline with refs, default), text (readable content, text boxes and code included), links, forms, tables, meta. filter keeps matching lines; offset and max count lines.",
            "inputSchema": object(json!({
                "view": {"type": "string", "enum": ["page", "text", "links", "forms", "tables", "meta", "changes"]},
                "filter": {"type": "string"},
                "offset": {"type": "integer"},
                "max": {"type": "integer"}
            }), &[]),
        },
        {
            "name": "browser_act",
            "description": "Act on an element by ref with real input, wait for the page to settle, and get back only what changed. action: click, fill (text), press (key), check, uncheck, select (value, or values for a multiple list), hover, scroll (to: top|bottom, or ref; until: a text to scroll to, in the page or ref's list), type (text at focus), upload (paths). Add submit:true to fill to press Enter; expect {gone, shows, enabled, url, changes} must hold after, or it fails. If the reply is approval.required or handoff.required, call browser_wait_human.",
            "inputSchema": object(json!({
                "action": {"type": "string", "enum": ["click", "fill", "press", "check", "uncheck", "select", "hover", "scroll", "type", "upload", "double_click", "right_click"]},
                "ref": {"type": "integer"},
                "text": {"type": "string"},
                "key": {"type": "string", "description": "e.g. Enter, Tab, Escape, ArrowDown, a"},
                "value": {"type": "string"},
                "values": {"type": "array", "items": {"type": "string"}, "description": "select: several options of a multiple list"},
                "to": {"type": "string"},
                "until": {"type": "string"},
                "paths": {"type": "array", "items": {"type": "string"}},
                "submit": {"type": "boolean"},
                "expect": {"type": "object"},
                "approvalId": {"type": "string"}
            }), &["action"]),
        },
        {
            "name": "browser_batch",
            "description": "Run several browser_act steps in order in one call (e.g. fill a whole form and click Save; {action:\"wait\", ms} pauses). Stops at the first failure. Returns the final change set.",
            "inputSchema": object(json!({
                "steps": {"type": "array", "items": {"type": "object"}, "description": "Each item has the same fields as browser_act"}
            }), &["steps"]),
        },
        {
            "name": "browser_wait",
            "description": "Wait until text appears, a selector is visible, a selector or text is gone, the URL contains a string, or ref `enabled` is usable; returns what changed. ms: just pause. No condition: waits for load.",
            "inputSchema": object(json!({
                "text": {"type": "string"}, "selector": {"type": "string"}, "gone": {"type": "string"},
                "urlContains": {"type": "string"}, "enabled": {"type": "integer"}, "ms": {"type": "integer"},
                "timeout": {"type": "integer", "description": "ms"}
            }), &[]),
        },
        {
            "name": "browser_tabs",
            "description": "Manage your tabs. op: list, new (url), switch (tabId), close (tabId), back, forward, reload.",
            "inputSchema": object(json!({
                "op": {"type": "string", "enum": ["list", "new", "switch", "close", "back", "forward", "reload"]},
                "tabId": {"type": "string"}, "url": {"type": "string"}
            }), &["op"]),
        },
        {
            "name": "browser_screenshot",
            "description": "Screenshot the page (or one element by ref) and return the image. Costs many tokens: prefer browser_read unless you need to see layout, images or a canvas.",
            "inputSchema": object(json!({"ref": {"type": "integer"}, "fullPage": {"type": "boolean"}}), &[]),
        },
        {
            "name": "browser_wait_human",
            "description": "Wait for the human operator: pass approvalId after approval.required, or nothing after handoff.required (CAPTCHA, login). Never try to solve a CAPTCHA yourself.",
            "inputSchema": object(json!({"approvalId": {"type": "string"}, "timeout": {"type": "integer"}}), &[]),
        },
        {
            "name": "browser_guide",
            "description": "How to use this browser, when you are unsure. topic=start: the loop, rules that save turns, errors (short). topic=skill: the full guide. topic=commands: find commands: pass query (words, e.g. \"selector click\") for the matching ones, or group for one section (open, read, act, batch, accounts, person, browser; all = every command, about 25,000 tokens). topic=gomini: GoMini, the optional local model that takes routine steps, and how to phrase subgoals for it.",
            "inputSchema": object(json!({
                "topic": {"type": "string", "enum": ["start", "commands", "gomini"]},
                "group": {"type": "string", "enum": ["open", "read", "act", "batch", "accounts", "person", "browser", "all"]},
                "query": {"type": "string"}
            }), &[]),
        },
        {
            "name": "browser_account",
            "description": "Use the accounts the person linked for you in the Account Center. op=accounts lists them (linked or not); op=apis lists the APIs you can call; op=call sends one API request as the account (connector, method, path, body) without you ever holding a token; op=sign_in has the browser type the saved sign-in into the current page's login form. If an account is not linked you get policy.denied: ask the person to link it, do not work around it.",
            "inputSchema": object(json!({
                "op": {"type": "string", "enum": ["accounts", "apis", "call", "sign_in"]},
                "account": {"type": "string"}, "connector": {"type": "string"},
                "method": {"type": "string"}, "path": {"type": "string"}, "body": {},
                "tabId": {"type": "string"}
            }), &["op"]),
        },
        {
            "name": "browser_call",
            "description": "Call any other AgentGo protocol method directly (cookies, downloads, PDF, dialogs, network log, workflow.collect ...). `browser_call {method:\"browser.capabilities\"}` lists them.",
            "inputSchema": object(json!({"method": {"type": "string"}, "params": {"type": "object"}}), &["method"]),
        }
    ])
}

/// The guide is the bundled skill itself: one text for every kind of host.
const GUIDE_START: &str = include_str!("../../../skills/agentgo/SKILL.md");
const GUIDE_COMMANDS: &str = include_str!("../../../skills/agentgo/references/commands.md");
const GUIDE_GOMINI: &str = include_str!("../../../skills/agentgo/references/gomini.md");
/// Over MCP the start is written in tool names (`browser_act`), not protocol
/// names: the shell guide sent agents to `browser_call` for every step.
const GUIDE_MCP: &str = include_str!("../../../skills/agentgo/references/mcp.md");

/// The text `browser_guide` answers with.
pub fn guide(
    topic: Option<&str>,
    group: Option<&str>,
    query: Option<&str>,
) -> Result<String, String> {
    // A query alone is a search of the commands.
    let query = query.filter(|q| !q.trim().is_empty());
    let topic = topic.unwrap_or(if query.is_some() { "commands" } else { "start" });
    match topic {
        "start" => Ok(GUIDE_MCP.to_string()),
        "skill" | "shell" => Ok(GUIDE_START.to_string()),
        "gomini" | "laya" => Ok(GUIDE_GOMINI.to_string()),
        "commands" => {
            if let Some(query) = query {
                return Ok(command_search(query));
            }
            let Some(group) = group else {
                return Ok(command_index());
            };
            if group == "all" {
                return Ok(GUIDE_COMMANDS.to_string());
            }
            let heading = match group {
                "open" => "## Open and move",
                "read" => "## Read the page",
                "act" => "## Act on the page",
                "batch" => "## Do several things in one call",
                "accounts" => "## Accounts, sign-ins and APIs",
                "person" => "## The person: approvals, handoff, control",
                "browser" => "## Browser, profile and files",
                other => return Err(format!("unknown group {other}")),
            };
            let from = GUIDE_COMMANDS
                .find(heading)
                .ok_or("that section is empty")?;
            let rest = &GUIDE_COMMANDS[from..];
            let to = rest[heading.len()..]
                .find("\n## ")
                .map_or(rest.len(), |at| at + heading.len());
            Ok(rest[..to].trim_end().to_string())
        }
        other => Err(format!(
            "unknown topic {other}; use start, commands, gomini or skill"
        )),
    }
}

/// The commands list is about 25,000 tokens: by default an agent gets its
/// sections and how to find one command, not the whole list.
fn command_index() -> String {
    let intro = GUIDE_COMMANDS
        .split(
            "
## ",
        )
        .next()
        .unwrap_or_default()
        .trim_end();
    let mut out = format!(
        "{intro}

Sections (pass group to read one, or query to find commands by words):
"
    );
    for section in GUIDE_COMMANDS
        .split(
            "
## ",
        )
        .skip(1)
    {
        let title = section.lines().next().unwrap_or_default();
        let count = section.lines().filter(|l| l.starts_with("| `")).count();
        out.push_str(&format!(
            "- {title} ({count} commands)
"
        ));
    }
    out.push_str("
Example: browser_guide {topic: \"commands\", query: \"selector\"}. group: \"all\" returns every command.");
    out
}

/// The command rows that best match `query`: rows with more of its words
/// first, and among them a word in the command's name or a whole word in its
/// description before one inside a longer word ("select" in "selector").
fn command_search(query: &str) -> String {
    let words: Vec<String> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut rows: Vec<(usize, usize, &str)> = GUIDE_COMMANDS
        .lines()
        .filter(|l| l.starts_with("| `"))
        .filter_map(|l| {
            let lower = l.to_lowercase();
            let name = lower.split('`').nth(1).unwrap_or("");
            let text: Vec<&str> = lower
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .collect();
            let (mut hits, mut score) = (0, 0);
            for word in &words {
                let stem = word.trim_end_matches('s');
                let points = if name
                    .split(['.', '_'])
                    .any(|part| part == word || part == stem)
                {
                    4
                } else if name.contains(word.as_str()) {
                    3
                } else if text
                    .iter()
                    .any(|t| *t == word || *t == stem || t.trim_end_matches('s') == stem)
                {
                    2
                } else if lower.contains(word.as_str()) {
                    1
                } else {
                    0
                };
                if points > 0 {
                    hits += 1;
                    score += points;
                }
            }
            (hits > 0).then_some((hits, score, l))
        })
        .collect();
    if rows.is_empty() {
        return format!(
            "No command matches {query:?}. Try other words, or pass group to read a section."
        );
    }
    rows.sort_by_key(|a| std::cmp::Reverse((a.0, a.1)));
    let shown = rows.len().min(12);
    let mut out = String::from("| Command | What it does |\n|---|---|\n");
    out.push_str(
        &rows[..shown]
            .iter()
            .map(|(_, _, l)| *l)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if rows.len() > shown {
        out.push_str(&format!(
            "\n({} weaker matches not shown; add words to narrow)",
            rows.len() - shown
        ));
    }
    out
}

fn act_method(action: &str) -> Option<&'static str> {
    Some(match action {
        "click" => "act.click",
        "double_click" => "act.double_click",
        "right_click" => "act.right_click",
        "fill" => "act.fill",
        "press" => "act.press",
        "check" => "act.check",
        "uncheck" => "act.uncheck",
        "select" => "act.select_option",
        "hover" => "act.hover",
        "scroll" => "act.scroll",
        "type" => "act.type",
        "upload" => "act.upload",
        // A timed pause between steps of a batch.
        "wait" => "wait.load",
        _ => return None,
    })
}

/// Maps a tool call onto one protocol command.
pub fn command_for(tool: &str, args: &Value) -> Result<(String, Value), String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    Ok(match tool {
        "browser_open" => (
            "navigate.url".into(),
            json!({"url": args["url"], "observe": text("view").unwrap_or("page")}),
        ),
        "browser_read" => {
            let method = match text("view").unwrap_or("page") {
                "page" => "observe.page",
                "text" => "observe.text",
                "links" => "observe.links",
                "forms" => "observe.forms",
                "tables" => "observe.tables",
                "meta" => "observe.page_metadata",
                "changes" => "state.diff",
                other => return Err(format!("unknown view {other}")),
            };
            (method.into(), args.clone())
        }
        "browser_act" => {
            let action = text("action").ok_or("browser_act needs action")?;
            (
                act_method(action)
                    .ok_or_else(|| format!("unknown action {action}"))?
                    .into(),
                args.clone(),
            )
        }
        "browser_batch" => {
            let steps = args["steps"]
                .as_array()
                .ok_or("browser_batch needs steps")?;
            let mapped = steps
                .iter()
                .map(|step| {
                    let action = step["action"].as_str().ok_or("each step needs action")?;
                    let method =
                        act_method(action).ok_or_else(|| format!("unknown action {action}"))?;
                    Ok(json!({"method": method, "params": step}))
                })
                .collect::<Result<Vec<_>, String>>()?;
            ("workflow.batch".into(), json!({"steps": mapped}))
        }
        "browser_wait" => {
            let conditional = ["text", "selector", "gone", "urlContains", "enabled", "ms"]
                .iter()
                .any(|key| args.get(*key).is_some());
            (
                if conditional {
                    "wait.condition"
                } else {
                    "wait.load"
                }
                .into(),
                args.clone(),
            )
        }
        "browser_tabs" => {
            let method = match text("op").unwrap_or("list") {
                "list" => "tabs.list",
                "new" => "tabs.create",
                "switch" => "tabs.activate",
                "close" => "tabs.close",
                "back" => "tab.back",
                "forward" => "tab.forward",
                "reload" => "tab.reload",
                other => return Err(format!("unknown op {other}")),
            };
            (method.into(), args.clone())
        }
        "browser_screenshot" => (
            "observe.screenshot".into(),
            json!({"ref": args.get("ref"), "fullPage": args.get("fullPage"), "inline": true}),
        ),
        "browser_wait_human" => match text("approvalId") {
            Some(_) => ("approval.wait".into(), args.clone()),
            None => ("handoff.wait".into(), args.clone()),
        },
        "browser_account" => match text("op") {
            Some("accounts") => ("accounts.list".into(), json!({})),
            Some("apis") => ("connectors.list".into(), json!({})),
            Some("call") => ("connectors.call".into(), args.clone()),
            Some("sign_in") => (
                "credentials.fill".into(),
                json!({"tabId": args.get("tabId")}),
            ),
            _ => return Err("browser_account needs op: accounts, apis, call or sign_in".into()),
        },
        "browser_delegate" => ("gomini.delegate".into(), args.clone()),
        "browser_call" => (
            text("method")
                .ok_or("browser_call needs method")?
                .to_string(),
            args.get("params").cloned().unwrap_or_else(|| json!({})),
        ),
        other => return Err(format!("unknown tool {other}")),
    })
}

/// Renders a protocol result as compact text for the model: page views and
/// change sets as plain lines, everything else as minified JSON.
pub fn render(result: &Value) -> String {
    let Some(map) = result.as_object() else {
        return result.to_string();
    };
    let mut out = Vec::new();
    let mut rest = map.clone();
    // A page view needs no tab id; any other reply keeps it (tabs.create's
    // new tab is the answer itself: AG-L02).
    let view = ["page", "text", "changes"]
        .iter()
        .any(|key| map.contains_key(*key));
    if !view {
        if let Some(tab) = rest.remove("tabId").filter(|value| !value.is_null()) {
            out.push(format!(
                "tabId: {}",
                tab.as_str().map_or_else(|| tab.to_string(), str::to_string)
            ));
        }
    }
    let mut take = |key: &str| rest.remove(key).filter(|value| !value.is_null());
    if let (Some(url), title) = (take("url"), take("title")) {
        let title = title
            .and_then(|t| t.as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(
            format!("{title} — {}", url.as_str().unwrap_or(""))
                .trim_start_matches(" — ")
                .to_string(),
        );
    }
    for key in ["page", "text", "changes", "filled"] {
        if let Some(Value::String(body)) = take(key) {
            if key == "changes" {
                out.push(if body.is_empty() {
                    "(no visible change)".into()
                } else {
                    format!("changed:\n{body}")
                });
            } else {
                out.push(body);
            }
        }
    }
    for key in [
        "tabId", "scroll", "total", "ms", "accepted", "loaded", "base64",
    ] {
        let dropped = take(key);
        if key == "loaded" && dropped == Some(Value::Bool(false)) {
            out.push("(the page had not finished loading)".into());
        }
    }
    if let Some(next) = take("next") {
        out.push(format!("(more: pass offset {next})"));
    }
    if !rest.is_empty() {
        out.push(Value::Object(rest).to_string());
    }
    out.join("\n")
}

fn error_text(error: &ClientError) -> String {
    match error {
        ClientError::Protocol {
            code,
            message,
            data,
        } if !data.is_null() => format!("{code}: {message} {data}"),
        other => other.to_string(),
    }
}

fn call_tool(client: &mut Client, name: &str, args: &Value) -> Value {
    if name == "browser_guide" {
        let text = guide(
            args["topic"].as_str(),
            args["group"].as_str(),
            args["query"].as_str(),
        );
        return json!({"content": [{"type": "text", "text": text.clone().unwrap_or_else(|error| error)}], "isError": text.is_err()});
    }
    let (method, params) = match command_for(name, args) {
        Ok(command) => command,
        Err(message) => {
            return json!({"content": [{"type": "text", "text": message}], "isError": true})
        }
    };
    match client.call(&method, params) {
        Ok(result) => {
            let mut content = Vec::new();
            if let Some(data) = result.get("base64").and_then(Value::as_str) {
                let mime = if result["format"] == "png" {
                    "image/png"
                } else {
                    "image/jpeg"
                };
                content.push(json!({"type": "image", "data": data, "mimeType": mime}));
            }
            content.push(json!({"type": "text", "text": render(&result)}));
            json!({"content": content})
        }
        Err(error) => {
            json!({"content": [{"type": "text", "text": error_text(&error)}], "isError": true})
        }
    }
}

/// Whether the person has GoMini on and downloaded (then agents get
/// `browser_delegate`).
fn gomini_active(client: &mut Client) -> bool {
    client
        .call("decision.status", json!({}))
        .is_ok_and(|status| status["active"] == true)
}

const INSTRUCTIONS: &str = "AgentGo is a real browser. browser_open returns a compact outline ([n] references); act with browser_act, or browser_batch for several steps you are sure of. Replies contain only what changed: read the page again only when you need it. Repeated buttons show their row (in \"...\"): pick by the row. A long or endless list: browser_act action scroll with until (a text) scrolls until it shows, in one call. Something that happens later: browser_wait (text, gone, or enabled: a ref) returns what changed. On approval.required or handoff.required call browser_wait_human; never attempt CAPTCHAs. browser_guide has the details when you need them.";
const INSTRUCTIONS_GOMINI: &str = " GoMini, a small model on this computer, is on: browser_delegate takes a routine run of steps (opening tabs, folders or sections until something shows, ticking a set of boxes, filling a long form with values you supply, retrying a step the page rejected) so you need not read and act through each one. A single click is faster done yourself.";

/// Serves MCP on the given streams until the host disconnects.
pub fn serve(mut client: Client, input: impl BufRead, mut output: impl Write) -> Client {
    for line in input.lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        let result = match message["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agentgo", "version": env!("CARGO_PKG_VERSION")},
                "instructions": if gomini_active(&mut client) {
                    format!("{INSTRUCTIONS}{INSTRUCTIONS_GOMINI}")
                } else {
                    INSTRUCTIONS.to_string()
                },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut list = tools();
                if gomini_active(&mut client) {
                    if let Some(items) = list.as_array_mut() {
                        items.insert(items.len() - 1, delegate_tool());
                    }
                }
                Ok(json!({"tools": list}))
            }
            "tools/call" => Ok(call_tool(
                &mut client,
                message["params"]["name"].as_str().unwrap_or(""),
                &message["params"]["arguments"],
            )),
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            method if id.is_none() || method.starts_with("notifications/") => continue,
            method => {
                Err(json!({"code": -32601, "message": format!("method not found: {method}")}))
            }
        };
        let Some(id) = id else {
            continue;
        };
        let reply = match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        };
        if writeln!(output, "{reply}")
            .and_then(|_| output.flush())
            .is_err()
        {
            break;
        }
    }
    client
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_maps_to_a_registered_command() {
        let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
        let cases = [
            ("browser_open", json!({"url": "example.com"})),
            ("browser_read", json!({})),
            ("browser_read", json!({"view": "changes"})),
            (
                "browser_act",
                json!({"action": "select", "ref": 3, "value": "x"}),
            ),
            (
                "browser_batch",
                json!({"steps": [{"action": "fill", "ref": 1, "text": "a"}]}),
            ),
            ("browser_wait", json!({})),
            ("browser_wait", json!({"text": "Done"})),
            ("browser_tabs", json!({"op": "new"})),
            ("browser_screenshot", json!({})),
            ("browser_wait_human", json!({})),
            ("browser_wait_human", json!({"approvalId": "ap-1"})),
            ("browser_account", json!({"op": "accounts"})),
            ("browser_account", json!({"op": "apis"})),
            (
                "browser_account",
                json!({"op": "call", "account": "github", "connector": "github", "path": "/user"}),
            ),
            ("browser_account", json!({"op": "sign_in"})),
        ];
        for (tool, args) in cases {
            let (method, _) = command_for(tool, &args).unwrap();
            let entry = registry
                .lookup(&method)
                .unwrap_or_else(|| panic!("{tool} maps to unregistered {method}"));
            assert_eq!(
                entry.status,
                agentgo_shell_bridge::MethodStatus::Implemented,
                "{method}"
            );
        }
        assert!(command_for("browser_act", &json!({"action": "explode"})).is_err());
        assert!(command_for("nope", &json!({})).is_err());
        // Every advertised tool is handled.
        for tool in tools().as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            let args = match name {
                "browser_open" => json!({"url": "x"}),
                "browser_act" => json!({"action": "click"}),
                "browser_batch" => json!({"steps": []}),
                "browser_tabs" => json!({"op": "list"}),
                "browser_call" => json!({"method": "tabs.list"}),
                "browser_account" => json!({"op": "accounts"}),
                "browser_guide" => continue,
                _ => json!({}),
            };
            assert!(command_for(name, &args).is_ok(), "{name}");
        }
    }

    #[test]
    fn results_render_as_compact_text() {
        let page = json!({"tabId": "content-1", "url": "https://a.test/", "title": "A", "page": "# A\n[1] link \"Home\"", "total": 2, "scroll": {"y": 0}, "loaded": true, "ms": 40});
        assert_eq!(render(&page), "A — https://a.test/\n# A\n[1] link \"Home\"");
        let delta = json!({"changes": "~ [2] textbox \"Name\" value=\"Ada\"", "field": "x", "accepted": true});
        assert_eq!(
            render(&delta),
            "changed:\n~ [2] textbox \"Name\" value=\"Ada\"\n{\"field\":\"x\"}"
        );
        assert_eq!(
            render(&json!({"changes": "", "target": "button \"OK\""})),
            "(no visible change)\n{\"target\":\"button \\\"OK\\\"\"}"
        );
        assert_eq!(
            render(
                &json!({"text": "Hello", "next": 6000, "total": 9000, "url": "u", "title": "T"})
            ),
            "T — u\nHello\n(more: pass offset 6000)"
        );
        // A new tab's id survives; a page view drops it.
        assert_eq!(
            render(
                &json!({"tabId": "content-4", "url": "https://a.test/", "title": "", "loading": true})
            ),
            "tabId: content-4\nhttps://a.test/\n{\"loading\":true}"
        );
    }

    #[test]
    fn tool_definitions_stay_small() {
        let size = tools().to_string().len();
        assert!(
            size < 6_000,
            "tool definitions are {size} bytes; keep them lean"
        );
    }

    #[test]
    fn the_guide_names_every_command_and_serves_each_topic() {
        let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
        for method in registry.implemented() {
            assert!(
                GUIDE_COMMANDS.contains(&format!("`{}`", method.name)),
                "{} is missing from the command reference; run node scripts/dev/gen-skill.js",
                method.name
            );
        }
        assert!(guide(None, None, None).unwrap().contains("# AgentGo"));
        // The MCP start is short and in tool names: it stays in an agent's
        // context for every later turn.
        let start = guide(None, None, None).unwrap();
        assert!(
            start.len() < 4_000 && start.contains("browser_act") && start.contains("until"),
            "{}",
            start.len()
        );
        assert!(guide(Some("skill"), None, None)
            .unwrap()
            .contains("## The loop"));
        assert!(guide(Some("gomini"), None, None)
            .unwrap()
            .contains("agentgo.gomini.decision/1"));
        // The old topic name still answers.
        assert_eq!(
            guide(Some("laya"), None, None),
            guide(Some("gomini"), None, None)
        );
        // The delegate tool is described compactly too.
        assert!(delegate_tool().to_string().len() < 1_500);
        let act = guide(Some("commands"), Some("act"), None).unwrap();
        assert!(act.contains("| `act.click` |") && !act.contains("| `navigate.url` |"));
        assert!(guide(Some("nope"), None, None).is_err());
        assert!(guide(Some("commands"), Some("nope"), None).is_err());
        let index = guide(Some("commands"), None, None).unwrap();
        assert!(
            index.len() < 4_000 && index.contains("Read the page"),
            "{index}"
        );
        let found = guide(Some("commands"), None, Some("dom ref")).unwrap();
        assert!(
            found.contains("locate.dom_ref") && found.len() < 3_000,
            "{found}"
        );
        assert!(guide(Some("commands"), Some("all"), None).unwrap().len() > 50_000);
        // A query alone searches; the best rows come first, and few of them.
        let select = guide(None, None, Some("select option multiple")).unwrap();
        let first = select.lines().nth(2).unwrap_or_default();
        assert!(
            first.contains("act.select_option") || first.contains("form.multi_select"),
            "{select}"
        );
        assert!(select.lines().filter(|l| l.starts_with("| `")).count() <= 12);
        // Every command the guide's first page tells an agent to call exists.
        for word in
            GUIDE_START.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
        {
            let dotted = word.matches('.').count() == 1
                && !word.starts_with('.')
                && !word.ends_with('.')
                // File names (references/gomini.md) are not commands.
                && !word.ends_with(".md")
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_');
            let domain = word.split('.').next().unwrap_or("");
            if dotted
                && registry
                    .all()
                    .iter()
                    .any(|m| m.name.starts_with(&format!("{domain}.")))
            {
                assert!(
                    registry.lookup(word).is_some()
                        || matches!(word, "dialog.accept" | "dialog.dismiss")
                        || matches!(word.split('.').nth(1), Some("required" | "denied")),
                    "the skill mentions {word}, which is not a command"
                );
            }
        }
    }
}
