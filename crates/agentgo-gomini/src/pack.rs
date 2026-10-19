//! The `gomini.agentgo/1` contract, ported from the model package's
//! `adapter/gomini_contract.py` and `adapter/gguf_input.py`.
//!
//! A decision record becomes one question with 2..24 named choices. Nothing
//! is ever cut to fit: an input over a limit is refused and the caller asks
//! the planner to narrow the subgoal. Every input is also checked token by
//! token against the worker's own tokenizer before the model sees it.

use serde_json::{json, Value};

use crate::pyjson::{self, Py};

/// Contract name.
pub const SCHEMA: &str = "gomini.agentgo/1";
/// Whole input, in tokens.
pub const MAX_LEN: usize = 1024;
/// Instruction plus options, in tokens.
pub const HEAD_LEN: usize = 512;
/// Choices per question.
pub const MAX_CHOICES: usize = 24;
/// Tokens per rendered option.
pub const OPTION_LEN: usize = 48;

/// The tokenizer's special tokens (tokenizer_config.json), longest first.
const SPECIAL: [&str; 7] = [
    "<start_of_turn>",
    "<end_of_turn>",
    "<mask>",
    "<bos>",
    "<eos>",
    "<unk>",
    "<pad>",
];
const CLS: &str = "<bos>";
const SEP: &str = "<eos>";
const MASK: &str = "<mask>";

/// The supervisor control the adapter always offers.
pub const STOP: (&str, &str) = (
    "stop",
    "Stop: cancelled, permission revoked, or attempt limit reached",
);

/// Replaces tokenizer markers in untrusted text so a page cannot change the
/// structure of the input (`<mask>` becomes `＜mask＞`).
pub fn escape(text: &str) -> String {
    let mut text = text.to_string();
    for token in SPECIAL {
        let replacement: String = token
            .chars()
            .map(|c| match c {
                '<' => '＜',
                '>' => '＞',
                '[' => '［',
                ']' => '］',
                other => other,
            })
            .collect();
        text = text.replace(token, &replacement);
    }
    text
}

/// What a choice key runs: a candidate id and, for typing, a value name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Candidate id from `decision.record` (`r12`, `scroll_down`, `stop`, ...).
    pub choose: String,
    /// Supplied value name for fill and select.
    pub value: Option<String>,
}

/// One GoMini question, already escaped, keys in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// The serialised state.
    pub state: String,
    /// `Goal: ... Completion: ... Constraints: ...`.
    pub instructions: String,
    /// Choice key and description, in the order the model sees them.
    pub criteria: Vec<(String, String)>,
}

/// What the executor tells the model about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Executor {
    /// The planner or person stopped the subgoal.
    pub cancelled: bool,
    /// Steps left in the budget.
    pub attempts_remaining: u64,
}

