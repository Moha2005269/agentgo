//! Narrowing a real page to what GoMini can take.
//!
//! `decision.record` lists every control on the page; real sites have
//! dozens to hundreds, GoMini takes at most 24 choices in 1,024 tokens. The
//! shortlist keeps the controls whose words match the subgoal (the planner
//! is told to name things the way the page does), the ways out (scroll,
//! back, wait, done, escalate) and the outline lines around them. It never
//! invents a candidate; if nothing matches, GoMini only sees the ways out
//! and will usually escalate, which hands the step back to the planner.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::pack::MAX_CHOICES;

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "into", "from", "then", "page", "field", "button",
    "link", "click", "fill", "enter", "type", "select", "choose", "value", "values", "supplied",
    "requested", "should", "must", "have", "has", "the", "its", "your", "you", "are", "not",
    "only", "when", "all", "every", "any", "text", "box", "open", "put",
];

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

fn is_control(candidate: &Value) -> bool {
    !candidate["id"].as_str().unwrap_or_default().starts_with('r')
        || candidate.get("ref").is_none()
}

/// How many GoMini choices a candidate becomes.
fn width(candidate: &Value, values: usize) -> usize {
    match candidate["op"].as_str().unwrap_or_default() {
        "fill" | "select" => values,
        _ => 1,
    }
}

/// A copy of `record` with at most `keep` page candidates (fewer if needed
/// to stay within 24 choices) and an outline reduced to them plus the
/// non-control lines, up to `outline_chars`.
pub fn shortlist(record: &Value, keep: usize, outline_chars: usize) -> Value {
    let values = record["values"].as_object().map_or(0, |m| m.len());
    let mut goal = words(record["subgoal"].as_str().unwrap_or_default());
    goal.extend(words(record["completion"].as_str().unwrap_or_default()));
    if let Some(map) = record["values"].as_object() {
        for name in map.keys() {
            goal.extend(words(name));
        }
    }
    let candidates: Vec<Value> = record["candidates"].as_array().cloned().unwrap_or_default();
    let controls: Vec<Value> = candidates.iter().filter(|c| is_control(c)).cloned().collect();
    let mut scored: Vec<(usize, usize, &Value)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !is_control(c))
        .filter_map(|(i, c)| {
            let overlap = words(c["target"].as_str().unwrap_or_default())
                .intersection(&goal)
                .count();
            (overlap > 0).then_some((overlap, i, c))
        })
        .collect();
    // Best match first; ties keep page order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut budget = MAX_CHOICES - 1 - controls.len();
    let mut kept: Vec<(usize, Value)> = Vec::new();
    for (_, index, candidate) in scored {
        let need = width(candidate, values);
        if kept.len() >= keep || need == 0 || need > budget {
            continue;
        }
        budget -= need;
        kept.push((index, candidate.clone()));
    }
    kept.sort_by_key(|(index, _)| *index);
    let refs: HashSet<String> = kept
        .iter()
        .filter_map(|(_, c)| c["ref"].as_u64().map(|r| format!("[{r}]")))
        .collect();
    let mut outline = String::new();
    for line in record["state"]["outline"].as_str().unwrap_or_default().lines() {
        let control = line.starts_with('[') && line.contains("] ");
        let keep_line = if control {
            refs.iter().any(|r| line.starts_with(r.as_str()))
        } else {
            outline.len() + line.len() < outline_chars
        };
        if keep_line {
            outline.push_str(line);
            outline.push('\n');
        }
    }
    let mut out = record.clone();
    out["candidates"] = json!(kept
        .into_iter()
        .map(|(_, c)| c)
        .chain(controls)
        .collect::<Vec<_>>());
    out["state"]["outline"] = json!(outline.trim_end());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_matching_controls_and_the_ways_out() {
        let mut candidates: Vec<Value> = (0..60)
            .map(|i| json!({"id": format!("r{i}"), "op": "click", "ref": i, "target": format!("link \"Article {i}\"")}))
            .collect();
        candidates.push(json!({"id": "r100", "op": "fill", "ref": 100, "target": "textbox \"Email address\""}));
        candidates.push(json!({"id": "r101", "op": "click", "ref": 101, "target": "button \"Newsletter signup\""}));
        for id in ["scroll_down", "scroll_up", "back", "wait", "done", "escalate"] {
            candidates.push(json!({"id": id, "op": id, "target": "control"}));
        }
        let outline: String = (0..60)
            .map(|i| format!("[{i}] link \"Article {i}\"\n"))
            .collect::<String>()
            + "# Newsletter\n[100] textbox \"Email address\"\n[101] button \"Newsletter signup\"";
        let record = json!({
            "subgoal": "put the email into the Email address field of the newsletter form",
            "values": {"email": "a@b.c"},
            "state": {"outline": outline},
            "candidates": candidates,
        });
        let short = shortlist(&record, 12, 400);
        let ids: Vec<&str> = short["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["r100", "r101", "scroll_down", "scroll_up", "back", "wait", "done", "escalate"]);
        let outline = short["state"]["outline"].as_str().unwrap();
        assert!(outline.contains("[100] textbox") && outline.contains("# Newsletter"));
        assert!(!outline.contains("Article 7"));
    }
}
