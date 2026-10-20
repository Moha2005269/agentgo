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
    "the",
    "and",
    "for",
    "with",
    "this",
    "that",
    "into",
    "from",
    "then",
    "page",
    "field",
    "button",
    "link",
    "click",
    "fill",
    "enter",
    "type",
    "select",
    "choose",
    "value",
    "values",
    "supplied",
    "requested",
    "should",
    "must",
    "have",
    "has",
    "the",
    "its",
    "your",
    "you",
    "are",
    "not",
    "only",
    "when",
    "all",
    "every",
    "any",
    "text",
    "box",
    "open",
    "put",
];

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        // "tabs" names "Tab 2", "folders" a folder: compare singular forms.
        .map(|w| match w.strip_suffix('s') {
            Some(stem) if stem.len() >= 3 && !stem.ends_with('s') => stem.to_string(),
            _ => w.to_string(),
        })
        .collect()
}

/// Controls that reveal other controls (a tab, a tree item, a collapsed
/// section, a "more" menu): the way to a target the subgoal names but the
/// page does not show yet.
fn reveals(candidate: &Value) -> bool {
    let target = candidate["target"].as_str().unwrap_or_default();
    target.starts_with("tab ")
        || target.starts_with("treeitem ")
        || target.starts_with("menuitem ")
        || target.contains(" collapsed")
        || target.contains("(icon: more")
        || target.contains("(icon: menu")
        || target.contains("(icon: toggle")
        || target.contains("(icon: expand")
}

fn is_control(candidate: &Value) -> bool {
    !candidate["id"]
        .as_str()
        .unwrap_or_default()
        .starts_with('r')
        || candidate.get("ref").is_none()
}

/// How many GoMini choices a candidate becomes.
fn width(candidate: &Value, values: usize) -> usize {
    match candidate["op"].as_str().unwrap_or_default() {
        "fill" | "select" => candidate["bind"].as_array().map_or(values, Vec::len),
        _ => 1,
    }
}

/// The fields the planner named for each value: `Primary City=city` in the
/// subgoal binds the value `city` to the field labelled "Primary City".
fn named_bindings(subgoal: &str, names: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // " and " separates pairs as often as a comma does in plain language.
    for part in subgoal.replace(" and ", ";").split([';', ',', '.', '\n']) {
        let Some((label, name)) = part.rsplit_once('=') else {
            continue;
        };
        let name = name
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
        if !names.iter().any(|n| n == name) {
            continue;
        }
        if let Some(label) = label_before(label, name) {
            out.push((label, name.to_string()));
        }
    }
    out
}