/// Builds a question from a `decision.record` answer (as `from_record` does).
///
/// `text` is the readable page text (with status lines), `recent` the last
/// own steps (`{"choice", "ok"}`), both chosen by the caller. `readback`,
/// when given, is what each supplied value's field holds now
/// (`{"city": "Maple-3321", "postal": ""}`), placed after `executor` as the
/// multi-step form training did. Fails when the record has more than 24
/// choices: the caller must narrow it first.
pub fn from_record(
    record: &Value,
    text: &str,
    recent: &[Value],
    executor: &Executor,
    readback: Option<Vec<(String, Value)>>,
) -> Result<(Request, Vec<(String, Action)>), String> {
    let mut choices: Vec<(String, String)> = Vec::new();
    let mut actions: Vec<(String, Action)> = Vec::new();
    let values: Vec<String> = record["values"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for candidate in record["candidates"].as_array().into_iter().flatten() {
        let id = candidate["id"].as_str().unwrap_or_default();
        let op = candidate["op"].as_str().unwrap_or_default();
        let target = candidate["target"].as_str().unwrap_or_default();
        if op == "fill" || op == "select" {
            // A field the planner named ("Postal code=postal") is offered with
            // its own value only (see shortlist::bind); otherwise with each.
            let bound: Option<Vec<String>> = candidate["bind"].as_array().map(|names| {
                names
                    .iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect()
            });
            // A value the planner bound to another field on this page is
            // not offered here.
            let elsewhere: Vec<&str> = record["boundValues"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            for name in values.iter().filter(|n| match &bound {
                Some(b) => b.contains(n),
                None => !elsewhere.contains(&n.as_str()),
            }) {
                let key = format!("{id}|{name}");
                choices.push((key.clone(), format!("{op} {target} with value {name}")));
                actions.push((
                    key,
                    Action {
                        choose: id.into(),
                        value: Some(name.clone()),
                    },
                ));
            }
        } else {
            choices.push((id.into(), format!("{op} {target}")));
            actions.push((
                id.into(),
                Action {
                    choose: id.into(),
                    value: None,
                },
            ));
        }
    }
    choices.push((STOP.0.into(), STOP.1.into()));
    actions.push((
        STOP.0.into(),
        Action {
            choose: STOP.0.into(),
            value: None,
        },
    ));
    if choices.len() > MAX_CHOICES {
        return Err(format!(
            "too many candidates ({}): the subgoal must be narrowed to at most {MAX_CHOICES} choices",
            choices.len()
        ));
    }
    let recent: Vec<Value> = recent.iter().rev().take(3).rev().cloned().collect();
    let mut state = vec![
        (
            "outline".into(),
            Py::Val(record["state"]["outline"].clone()),
        ),
        ("text".into(), Py::Val(json!(text))),
        ("values".into(), Py::Val(record["values"].clone())),
        ("recent".into(), Py::Val(json!(recent))),
        (
            "executor".into(),
            Py::Obj(vec![
                ("cancelled".into(), Py::Val(json!(executor.cancelled))),
                (
                    "attempts_remaining".into(),
                    Py::Val(json!(executor.attempts_remaining)),
                ),
            ]),
        ),
    ];
    if let Some(readback) = readback {
        state.push((
            "field_readback".into(),
            Py::Obj(readback.into_iter().map(|(k, v)| (k, Py::Val(v))).collect()),
        ));
    }
    let state = Py::Obj(state);
    let mut instructions = format!("Goal: {}", record["subgoal"].as_str().unwrap_or_default());
    if let Some(completion) = record["completion"].as_str().filter(|c| !c.is_empty()) {
        instructions.push_str(&format!(" Completion: {completion}"));
    }
    let constraints: Vec<&str> = record["constraints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !constraints.is_empty() {
        instructions.push_str(&format!(" Constraints: {}", constraints.join("; ")));
    }
    let request = Request {
        state: escape(&pyjson::dumps(&state)),
        instructions: escape(&instructions),
        criteria: choices.into_iter().map(|(k, v)| (k, escape(&v))).collect(),
    };
    Ok((request, actions))
}

/// The JSON body for `POST /v1/systemone`, keys in order.
pub fn body(request: &Request) -> String {
    let criteria: Vec<String> = request
        .criteria
        .iter()
        .map(|(k, v)| format!("{}:{}", pyjson::string(k), pyjson::string(v)))
        .collect();
    format!(
        r#"{{"state":{},"questions":{{"next":{{"type":"choice","instructions":{},"criteria":{{{}}}}}}}}}"#,
        pyjson::string(&request.state),
        pyjson::string(&request.instructions),
        criteria.join(",")
    )
}

/// Something that turns text into the model's token ids.
pub trait Tokenize {
    /// Token ids with special tokens parsed and none added.
    fn tokens(&self, text: &str) -> Result<Vec<i64>, String>;
}

/// A request that passed every check, with its exact token ids.
#[derive(Debug, Clone, PartialEq)]
pub struct Packed {
    /// The input ids the model will see.
    pub ids: Vec<i64>,
    /// Choice keys in order.
    pub keys: Vec<String>,
}

fn single(tok: &dyn Tokenize, special: &str) -> Result<i64, String> {
    match tok.tokens(special)?.as_slice() {
        [id] => Ok(*id),
        other => Err(format!("{special} is not one token ({other:?})")),
    }
}

/// Checks a request against every limit and the tokenizer (`encode` plus
/// `checked_request`): refuses rather than truncates.
pub fn check(tok: &dyn Tokenize, request: &Request) -> Result<Packed, String> {
    let n = request.criteria.len();
    if !(2..=MAX_CHOICES).contains(&n) {
        return Err(format!("GoMini needs 2..{MAX_CHOICES} choices, got {n}"));
    }
    let (cls, sep, mask) = (single(tok, CLS)?, single(tok, SEP)?, single(tok, MASK)?);
    let head = tok.tokens(&format!("choice question: {}", request.instructions))?;
    let rendered: Vec<String> = request
        .criteria
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                k.clone()
            } else {
                format!("{k}: {v}")
            }
        })
        .collect();
    let options: Vec<Vec<i64>> = rendered
        .iter()
        .map(|text| tok.tokens(&format!(" {text}")))
        .collect::<Result<_, _>>()?;
    if let Some((i, _)) = options
        .iter()
        .enumerate()
        .find(|(_, o)| o.len() > OPTION_LEN)
    {
        return Err(format!(
            "choice {} is longer than {OPTION_LEN} tokens; the planner must shorten it",
            request.criteria[i].0
        ));
    }
    if options.iter().map(|o| o.len() + 1).sum::<usize>() + head.len().max(16) > HEAD_LEN {
        return Err(
            "the subgoal and its choices exceed the 512-token head; narrow the subgoal".into(),
        );
    }
    let state = tok.tokens(&request.state)?;
    let mut ids = vec![cls];
    ids.extend(&head);
    ids.push(sep);
    for option in &options {
        ids.push(mask);
        ids.extend(option);
    }
    ids.push(sep);
    ids.extend(&state);
    ids.push(sep);
    if ids.len() > MAX_LEN {
        return Err(format!(
            "the page state is {} tokens over the {MAX_LEN}-token limit; compact it",
            ids.len() - MAX_LEN
        ));
    }
    if ids.iter().filter(|id| **id == mask).count() != n {
        return Err("an unescaped marker reached the input".into());
    }
    let options_text: String = rendered
        .iter()
        .map(|text| format!("{MASK} {text}"))
        .collect();
    let prompt = format!(
        "{CLS}choice question: {}{SEP}{options_text}{SEP}{}{SEP}",
        request.instructions, request.state
    );
    if tok.tokens(&prompt)? != ids {
        return Err(
            "tokenisation differs between pieces and the whole input; reword or compact".into(),
        );
    }
    Ok(Packed {
        ids,
        keys: request.criteria.iter().map(|(k, _)| k.clone()).collect(),
    })
}

