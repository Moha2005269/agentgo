# Observation size on live sites

Measured 2026-10-06 by `tests/integration/site_render_smoke.js --write` on the
development machine, AgentGo 0.1.0. One run, live network, so numbers move with
the sites; treat them as an order of magnitude, not a benchmark result.

What is compared: the characters an agent must read to know what it can do on
the page. `HTML` is `document.documentElement.outerHTML`; `innerText` is the
visible text; `outline` is the first page (up to 400 lines) of `observe.page`
(headings plus interactive elements with references). Tokens are roughly
characters / 4.

| Site | DOM nodes | HTML chars | innerText chars | outline chars | refs | HTML ÷ outline | load+observe ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| https://example.com/ | 22 | 1974 | 917 | 21 | 1 | 94× | 1537 |
| https://www.google.com/ | 468 | 248859 | 138 | 491 | 19 | 507× | 952 |
| https://github.com/ | 1784 | 577686 | 5532 | 4008 | 102 | 144× | 1431 |
| https://developer.mozilla.org/en-US/ | 736 | 75665 | 5018 | 3362 | 87 | 23× | 896 |
| https://www.w3.org/ | 377 | 51257 | 3333 | 1892 | 47 | 27× | 1397 |
| https://en.wikipedia.org/wiki/Web_browser | 4167 | 593264 | 20359 | 10023 | 389 of 522 | 59× | 938 |
| https://www.rust-lang.org/ | 242 | 18271 | 3485 | 1503 | 37 | 12× | 988 |
| https://www.chromium.org/Home/ | 137 | 15036 | 2128 | 1571 | 48 | 10× | 1284 |
| https://news.ycombinator.com/ | 821 | 34799 | 4154 | 6422 | 231 | 5× | 1537 |
| https://stackoverflow.com/questions | 42 | 27312 | 265 | 142 | 2 | 192× | 5381 |

Total: HTML 1644123 chars (~411031 tokens) vs outline 29435 chars
(~7359 tokens): **55.9× smaller** over 10 sites.
Human-verification challenges met during the run: 0.

## What this does and does not show

- It shows the size of AgentGo's default observation against the raw page.
- It is **not** the A/B/C0/C1 experiment of the graduation roadmap: no agent
  solved tasks here, no success rate was measured, and Chrome + browser-use was
  not run. Browser-use also reduces the DOM before sending it to a model, so
  the fair token comparison is that later controlled experiment.
- After the first observation an action returns only a delta, which is where
  most multi-step savings come from; that is measured per task, not here.
