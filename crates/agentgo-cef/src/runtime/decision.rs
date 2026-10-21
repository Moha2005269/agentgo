//! The decision interface for GoMini, the local choice model.
//!
//! GoMini is a small model that picks the next browser step so the large
//! external model does not have to read every page (gomini.rs runs it).
//! What it is fed is fixed here so agents, traces and training data all use
//! one shape:
//!
//! - a **subgoal** from the planner, with its constraints and the exact
//!   text values it may type (a chooser never invents text);
//! - the **current state** in compact form, with a revision;
//! - the **candidates**: every step that is possible right now, already
//!   reduced to "operation on reference", plus the ways out (scroll, wait,
//!   back, done, escalate);
//! - the **recent outcomes**, so it does not repeat a step that just failed.
//!
//! The answer is one candidate id (and, for typing, the name of a supplied
//! value). `decision.apply` runs it only if the page is still at the revision
//! the choice was made for. Execution, waiting, policy and verification stay
//! in the browser.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{json, Value};

/// Version of the record. Change it whenever a field changes meaning.
pub const SCHEMA: &str = "agentgo.gomini.decision/1";

/// (session, tab) -> (page revision, candidates offered, values supplied,
/// whether the subgoal clears boxes).
type Offered = HashMap<(String, String), (String, Vec<Value>, Value, bool)>;

thread_local! {
    /// The last record built per (session, tab): what `decision.apply` may run.
    static OFFERED: RefCell<Offered> = RefCell::new(HashMap::new());
}

/// The operation a chooser may apply to an outline line of this role.
fn operation(role: &str) -> &'static str {
    match role.split('(').next().unwrap_or(role) {
        "textbox" | "searchbox" | "textarea" | "spinbutton" => "fill",
        "combobox" | "listbox" => "select",
        "checkbox" | "radio" | "switch" => "check",
        _ => "click",
    }
}

/// Whether a subgoal asks to clear boxes rather than tick them.
pub fn clears(subgoal: &str) -> bool {
    let lower = subgoal.to_lowercase();
    [
        "uncheck",
        "untick",
        "deselect",
        "unselect",
        "clear the",
        "turn off",
        "unmark",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// Turns the outline (`[7] button "Save"` lines) into candidates.
pub fn candidates(outline: &str) -> Vec<Value> {
    let mut out: Vec<Value> = outline
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (reference, label) = rest.split_once("] ")?;
            let reference: u64 = reference.parse().ok()?;
            let role = label.split(' ').next().unwrap_or("");
            // A disabled control is not a possible step.
            if label.contains(" disabled") {
                return None;
            }
            let state = label.split(" in \"").next().unwrap_or(label);
            let mut op = operation(role);
            // A read-only field (a date picker's box) opens its picker when
            // clicked; it takes no typed text.
            if op == "fill" && state.contains(" readonly") {
                op = "click";
            }
            Some(
                json!({"id": format!("r{reference}"), "op": op, "ref": reference, "target": label}),
            )
        })
        .collect();
    for (id, op, meaning) in [
        ("scroll_down", "scroll", "show the next part of the page"),
        ("scroll_up", "scroll", "show the previous part of the page"),
        ("back", "back", "return to the previous page"),
        ("wait", "wait", "the page is still changing; look again"),
        ("done", "done", "the completion condition holds"),
        ("escalate", "escalate", "no candidate fits; ask the planner"),
    ] {
        out.push(json!({"id": id, "op": op, "target": meaning}));
    }
    out
}