/// GoMini's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The chosen key.
    pub choice: String,
    /// Probability per key (uncalibrated).
    pub probabilities: Vec<(String, f64)>,
    /// Highest probability.
    pub top_probability: f64,
    /// Highest minus second highest.
    pub margin: f64,
    /// 1 - normalised entropy.
    pub entropy_confidence: f64,
    /// Input tokens the worker read.
    pub input_tokens: usize,
}

/// Reads a `/v1/systemone` answer, refusing anything that does not match
/// the checked request exactly.
pub fn read_answer(answer: &Value, packed: &Packed) -> Result<Choice, String> {
    if answer["usage"]["input_tokens"].as_u64() != Some(packed.ids.len() as u64) {
        return Err("the worker read a different number of tokens than were checked".into());
    }
    let next = &answer["answers"]["next"];
    let choice = next["choice"]
        .as_str()
        .ok_or("the worker gave no choice")?
        .to_string();
    let map = next["probabilities"]
        .as_object()
        .ok_or("the worker gave no probabilities")?;
    if !packed.keys.contains(&choice)
        || map.len() != packed.keys.len()
        || packed.keys.iter().any(|k| !map.contains_key(k))
    {
        return Err("the worker's answer does not match the offered choices".into());
    }
    let probabilities: Vec<(String, f64)> = packed
        .keys
        .iter()
        .map(|k| (k.clone(), map[k].as_f64().unwrap_or(0.0)))
        .collect();
    let mut sorted: Vec<f64> = probabilities.iter().map(|(_, p)| *p).collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let entropy: f64 = sorted.iter().map(|p| -p * p.max(1e-12).ln()).sum();
    Ok(Choice {
        choice,
        top_probability: sorted[0],
        margin: sorted[0] - sorted.get(1).copied().unwrap_or(0.0),
        entropy_confidence: if sorted.len() > 1 {
            1.0 - entropy / (sorted.len() as f64).ln()
        } else {
            1.0
        },
        probabilities,
        input_tokens: packed.ids.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One token per word (with its leading space), specials as single
    /// tokens: enough to test the limits and the piece/whole comparison.
    struct CharTok;
    impl Tokenize for CharTok {
        fn tokens(&self, text: &str) -> Result<Vec<i64>, String> {
            let mut out = Vec::new();
            let mut rest = text;
            'outer: while !rest.is_empty() {
                for (i, special) in [CLS, SEP, MASK].iter().enumerate() {
                    if let Some(after) = rest.strip_prefix(special) {
                        out.push(-(i as i64) - 1);
                        rest = after;
                        continue 'outer;
                    }
                }
                let start = usize::from(rest.starts_with(' '));
                let end = rest[start..]
                    .find([' ', '<'])
                    .map_or(rest.len(), |i| (i + start).max(1));
                let word = &rest[..end];
                out.push(
                    word.bytes()
                        .fold(7i64, |h, b| h.wrapping_mul(31).wrapping_add(b as i64))
                        .abs(),
                );
                rest = &rest[end..];
            }
            Ok(out)
        }
    }

    fn record() -> Value {
        json!({
            "subgoal": "Enter the supplied entry in Primary Postal code.",
            "completion": "Primary Postal code has the requested value.",
            "constraints": ["Do not submit other forms."],
            "values": {"entry": "Pacific-96778"},
            "state": {"outline": "[748] textbox \"Primary Postal code\" value=\"\""},
            "candidates": [
                {"id": "r748", "op": "fill", "ref": 748, "target": "textbox \"Primary Postal code\" value=\"\""},
                {"id": "r9", "op": "click", "ref": 9, "target": "button \"<mask> Save\""},
                {"id": "scroll_down", "op": "scroll", "target": "show the next part of the page"},
                {"id": "done", "op": "done", "target": "the completion condition holds"},
            ]
        })
    }

    #[test]
    fn a_record_becomes_the_trained_question() {
        let executor = Executor {
            cancelled: false,
            attempts_remaining: 7,
        };
        let (request, actions) = from_record(
            &record(),
            "Status: Page ready.",
            &[json!({"choice": "r9", "ok": true})],
            &executor,
            None,
        )
        .unwrap();
        let keys: Vec<&str> = request.criteria.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["r748|entry", "r9", "scroll_down", "done", "stop"]);
        assert_eq!(
            request.criteria[0].1,
            "fill textbox \"Primary Postal code\" value=\"\" with value entry"
        );
        // A marker inside page text is escaped, never left as a token.
        assert_eq!(request.criteria[1].1, "click button \"＜mask＞ Save\"");
        assert_eq!(
            request.instructions,
            "Goal: Enter the supplied entry in Primary Postal code. Completion: Primary Postal code has the requested value. Constraints: Do not submit other forms."
        );
        assert!(request.state.starts_with(r#"{"outline": "[748] textbox \"Primary Postal code\" value=\"\"", "text": "Status: Page ready.", "values": {"entry": "Pacific-96778"}, "recent": [{"choice": "r9", "ok": true}], "executor": {"cancelled": false, "attempts_remaining": 7}}"#));
        assert_eq!(
            actions[0].1,
            Action {
                choose: "r748".into(),
                value: Some("entry".into())
            }
        );
        let packed = check(&CharTok, &request).unwrap();
        assert_eq!(packed.keys.len(), 5);
        assert_eq!(packed.ids.iter().filter(|id| **id == -3).count(), 5);
        let body: Value = serde_json::from_str(&body(&request)).unwrap();
        assert_eq!(body["questions"]["next"]["type"], "choice");
        assert_eq!(body["state"], request.state);
    }

    #[test]
    fn oversized_inputs_are_refused_not_truncated() {
        let executor = Executor {
            cancelled: false,
            attempts_remaining: 3,
        };
        let mut many = record();
        many["candidates"] = json!((0..30)
            .map(|i| json!({"id": format!("r{i}"), "op": "click", "target": "x"}))
            .collect::<Vec<_>>());
        assert!(from_record(&many, "", &[], &executor, None)
            .unwrap_err()
            .contains("narrowed"));

        let mut long = record();
        long["state"]["outline"] = json!("y ".repeat(1200));
        let (request, _) = from_record(&long, "", &[], &executor, None).unwrap();
        assert!(check(&CharTok, &request)
            .unwrap_err()
            .contains("over the 1024"));

        let mut wide = record();
        wide["candidates"][1]["target"] = json!("z ".repeat(60));
        let (request, _) = from_record(&wide, "", &[], &executor, None).unwrap();
        assert!(check(&CharTok, &request)
            .unwrap_err()
            .contains("longer than 48"));
    }

    #[test]
    fn answers_must_match_the_checked_request() {
        let packed = Packed {
            ids: vec![1, 2, 3],
            keys: vec!["a".into(), "b".into()],
        };
        let good = json!({"usage": {"input_tokens": 3}, "answers": {"next": {"choice": "b", "probabilities": {"a": 0.2, "b": 0.8}}}});
        let choice = read_answer(&good, &packed).unwrap();
        assert_eq!(choice.choice, "b");
        assert!((choice.margin - 0.6).abs() < 1e-9);
        let mut wrong_count = good.clone();
        wrong_count["usage"]["input_tokens"] = json!(4);
        assert!(read_answer(&wrong_count, &packed).is_err());
        let mut unknown = good.clone();
        unknown["answers"]["next"]["choice"] = json!("c");
        assert!(read_answer(&unknown, &packed).is_err());
    }
}
