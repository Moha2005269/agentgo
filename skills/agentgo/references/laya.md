# Laya: the local choice model, and how to talk to it

Status: **planned, not available.** The browser is complete without it. The
person switches it on in Settings ("Laya, the local choice model"); off is the
default and means the agent's own model decides everything. `decision.status`
reports the switch and whether a model is installed (today: never).

This file records the plan so it is not lost, and fixes the one thing that can
be fixed before the model exists: what goes in and what comes out.

## Why

A large model that drives a browser reads a page and chooses one step, again
and again. Most of those choices are small ("which of these twelve controls is
the Search button"). Laya is a small model (the starting candidates are the
open Laya 322M browser checkpoints) that runs on the person's computer and
makes those small choices. The aim: fewer tokens for the large model, faster
tasks, fewer mistakes. Whether that holds is what the graduation experiment
measures:

| Configuration | Browser | Local model | Planner |
|---|---|---|---|
| A | Chrome with browser-use | the original checkpoint | fixed |
| B | AgentGo | none | fixed |
| C0 | AgentGo | the same original checkpoint, through this interface | fixed |
| C1 | AgentGo | the checkpoint fine-tuned on AgentGo data | fixed |

A against C0 compares the browsers. B against C0 shows what the local model
adds. C0 against C1 shows what fine-tuning adds. The model still has to be
fine-tuned on AgentGo's own records; until then nothing here is a result.

## Who does what

- **The external agent plans.** It knows the task, writes all text, and hands
  over one small subgoal at a time.
- **Laya chooses.** Given a subgoal and the steps that are possible right now,
  it names one. It never writes text and never sees the whole task.
- **AgentGo acts.** It runs the chosen step, waits, applies the person's
  policy, checks the result, and reports. A choice cannot bypass an approval,
  a blocked account or a human check.

## What Laya is fed: the decision record

`decision.record {subgoal, constraints?, values?, completion?}` builds it for
the current tab. Version `agentgo.laya.decision/0`:

| Part | Content | Why it is small |
|---|---|---|
| `subgoal` | one step in plain words, at most 600 characters | the model never sees the whole task |
| `constraints` | things that must hold ("do not submit", "stay on this site") | a short list |
| `values` | the exact texts that may be typed, each with a name | the model picks a name; it cannot invent text |
| `completion` | how to tell the subgoal is done | lets the model answer `done` |
| `state` | `url`, `title`, `revision`, and the compact `outline` | the outline, not the HTML |
| `candidates` | every possible step: `{id, op, ref, target}` | the model chooses among a few dozen ids |
| `recent` | the last few steps and whether they worked | so a failed step is not repeated |

Candidates are built by the browser, not by the model. Each actionable outline
line becomes one: `fill` for text fields, `select` for lists, `check` for
checkboxes and radios, `click` for the rest; disabled controls are left out.
Six more are always there: `scroll_down`, `scroll_up`, `back`, `wait`, `done`
and `escalate`.

The answer is one object: `{"choose": "r12", "value": "email", "confidence": 0.8}`.
`value` names one of the supplied values and is needed for `fill` and `select`
only.

`decision.apply {choose, value?}` runs it. It refuses when the page is no
longer at the `revision` the record was built for (`target.stale`: build a new
record), when the id was not offered, or when the value name was not supplied.
`done` and `escalate` run nothing and return `control`.

## How to write for Laya (and for yourself)

These are the rules the planner follows when it delegates, with or without the
model. They are what makes the input small.

1. **One step per subgoal.** "Open the sign-in form", not "sign in and check
   the orders". If you would need the word "then", split it.
2. **Name the thing as the page names it.** Use the words a person sees
   ("the Search button", "the Email field"), not selectors or positions.
3. **Supply every text.** Anything to be typed goes in `values` with a clear
   name: `{"email": "...", "city": "Cairo"}`. If the text depends on what the
   page shows, read the page first and supply it then.
4. **Say when it is done.** `completion: "the page shows the order number"`.
   Without it the chooser cannot stop.
5. **State what must not happen.** Constraints are cheap and prevent the
   costly mistakes: "do not submit the form", "do not leave this site".
6. **Take `escalate` seriously.** It means no candidate fits. Look at the
   page yourself (`observe.page`), then give a better subgoal. Do not repeat
   the same one.
7. **Do not send history.** The record already carries the recent outcomes.
   Sending your reasoning costs tokens and helps nobody.

Example:

```json
{"method": "decision.record", "params": {
  "subgoal": "put the email address into the Email field",
  "values": {"email": "ada@example.com"},
  "constraints": ["do not submit the form"],
  "completion": "the Email field contains the address"}}
```

## What is still to do

- Build the local worker that loads a checkpoint and answers records. Keep the
  browser usable without it.
- Measure candidate coverage: how often the right step is missing from the
  candidates. That is a browser fault, not a model fault.
- Add delegation: the agent hands over a subgoal and gets back a state
  (`accepted`, `running`, `completed`, `failed`, `decision required`,
  `paused`, `cancelled`, `human handoff required`) while Laya makes several
  choices in a row.
- Collect training records (state, all candidates, the chosen one, the
  outcome, later corrections), fine-tune, and validate before any claim.
- Calibrate confidence on validation data; raw confidence is not trusted.
- Freeze this record before training starts. Any change after that is a new
  version.