/// Builds the record and remembers its candidates for `decision.apply`.
pub fn record(
    session: &str,
    tab: &str,
    revision: String,
    view: &Value,
    recent: Value,
    params: &Value,
) -> Result<Value, String> {
    let subgoal = params["subgoal"]
        .as_str()
        .map(str::trim)
        .filter(|goal| !goal.is_empty())
        .ok_or("decision.record needs params.subgoal: one small step in plain words")?;
    if subgoal.chars().count() > 600 {
        return Err("a subgoal is one small step: at most 600 characters".into());
    }
    let values = match &params["values"] {
        Value::Null => json!({}),
        Value::Object(map) if map.len() <= 20 && map.values().all(Value::is_string) => {
            params["values"].clone()
        }
        _ => return Err("params.values must be an object of up to 20 named text values".into()),
    };
    let outline = view["page"].as_str().unwrap_or("");
    let clearing = clears(subgoal);
    let mut offered = candidates(outline);
    // "Focus the text box" with nothing to type: a box is clicked, not filled.
    let lower = subgoal.to_lowercase();
    if values.as_object().is_some_and(|v| v.is_empty())
        && (lower.contains("focus")
            || lower.contains("click in")
            || lower.contains("click on the text"))
    {
        for candidate in offered.iter_mut().filter(|c| c["op"] == "fill") {
            candidate["op"] = json!("click");
        }
    }
    OFFERED.with(|all| {
        all.borrow_mut().insert(
            (session.to_string(), tab.to_string()),
            (revision.clone(), offered.clone(), values.clone(), clearing),
        )
    });
    Ok(json!({
        "schema": SCHEMA,
        "subgoal": subgoal,
        "constraints": params.get("constraints").cloned().unwrap_or_else(|| json!([])),
        "values": values,
        "completion": params.get("completion"),
        "state": {"tabId": tab, "url": view["url"], "title": view["title"], "revision": revision, "outline": outline, "more": view["next"]},
        "candidates": offered,
        "recent": recent,
        "answer": {"choose": "<candidate id>", "value": "<name of one supplied value, for fill and select>", "confidence": "<0..1, optional>"},
    }))
}

