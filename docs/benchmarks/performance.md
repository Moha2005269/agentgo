# Performance of the common operations

Measured 2026-10-06 by `scripts/dev/bench.js --write` on the development
machine, **debug build**, private desktop (window covered, never active), local
fixture page `tests/fixtures/pages/agent.html`. Medians. One machine and one
run: use these as the order of magnitude and as a regression check, not as a
claim about other hardware.

| Operation | Median | Budget |
|---|---:|---:|
| Start to first answered command | 585 ms | 6000 ms |
| navigate.url, loaded and observed | 171 ms | 1500 ms |
| observe.page | 12 ms | 150 ms |
| act.click, settled, with change set | 395 ms | 900 ms |
| act.fill, settled, with change set | 382 ms | 900 ms |
| observe.screenshot (JPEG) | 159 ms | 1200 ms |
| Memory, all processes, one tab | 726 MB | 1200 MB |

An action waits until the page has been quiet for about 250 ms before it
reports what changed, so roughly that much of every action time is deliberate.

## What the agent has to read

| Representation of the same page | Characters | About tokens |
|---|---:|---:|
| Full HTML | 4701 | 1175 |
| AgentGo outline (`observe.page`) | 571 | 143 |
| Whole reply to an action (change set) | 135 | 34 |
| One JPEG screenshot, base64 | 156844 | image input, typically 1000 or more |

Token counts are characters / 4, an estimate. The comparison with Chrome plus
browser-use on real tasks (success rate, turns, tokens) is the A/B/C experiment
of the graduation roadmap and is **not** measured here.