/// The field label written before `=name`. Takes the planner's usual forms:
/// "using Primary City=city", "Primary City=city", and the plain-language
/// "fill the Username field with user=user" (label "username"), where the
/// text before "=" ends in the value's own name.
fn label_before(text: &str, name: &str) -> Option<String> {
    let mut label = text.trim().trim_matches('"').to_lowercase();
    // "... Username field with user=user": the label is before "with".
    let tail = format!(" with {}", name.to_lowercase());
    if let Some(head) = label.strip_suffix(&tail) {
        label = head.to_string();
    }
    for lead in [" using ", " with ", " into "] {
        if let Some((_, after)) = label.rsplit_once(lead) {
            if !after.trim().is_empty() {
                label = after.to_string();
            }
        }
    }
    let mut words: Vec<&str> = label
        .split(|c: char| c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    while words.first().is_some_and(|w| {
        matches!(
            *w,
            "fill"
                | "enter"
                | "type"
                | "put"
                | "set"
                | "the"
                | "a"
                | "an"
                | "in"
                | "into"
                | "and"
                | "then"
        )
    }) {
        words.remove(0);
    }
    while words
        .last()
        .is_some_and(|w| matches!(*w, "field" | "box" | "input" | "textbox" | "fields"))
    {
        words.pop();
    }
    let label = words.join(" ").trim_matches('"').trim().to_string();
    (!label.is_empty() && label != name.to_lowercase()).then_some(label)
}

/// Lower-case words of a label, without punctuation ("E-mail address:" is
/// "e mail address").
fn label_words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The quoted label of a candidate (`textbox(tel) "Telephone:"`).
fn candidate_label(target: &str) -> &str {
    match (target.find('"'), target.rfind('"')) {
        (Some(start), Some(end)) if end > start => &target[start + 1..end],
        _ => target,
    }
}

/// Whether the field's label is the one the planner named: the same words,
/// ignoring punctuation and a trailing colon or asterisk ("Telephone" names
/// "Telephone:"), or the named words at the start of a longer label
/// ("Email" names "Email address").
fn label_matches(named: &str, field: &str) -> bool {
    let (named, field) = (label_words(named), label_words(field));
    !named.is_empty()
        && (field == named
            || field.starts_with(&format!("{named} "))
            || field.replace(' ', "") == named.replace(' ', ""))
}

/// Marks fill/select candidates whose label the planner named with the
/// value(s) bound to them (`bind`), so each is offered with its own value.
/// A value bound to a field on the page is not offered for other fields
/// (`boundValues`).
pub fn bind(record: &mut Value) {
    let names: Vec<String> = record["values"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let pairs = named_bindings(record["subgoal"].as_str().unwrap_or_default(), &names);
    if pairs.is_empty() {
        return;
    }
    let mut placed: Vec<String> = Vec::new();
    if let Some(candidates) = record["candidates"].as_array_mut() {
        for candidate in candidates.iter_mut() {
            if !matches!(candidate["op"].as_str(), Some("fill" | "select")) {
                continue;
            }
            let label =
                candidate_label(candidate["target"].as_str().unwrap_or_default()).to_string();
            let bound: Vec<&str> = pairs
                .iter()
                .filter(|(named, _)| label_matches(named, &label))
                .map(|(_, name)| name.as_str())
                .collect();
            if !bound.is_empty() {
                placed.extend(bound.iter().map(|n| n.to_string()));
                candidate["bind"] = json!(bound);
            }
        }
    }
    if !placed.is_empty() {
        record["boundValues"] = json!(placed);
    }
}

/// A copy of `record` with at most `keep` page candidates (fewer if needed
/// to stay within 24 choices) and an outline reduced to them plus the
/// non-control lines, up to `outline_chars`.
pub fn shortlist(record: &Value, keep: usize, outline_chars: usize) -> Value {
    let mut record = record.clone();
    bind(&mut record);
    let record = &record;
    let values = record["values"].as_object().map_or(0, |m| m.len());
    let mut goal = words(record["subgoal"].as_str().unwrap_or_default());
    goal.extend(words(record["completion"].as_str().unwrap_or_default()));
    if let Some(map) = record["values"].as_object() {
        for name in map.keys() {
            goal.extend(words(name));
        }
    }
    let candidates: Vec<Value> = record["candidates"].as_array().cloned().unwrap_or_default();
    let controls: Vec<Value> = candidates
        .iter()
        .filter(|c| is_control(c))
        .cloned()
        .collect();
    // Ranked: controls the subgoal's words name (best match first), then
    // controls that reveal others (tabs, tree items, "more" menus), then the
    // rest of the page in order. A small page fits whole; on a large one the
    // named controls and the ways to hidden ones come first.
    let mut scored: Vec<(usize, usize, &Value)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !is_control(c))
        .map(|(i, c)| {
            let overlap = words(c["target"].as_str().unwrap_or_default())
                .intersection(&goal)
                .count();
            let rank = if overlap > 0 {
                100 + overlap
            } else if reveals(c) {
                50
            } else {
                0
            };
            (rank, i, c)
        })
        .collect();
    // Unnamed, non-revealing controls only when the whole page fits; on a
    // large page they would only crowd out the ones that matter.
    let page_fits = scored.len() <= keep;
    scored.retain(|(rank, _, _)| *rank > 0 || page_fits);
    // Best rank first; ties keep page order.
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
    for line in record["state"]["outline"]
        .as_str()
        .unwrap_or_default()
        .lines()
    {
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
    fn plain_language_subgoals_name_their_fields() {
        let names = vec!["user".to_string(), "pw".to_string(), "city".to_string()];
        let pairs = named_bindings(
            "Fill the Username field with user=user and the Password field with pw=pw",
            &names,
        );
        assert!(
            pairs.contains(&("username".into(), "user".into())),
            "{pairs:?}"
        );
        let pairs = named_bindings("Fill this travel form using Primary City=city", &names);
        assert_eq!(pairs, vec![("primary city".into(), "city".into())]);
        let pairs = named_bindings("Enter city=city", &names);
        assert!(
            pairs.is_empty(),
            "a value named after itself names no field: {pairs:?}"
        );
    }

    #[test]
    fn labels_match_without_their_punctuation() {
        let mut record = json!({
            "subgoal": "Fill this order form using Customer name=name; Telephone=phone; E-mail address=email. Never submit.",
            "values": {"name": "a", "phone": "b", "email": "c"},
            "candidates": [
                {"id": "r1", "op": "fill", "ref": 1, "target": "textbox \"Customer name:\""},
                {"id": "r2", "op": "fill", "ref": 2, "target": "textbox(tel) \"Telephone:\""},
                {"id": "r3", "op": "fill", "ref": 3, "target": "textbox(email) \"E-mail address:\""},
                {"id": "r4", "op": "fill", "ref": 4, "target": "textbox \"Delivery instructions:\""}
            ]
        });
        bind(&mut record);
        let binds: Vec<Value> = record["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["bind"].clone())
            .collect();
        assert_eq!(
            binds,
            vec![
                json!(["name"]),
                json!(["phone"]),
                json!(["email"]),
                Value::Null
            ]
        );
        assert_eq!(record["boundValues"], json!(["name", "phone", "email"]));
        assert!(label_matches("Email", "Email address"));
        assert!(!label_matches("Email", "Delivery instructions"));
    }

    #[test]
    fn named_fields_get_only_their_own_value() {
        let record = json!({
            "subgoal": "Fill this Library form using Book title=v1; Author=v2; Shelf=v3; Genre=v4. Save draft when all required fields match.",
            "values": {"v1": "a", "v2": "b", "v3": "c", "v4": "d"},
            "state": {"outline": ""},
            "candidates": [
                {"id": "r1", "op": "fill", "ref": 1, "target": "textbox \"Book title\" value=\"\""},
                {"id": "r2", "op": "fill", "ref": 2, "target": "textbox \"Author\" value=\"\""},
                {"id": "r3", "op": "fill", "ref": 3, "target": "textbox \"Shelf\" value=\"\""},
                {"id": "r4", "op": "select", "ref": 4, "target": "combobox \"Genre\" value=\"Choose\""},
                {"id": "r5", "op": "fill", "ref": 5, "target": "textbox \"Search\""},
                {"id": "done", "op": "done", "target": "x"},
            ],
        });
        let short = shortlist(&record, 16, 400);
        let binds: Vec<Value> = short["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["bind"].clone())
            .collect();
        assert_eq!(
            binds[..4],
            [json!(["v1"]), json!(["v2"]), json!(["v3"]), json!(["v4"])]
        );
        let executor = crate::pack::Executor {
            cancelled: false,
            attempts_remaining: 8,
        };
        let (request, _) = crate::pack::from_record(&short, "", &[], &executor, None).unwrap();
        let keys: Vec<&str> = request.criteria.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(&keys[..4], ["r1|v1", "r2|v2", "r3|v3", "r4|v4"]);
        assert!(named_bindings("no names here", &["v1".into()]).is_empty());
    }

    #[test]
    fn keeps_matching_controls_and_the_ways_out() {
        let mut candidates: Vec<Value> = (0..60)
            .map(|i| json!({"id": format!("r{i}"), "op": "click", "ref": i, "target": format!("link \"Article {i}\"")}))
            .collect();
        candidates.push(
            json!({"id": "r100", "op": "fill", "ref": 100, "target": "textbox \"Email address\""}),
        );
        candidates.push(json!({"id": "r101", "op": "click", "ref": 101, "target": "button \"Newsletter signup\""}));
        for id in [
            "scroll_down",
            "scroll_up",
            "back",
            "wait",
            "done",
            "escalate",
        ] {
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
        assert_eq!(
            ids,
            [
                "r100",
                "r101",
                "scroll_down",
                "scroll_up",
                "back",
                "wait",
                "done",
                "escalate"
            ]
        );
        let outline = short["state"]["outline"].as_str().unwrap();
        assert!(outline.contains("[100] textbox") && outline.contains("# Newsletter"));
        assert!(!outline.contains("Article 7"));
    }
}
