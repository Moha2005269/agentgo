# AgentGo 0.1.5 benchmarks

Everything here compares AgentGo 0.1.4 with 0.1.5 on the same machine
(Windows 11, Ryzen, RX 7800 XT), home connection, 2026-10-20 and 21. Pages are
BrowserGym's MiniWoB++ tasks (browsergym-miniwob 0.14.3), served locally with
BrowserGym's own start-up (seeded problem, display removed, long timer); a
task's reward is the page's own. GoMini runs on the GPU through Vulkan.

## 1. What the agent sees (130 MiniWoB++ pages, no model)

For every MiniWoB++ page, the clickable elements a person can use (native
controls plus elements with their own click/mousedown listener, from
Chromium's DevTools) are compared with AgentGo's outline after
`browser_open`.

| | 0.1.4 | 0.1.5 |
|---|---:|---:|
| Pages | 130 | 130 |
| First outline is the empty starting page | 130 | 0 |
| Pages with no control in the outline | 13 | 0 |
| Controls listed (DevTools finds 749 elements with a listener or native role) | 496 | 914 |
| Named elements DevTools finds that the outline does not list | 128 | 77 |
| `browser_open` time, mean / median (ms) | 2,248 / 2,124 | 2,673 / 2,294 |

0.1.4 answered `browser_open` before the page's load handlers ran, so every
first outline was MiniWoB++'s empty starting page. 0.1.5 waits until the page
is still (at most 0.8 s more), which is most of the extra 170 ms at the
median. The outline lists more lines than DevTools' count because a row and
the parts of it a person can press separately (an email's sender, subject
and icons) are each listed. Most of the 77 elements left are words inside a
listed row that the row's own click handles (DevTools reports a listener on
each word); the rest are mostly drag handles and canvas parts.

## 2. Setting values (one test page, 25 field kinds)

Each field gets one `act.fill`, `act.select`, `act.check` or `act.type`; the
page records the value it saw and the events that fired.

| | 0.1.4 | 0.1.5 |
|---|---:|---:|
| Cases passed: 24 field kinds (text, password, email, number, tel, url, search, date, time, datetime-local, month, week, colour, range, textarea, select lists, checkbox, radio buttons, a script-controlled input and others) and typing into a terminal-like box that listens for keys | 15 / 25 | **25 / 25** |

Date, time, month, week, colour and range fields took no value from
`act.fill` in 0.1.4; `act.type` ignored `ref` and sent no key events.

## 3. Real websites (20 sites, first visit, read-only)

`browser_open`, then `browser_read` 1.5 s later. No sign-in, nothing
submitted.

| Site | Controls in the `browser_open` reply, 0.1.4 | 0.1.5 | Open time 0.1.4 (ms) | 0.1.5 (ms) |
|---|---:|---:|---:|---:|
| en.wikipedia.org | 146 | 146 | 3,216 | 3,057 |
| bbc.com | 112 | 112 | 3,469 | 3,214 |
| github.com | 134 | 135 | 4,715 | 4,562 |
| news.ycombinator.com | 150 | 150 | 3,738 | 3,255 |
| reddit.com | 131 | 139 | 4,958 | 4,740 |
| stackoverflow.com | **0** | 132 | 4,519 | 4,476 |
| amazon.com (search) | 4 | 4 | 3,179 | 2,655 |
| booking.com | **0** | 117 | 3,109 | 3,917 |
| youtube.com | 34 | 35 | 7,014 | 5,324 |
| nytimes.com | 144 | 146 | 4,322 | 5,522 |
| imdb.com (Top 250) | **0** | 149 | 4,079 | 10,175 |
| developer.mozilla.org | 144 | 142 | 4,782 | 4,490 |
| w3schools.com | 142 | 142 | 3,341 | 3,034 |
| espn.com | 125 | 114 | 7,154 | 4,661 |
| weather.gov | 70 | 70 | 5,151 | 5,772 |
| craigslist.org | 133 | 149 | 8,603 | 4,271 |
| etsy.com | **0** | human check handed to the person | 3,790 | 3,149 |
| aljazeera.com | 121 | 125 | 3,489 | 2,888 |
| huggingface.co | 131 | 131 | 3,884 | 3,540 |
| google.com (search) | human check | 78 | 3,622 | 4,299 |

