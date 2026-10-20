# GoMini: the local model, and how to hand it work

GoMini is a small model (321.7M parameters) that runs on the person's
computer. The person downloads it in AgentGo Setup or in Settings > GoMini
and switches it on there; an agent cannot. `decision.status` (or
`gomini.status`) says whether it is on: `active: true`.

When it is on, you plan and GoMini takes the routine steps. You stop reading
every page and choosing every click: one `gomini.delegate` (MCP:
`browser_delegate`) replaces a run of `browser_read` + `browser_act` turns,
and GoMini decides each step in milliseconds on this computer.

## Who does what

- **You plan.** You know the task, write every text, and hand over one small
  subgoal at a time.
- **GoMini chooses.** Given the subgoal and the steps the page offers now, it
  names one: fill this field with that supplied value, click, check, select,
  scroll, wait, done, or ask you. It never writes text, never invents a
  value and never sees the whole task.
- **AgentGo acts and checks.** It runs the choice, reads typed fields back,
  applies the person's policy, and stops on Pause or Stop. A choice can never
  bypass an approval, a blocked account or a human check.

## Delegating

```json
{"method": "gomini.delegate", "params": {
  "subgoal": "Fill this travel form using Primary City=city; Postal code=postal. Save draft when all required fields match. Never publish.",
  "values": {"city": "Maple-3321", "postal": "11511"},
  "completion": "Draft saved successfully with every requested field value",
  "constraints": ["Never publish. Never invent a missing value."],
  "maxSteps": 8}}
```

The answer:

| `status` | Means | You do |
|---|---|---|
| `done` | GoMini says the completion holds (`verified: false`). It is only reported after at least one step ran, or when every supplied value is in its field | Glance at `evidence.status` (the page's status lines); usually enough. |
| `needs_help` | `reason` says why: nothing fits, a step for you, no progress, too big, or "the step ran but the page did not visibly change" (check whether that already did the job) | Read the page yourself and give a narrower subgoal, or do the step. |
| `stopped` | The person pressed Stop, or the budget ran out | Ask the person, or start over. |

When the values are in place and what is left is yours (send, submit, sign
in), `done` says which control with its reference: `reason: "... [106]
clickable "(icon: send reply)" ... is yours"`. Press it with `browser_act`; no
read needed. A subgoal that ends with such a step ("... and send it") gets
the same answer.

While a supplied value is not in its field yet, the browser takes GoMini's
best step that fills it before any click (Search, Submit) or "done". Name the
field for each value (`Primary City=city`, or "fill the City field with
city=city"): a value is then only offered for the field you named.

Boxes are ticked, never cleared by choosing them again; say "uncheck" or
"clear" when a box should go off. Name boxes in quotes ("Check "Hhc" and
"4jU"") and only those are ticked, and the delegation is done once they all
are. A target hidden under the control that opens it (the "+" of a closed
menu) is reached by pressing that control first.

`steps` lists every choice GoMini made, in order (a failed step says why; a
step left to you is `{"choice": ..., "held": true}`), so you can see what
happened without reading the page again. `evidence.status`
holds the page's visible status lines; on `needs_help` there is also
`evidence.outline`, the start of the page as it is now.

## What always comes back to you

Steps whose target names paying, buying, deleting, sending, publishing,
submitting, checking out, confirming, signing in or out, passwords, one-time
codes or CAPTCHAs are never run by GoMini: the delegation returns
`needs_help` and you decide. The browser's own approvals and hand-offs apply
on top (`approval.required`, `handoff.required`: `browser_wait_human`).
GoMini also stops if the tab leaves the site the subgoal started on.

## How to write a subgoal (for GoMini, and for yourself)

1. **One step per subgoal.** "Fill the shipping form and save the draft", not
   "check out". If you would need the word "then" across pages, split it.
2. **Name things the way the page does.** "the Postal code field", "the Save
   draft button". GoMini matches your words to the page's controls; a page
   has more controls than it takes (24 choices), so it keeps the ones your
   words name.
3. **Supply every text.** Anything to be typed goes in `values` with a clear
   name, and say which field gets which value: "Primary City=city".
4. **Say when it is done.** `completion: "the page shows the order number"`.
5. **State what must not happen.** Constraints are cheap: "never publish",
   "stay on this site".
6. **Take `needs_help` seriously.** Do not repeat the same subgoal; look at
   the page (`observe.page`) and narrow it.

## Without GoMini

When it is off, `gomini.delegate` answers `capability.unsupported` and you
work as usual. You can still use the decision record yourself to work in
fewer tokens: `decision.record {subgoal, values}` returns the possible steps
as candidates, `decision.apply {choose, value}` runs one (only at the page
revision it was offered for).

## The decision record

`decision.record {subgoal, constraints?, values?, completion?}` builds it for
the current tab. Version `agentgo.gomini.decision/1`:

| Part | Content |
|---|---|
| `subgoal` | one step in plain words, at most 600 characters |
| `constraints` | things that must hold |
| `values` | the exact texts that may be typed, each with a name (up to 20) |
| `completion` | how to tell the subgoal is done |
| `state` | `url`, `title`, `revision`, the compact `outline`, and `status`: visible status, alert and live-region lines |
| `candidates` | every possible step: `{id, op, ref, target}`, plus `scroll_down`, `scroll_up`, `back`, `wait`, `done`, `escalate` |
| `recent` | your own session's last few commands (never another agent's) |

## Limits worth knowing

GoMini was trained and checked on AgentGo's own records of forms, single
actions, reveal-without-click and navigation; general browsing on arbitrary
sites is not proven, and its probabilities are not calibrated. That is why
every choice is checked independently and `done` is a claim, not a proof.
Element-geometry questions are a known weak spot: ask the browser directly
(`runtime.get_rect`). Model and runtime: `GoMini-v5-Q8_0` on llama.cpp
(`b11559`), on the graphics card through Vulkan when it can, else the
processor; nothing leaves the computer.