/// The protocol command and parameters for a chosen candidate, if the choice
/// is one this session was offered for this tab at this revision.
pub fn resolve(
    session: &str,
    tab: &str,
    revision: &str,
    params: &Value,
) -> Result<(String, Value), String> {
    let choice = params["choose"]
        .as_str()
        .ok_or("decision.apply needs params.choose: a candidate id from decision.record")?;
    let (offered_at, offered, supplied, clearing) = OFFERED
        .with(|all| {
            all.borrow()
                .get(&(session.to_string(), tab.to_string()))
                .cloned()
        })
        .ok_or("call decision.record first: there are no candidates for this tab")?;
    if offered_at != revision {
        return Err(format!("stale: the page changed since the candidates were built (revision {offered_at} -> {revision}); call decision.record again"));
    }
    let candidate = offered
        .iter()
        .find(|candidate| candidate["id"] == choice)
        .ok_or_else(|| format!("{choice} is not one of the offered candidates"))?;
    let value = || -> Result<Value, String> {
        let name = params["value"]
            .as_str()
            .ok_or("this candidate needs params.value: the name of a supplied value")?;
        supplied.get(name).filter(|text| text.is_string()).cloned().ok_or_else(|| format!("no supplied value is named {name}; a chooser may only use the values it was given"))
    };
    let reference = &candidate["ref"];
    Ok(match (candidate["op"].as_str().unwrap_or(""), choice) {
        ("click", _) => ("act.click".into(), json!({"tabId": tab, "ref": reference})),
        // A radio button is chosen, never cleared by choosing it again.
        ("check", _)
            if candidate["target"]
                .as_str()
                .is_some_and(|t| t.starts_with("radio")) =>
        {
            ("act.check".into(), json!({"tabId": tab, "ref": reference}))
        }
        // A box is ticked (never cleared by choosing it again), or cleared
        // when that is what the subgoal asks.
        ("check", _) if clearing => ("act.toggle".into(), json!({"tabId": tab, "ref": reference})),
        ("check", _) => ("act.check".into(), json!({"tabId": tab, "ref": reference})),
        ("fill", _) => (
            "act.fill".into(),
            json!({"tabId": tab, "ref": reference, "text": value()?}),
        ),
        ("select", _) => (
            "act.select_option".into(),
            json!({"tabId": tab, "ref": reference, "value": value()?}),
        ),
        ("scroll", "scroll_up") => ("act.scroll".into(), json!({"tabId": tab, "pages": -1})),
        ("scroll", _) => ("act.scroll".into(), json!({"tabId": tab, "pages": 1})),
        ("back", _) => ("tab.back".into(), json!({"tabId": tab})),
        ("wait", _) => ("wait.settle".into(), json!({"tabId": tab})),
        (other, _) => return Err(format!("control:{other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTLINE: &str = "# Checkout\n[3] textbox \"Email\"\n[4] combobox \"Country\"\n[5] checkbox \"Remember me\"\n[6] button \"Pay\" disabled\n[7] link \"Help\"";

    #[test]
    fn candidates_are_the_possible_steps_and_the_ways_out() {
        let found = candidates(OUTLINE);
        let brief: Vec<String> = found
            .iter()
            .map(|c| {
                format!(
                    "{}:{}",
                    c["id"].as_str().unwrap(),
                    c["op"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            brief,
            [
                "r3:fill",
                "r4:select",
                "r5:check",
                "r7:click",
                "scroll_down:scroll",
                "scroll_up:scroll",
                "back:back",
                "wait:wait",
                "done:done",
                "escalate:escalate"
            ]
        );
    }

    #[test]
    fn boxes_are_ticked_and_read_only_fields_clicked() {
        let outline = concat!(
            "[1] checkbox \"Hhc\" checked\n",
            "[2] checkbox \"4jU\"\n",
            "[3] textbox \"\" readonly\n",
            "[4] radio \"A\" checked"
        );
        let ops: Vec<String> = candidates(outline)
            .iter()
            .take_while(|c| c.get("ref").is_some())
            .map(|c| {
                format!(
                    "{}:{}",
                    c["id"].as_str().unwrap(),
                    c["op"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(ops, ["r1:check", "r2:check", "r3:click", "r4:check"]);
        let view = json!({"page": outline});
        record(
            "c",
            "t",
            "1".into(),
            &view,
            json!([]),
            &json!({"subgoal": "Check 4jU"}),
        )
        .unwrap();
        // Choosing a ticked box again leaves it ticked.
        assert_eq!(
            resolve("c", "t", "1", &json!({"choose": "r1"})).unwrap().0,
            "act.check"
        );
        record(
            "c",
            "t",
            "2".into(),
            &view,
            json!([]),
            &json!({"subgoal": "Uncheck Hhc"}),
        )
        .unwrap();
        assert_eq!(
            resolve("c", "t", "2", &json!({"choose": "r1"})).unwrap().0,
            "act.toggle"
        );
    }

    #[test]
    fn a_choice_runs_only_for_the_revision_and_values_it_was_made_with() {
        let view = json!({"page": OUTLINE, "url": "https://shop.example/", "title": "Checkout"});
        let made = record(
            "s",
            "t",
            "7-5".into(),
            &view,
            json!([]),
            &json!({"subgoal": "enter the email", "values": {"email": "ada@example.com"}}),
        )
        .unwrap();
        assert_eq!(made["schema"], SCHEMA);
        assert_eq!(made["values"]["email"], "ada@example.com");
        assert_eq!(
            resolve("s", "t", "7-5", &json!({"choose": "r3", "value": "email"})).unwrap(),
            (
                "act.fill".to_string(),
                json!({"tabId": "t", "ref": 3, "text": "ada@example.com"})
            )
        );
        // The page moved on, another session, an id that was not offered, a
        // disabled control, and text the planner never supplied: all refused.
        assert!(
            resolve("s", "t", "8-5", &json!({"choose": "r3", "value": "email"}))
                .unwrap_err()
                .starts_with("stale")
        );
        assert!(resolve(
            "other",
            "t",
            "7-5",
            &json!({"choose": "r3", "value": "email"})
        )
        .is_err());
        assert!(resolve("s", "t", "7-5", &json!({"choose": "r99"})).is_err());
        assert!(resolve("s", "t", "7-5", &json!({"choose": "r6"})).is_err());
        assert!(resolve(
            "s",
            "t",
            "7-5",
            &json!({"choose": "r3", "value": "password"})
        )
        .is_err());
        assert!(resolve("s", "t", "7-5", &json!({"choose": "r3"})).is_err());
        assert_eq!(
            resolve("s", "t", "7-5", &json!({"choose": "done"})).unwrap_err(),
            "control:done"
        );
        assert!(record(
            "s",
            "t",
            "1".into(),
            &view,
            json!([]),
            &json!({"subgoal": " "})
        )
        .is_err());
        assert!(record(
            "s",
            "t",
            "1".into(),
            &view,
            json!([]),
            &json!({"subgoal": "x", "values": {"n": 1}})
        )
        .is_err());
    }
}