0.1.4 answered four first visits with an empty page: the one a browser check
(Cloudflare, AWS WAF, DataDome) left behind. 0.1.5 reads the real page once
the check passes, and hands a check that stays (Etsy's DataDome captcha) to
the person. Amazon's search page shows 4 controls in both versions: Amazon
answered this connection with a short block page (236 characters of text). Median open
time: 3,981 ms (0.1.4), 4,285 ms (0.1.5), most of the difference being the
checks 0.1.5 now waits out (IMDb). Unnamed controls on these 20 pages: 68
(0.1.4), 26 (0.1.5).

## 4. GoMini delegations (13 held-out MiniWoB++ task types x 5 seeds)

Each task's instruction becomes one `browser_delegate` subgoal, written the
way `references/gomini.md` asks. A final Submit is the planner's, so the
script presses it after a delegation that says done. The page's reward says
whether the delegation did its part. None of these pages were used to train
GoMini.

| Task | 0.1.4 + GoMini v4 | 0.1.5 + GoMini v5 |
|---|---:|---:|
| click-button | 4/5 | 4/5 |
| click-link | 5/5 | 4/5 |
| click-option | 0/5 | 4/5 |
| click-checkboxes | 0/5 | 5/5 |
| click-tab-2 | 2/5 | 2/5 |
| enter-text | 0/5 | 5/5 |
| focus-text | 0/5 | 0/5 |
| choose-list | 0/5 | 5/5 |
| enter-date | 0/5 | 5/5 |
| navigate-tree | 4/5 | 5/5 |
| click-collapsible | 0/5 | 0/5 |
| social-media | 0/5 | 0/5 |
| form-sequence-2 | 0/5 | 0/5 |
| **Solved** | **15/65** | **39/65** |
| False "done" (GoMini said done, the page did not count it) | 26 | **3** |
| Median time per delegation (ms) | 2,236 | 2,649 |

Where v5 still fails: it says it has no step for "focus the text box"
(focus-text) and for opening a collapsed section (click-collapsible), kinds
of step it was never trained on; on form-sequence-2 it does not map "the 3rd
radio button" to `radio "3 of 3"`; on click-tab-2 it clicked a wrong link in
3 of 5 seeds (2 of its 3 false "done"); social-media's icons ignore a real
mouse click on this page. The third false "done" is click-button seed 201: it
pressed a "No" button and the page scored -1.

## 5. An agent using AgentGo (Claude Opus 5.5)

One agent (Claude Opus 5.5 in Claude Code) solved MiniWoB++ tasks through
AgentGo, alone and with GoMini, and its tokens were taken from Claude Code's
own usage records. 12 tasks from the 0.1.4 benchmark with new seeds, and 12
tasks it had never opened.

Measured on the 0.1.5 build before the last round of fixes (see "Limits").
Tokens are what the tool calls and replies added to the agent's context per
task (Claude Code's usage records); browser output is the characters the
browser tools returned.

| | AgentGo 0.1.4 | AgentGo 0.1.5 | AgentGo 0.1.4 + GoMini v4 | AgentGo 0.1.5 + GoMini v5 |
|---|---|---|---|---|
| **12 tasks from the 0.1.4 run (new seeds)** | | | | |
| Solved | 10/12 | 11/12 | 10/12 | 10/12 |
| Mean reward | 0.83 | 0.92 | 0.83 | 0.90 |
| Agent turns per task | 5.4 | 4.8 | 6.9 | 3.7 |
| Seconds per task (median) | 23.9 | 23.2 | 31.0 | 25.8 |
| Tokens added per task | 4,914 | 3,607 | 3,375 | 2,409 |
| Browser output per task (characters) | 10,021 | 3,625 | 2,630 | 1,673 |
| **12 tasks never opened before** | | | | |
| Solved | — | 12/12 | — | 11/12 |
| Agent turns per task | — | 1.7 | — | 2.1 |
| Seconds per task (median) | — | 10.1 | — | 11.0 |
| Tokens added per task | — | 1,218 | — | 1,455 |

The tasks that failed:

- AgentGo, email-inbox (delete the email from Donelle): the trash icon in
  Donelle's row was clicked and the page scored it -1 (being checked).
- AgentGo + GoMini, click-checkboxes-large (reward 0.75): GoMini chose a
  ticked box again, which cleared it (fixed: boxes are ticked, never cleared
  by choosing them again, and a ticked box is not offered).
- AgentGo + GoMini, social-media: the reply icon ignores a real mouse click
  on this page; the same happened with 0.1.4.
- AgentGo + GoMini, copy-paste-2: the outline trimmed the trailing space of
  the text to copy, so the pasted text differed (fixed: values are shown
  exactly).

On the never-seen tasks the agent mostly acted without GoMini (one call
fills a form); where it delegated, GoMini looped on two check boxes (fixed:
boxes are ticked, never cleared by choosing them again) and skipped the "+"
of a closed pie menu (fixed: a covering control that opens the target is
pressed first).


## Limits

- One machine, one connection, one run per seed. Differences of one task
  move a rate by 8 points (12 tasks) or 1.5 points (65 delegations).
- The agent-driven runs are a single agent session; the agent learns as it
  goes, so the 12 never-seen tasks are the cleaner measure. They ran on the
  0.1.5 build before its last fixes (found by those runs); sections 1 to 4
  were measured on the final build.
- In the agent-driven runs, one AgentGo task (email-inbox) had the session's
  context compaction and a usage-limit pause inside it; its time and tokens
  are left out of the means.
- MiniWoB++ pages are small; real sites are larger and slower. Section 3 is
  the real-web check.

## Known, not fixed in 0.1.5

- An icon whose page binds its click listener after the page has settled is
  found on the next read, not in the `browser_open` reply, and then shows
  once as added (`+`) in a change set.
- Menu items stacked under the control that opens them (a closed pie menu)
  are listed before they are revealed; clicking one answers
  `target.blocked` naming the opener (GoMini presses the opener first).
