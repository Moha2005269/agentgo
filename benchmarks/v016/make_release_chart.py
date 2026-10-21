"""Draws agentgo/assets/bench-release.svg for 0.1.5 -> 0.1.6 in the 0.1.5
chart's card design."""
from pathlib import Path

OUT = Path(r"C:\Users\m7md0\agentgo\assets\bench-release.svg")
CARDS = [
    ("Wrong item after a list re-render", "0/4", "4/4", "React key={index} inbox, right notification"),
    ("Input tokens, 300-row virtual list", "249k", "59k", "per task, AgentGo alone"),
    ("Wrong actions in the agent benchmark", "6", "0", "76 runs with GoMini, 38 tasks"),
    ("Calls to reach an item in a long list", "32", "1", "browser_act scroll until a text shows"),
    ("GoMini subgoals finished", "39", "53", "of 65 held-out MiniWoB++ subgoals"),
    ("GoMini right step, held-out pages", "76.6%", "96.6%", "GoMini v5 to v6"),
]
parts = ["""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 540" width="1200" height="540" role="img" aria-label="AgentGo 0.1.5 to 0.1.6: what the benchmark fixed">
<title>AgentGo 0.1.5 to 0.1.6: what the benchmark fixed</title>
<defs>
<radialGradient id="glow" cx="12%" cy="0%" r="70%"><stop offset="0" stop-color="#ff6a2b" stop-opacity=".22"/><stop offset="1" stop-color="#ff6a2b" stop-opacity="0"/></radialGradient>
<pattern id="grid" width="36" height="36" patternUnits="userSpaceOnUse"><path d="M36 0H0V36" fill="none" stroke="#fff" stroke-opacity=".035"/></pattern>
</defs>
<style>
text{font-family:'Segoe UI',Inter,system-ui,-apple-system,Helvetica,Arial,sans-serif}
.fade{animation:fade .8s ease both}
@keyframes fade{from{opacity:0;transform:translateY(6px)}to{opacity:1;transform:none}}
@media (prefers-reduced-motion:reduce){.fade{animation:none}}
</style>
<rect width="1200" height="540" rx="24" fill="#0e0d0c"/>
<rect width="1200" height="540" rx="24" fill="url(#glow)"/>
<rect width="1200" height="540" rx="24" fill="url(#grid)"/>
<rect x=".5" y=".5" width="1199" height="539" rx="23.5" fill="none" stroke="#fff" stroke-opacity=".08"/>
<text x="56" y="62" fill="#ff6a2b" font-size="13" font-weight="700" letter-spacing="2.4">AGENTGO 0.1.5 → 0.1.6</text><text x="56" y="104" fill="#f4f0e8" font-size="32" font-weight="600" letter-spacing="-.5">What the benchmark fixed</text><text x="56" y="134" fill="#a39a8e" font-size="15">Every number measured on the same tasks before and after · docs/benchmarks/agentgo-0.1.6.md</text>"""]
for i, (title, old, new, note) in enumerate(CARDS):
    x = 56 + 368 * (i % 3)
    y = 186 + 170 * (i // 3)
    tx = x + 22
    arrow = tx + int(17.5 * len(old)) + 14
    parts.append(f'<g class="fade" style="animation-delay:{0.10 + 0.12 * i:.2f}s"><rect x="{x}" y="{y}" width="344" height="150" rx="16" fill="#1a1816" stroke="#fff" stroke-opacity=".07"/>'
                 f'<text x="{tx}" y="{y + 34}" fill="#a39a8e" font-size="14">{title}</text>'
                 f'<text x="{tx}" y="{y + 84}" fill="#6f675e" font-size="30" font-weight="600" text-decoration="line-through">{old}</text>'
                 f'<text x="{arrow}" y="{y + 84}" fill="#6f675e" font-size="26">→</text>'
                 f'<text x="{arrow + 38}" y="{y + 86}" fill="#ff6a2b" font-size="42" font-weight="700">{new}</text>'
                 f'<text x="{tx}" y="{y + 130}" fill="#6f675e" font-size="12">{note}</text></g>')
parts.append("</svg>\n")
OUT.write_text("\n".join(parts), encoding="utf-8")
print("release chart written")
