"""Redraws agentgo/assets/bench-agents.svg (the README's four-browser chart) from
a comparison run, in the 0.1.5 chart's design.
python v016/make_bench_chart.py <label suffix>"""
import json
import statistics as st
import sys
from pathlib import Path

BENCH = Path(__file__).resolve().parent.parent
OUT = Path(r"C:\Users\m7md0\agentgo\assets\bench-agents.svg")
S = sys.argv[1]
SETUPS = [("AgentGo 0.1.6", f"cmp-solo016-{S}", "#ff6a2b", True), ("AgentGo + GoMini", f"cmp-gm016v6-{S}", "#ffb36b", True),
          ("Playwright MCP", f"cmp-playwright-{S}", "#8d877f", False), ("browser-use", f"cmp-browseruse-{S}", "#5e5852", False)]


def load(label):
    return [json.loads(l) for l in (BENCH / "results" / "v016" / label / "runs.jsonl").open(encoding="utf-8")]


def inp(r):
    u = r["usage"] or {}
    return (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0)


F = json.loads((BENCH / "results" / "v016" / "final-37.json").read_text())
NAMES = {"AgentGo 0.1.6": "AgentGo 0.1.6", "AgentGo + GoMini": "AgentGo 0.1.6 + GoMini v6",
         "Playwright MCP": "Playwright MCP 0.0.83", "browser-use": "browser-use 0.13.11"}
stats = {n: {"solved": 100 * F[NAMES[n]]["solved"] / 37, "median": F[NAMES[n]]["median"],
             "tokens": F[NAMES[n]]["tokens"] / 1000, "turns": F[NAMES[n]]["turns"], "ok": F[NAMES[n]]["solved"]} for n in NAMES}
SETUPS = [("AgentGo + GoMini", "", "#ffb36b", True), ("AgentGo 0.1.6", "", "#ff6a2b", True),
          ("browser-use", "", "#5e5852", False), ("Playwright MCP", "", "#8d877f", False)]

parts = ["""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 640" width="1200" height="640" role="img" aria-label="AgentGo agent benchmark: same model, same 37 tasks, four browsers">
<title>AgentGo agent benchmark: same model, same 37 tasks, four browsers</title>
<defs>
<radialGradient id="glow" cx="12%" cy="0%" r="70%"><stop offset="0" stop-color="#ff6a2b" stop-opacity=".22"/><stop offset="1" stop-color="#ff6a2b" stop-opacity="0"/></radialGradient>
<pattern id="grid" width="36" height="36" patternUnits="userSpaceOnUse"><path d="M36 0H0V36" fill="none" stroke="#fff" stroke-opacity=".035"/></pattern>
</defs>
<style>
text{font-family:'Segoe UI',Inter,system-ui,-apple-system,Helvetica,Arial,sans-serif}
.grow{transform-box:fill-box;transform-origin:left center;animation:grow 1.4s cubic-bezier(.22,.8,.2,1) both}
.fade{animation:fade .8s ease both}
@keyframes grow{from{transform:scaleX(0)}to{transform:scaleX(1)}}
@keyframes fade{from{opacity:0;transform:translateY(6px)}to{opacity:1;transform:none}}
@media (prefers-reduced-motion:reduce){.grow,.fade{animation:none}}
</style>
<rect width="1200" height="640" rx="24" fill="#0e0d0c"/>
<rect width="1200" height="640" rx="24" fill="url(#glow)"/>
<rect width="1200" height="640" rx="24" fill="url(#grid)"/>
<rect x=".5" y=".5" width="1199" height="639" rx="23.5" fill="none" stroke="#fff" stroke-opacity=".08"/>
<text x="56" y="62" fill="#ff6a2b" font-size="13" font-weight="700" letter-spacing="2.4">AGENT BENCHMARK · MINIWOB++ AND SINGLE-PAGE APPS</text><text x="56" y="104" fill="#f4f0e8" font-size="32" font-weight="600" letter-spacing="-.5">Same model, same 37 tasks, four browsers</text><text x="56" y="134" fill="#a39a8e" font-size="15">gpt-6-luna drives each browser through its own MCP tools · one fresh session per task · all four at the same time · the page decides</text>"""]
panels = [((56, 200), "Tasks solved", "higher is better", "solved", lambda v, s: f"{s['ok']}/37", 100.0),
          ((616, 200), "Median time per task", "lower is better", "median", lambda v, s: f"{v:.0f} s", None),
          ((56, 400), "Input tokens per task", "lower is better", "tokens", lambda v, s: f"{v:.0f}k", None),
          ((616, 400), "Agent turns per task", "lower is better", "turns", lambda v, s: f"{v:.1f}", None)]
delay = 0.0
for (x, y), title, hint, key, fmt, top in panels:
    scale = top or max(stats[n][key] for n, *_ in SETUPS)
    parts.append(f'<text x="{x}" y="{y}" fill="#f4f0e8" font-size="17" font-weight="600">{title}</text>')
    parts.append(f'<text x="{x + 520}" y="{y}" fill="#6f675e" font-size="12" text-anchor="end">{hint}</text>')
    for i, (name, _, color, ours) in enumerate(SETUPS):
        ty = y + 37 + 34 * i
        v = stats[name][key]
        w = "700" if ours else "400"
        parts.append(f'<text x="{x}" y="{ty}" fill="{"#f4f0e8" if ours else "#a39a8e"}" font-size="14" font-weight="{w}">{name}</text>')
        parts.append(f'<rect x="{x + 150}" y="{ty - 12}" width="300" height="16" rx="5" fill="#fff" fill-opacity=".04"/>')
        parts.append(f'<rect class="grow" style="animation-delay:{delay:.2f}s" x="{x + 150}" y="{ty - 12}" width="{max(2, 300 * v / scale):.1f}" height="16" rx="5" fill="{color}"/>')
        parts.append(f'<text class="fade" style="animation-delay:{delay + 0.6:.2f}s" x="{x + 520}" y="{ty + 1}" fill="#f4f0e8" font-size="14" font-weight="{w}" text-anchor="end">{fmt(v, stats[name])}</text>')
        delay += 0.12
parts.append('<text x="56" y="612" fill="#6f675e" font-size="12">AgentGo 0.1.6 (GoMini v6) · Playwright MCP 0.0.83 · browser-use 0.13.11 · 23 MiniWoB++ tasks (seeds 41/51) + 14 single-page-app tasks · Windows 11, October 2026 · docs/benchmarks/agentgo-0.1.6.md</text>')
parts.append("</svg>\n")
OUT.write_text("\n".join(parts), encoding="utf-8")
print(json.dumps({n: {k: round(v, 1) for k, v in s.items()} for n, s in stats.items()}, indent=1))
