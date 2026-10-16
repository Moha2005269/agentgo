// One-off migration: rebuilds ui/index.html (the New Tab page) from the
// approved prototype, keeping its visual identity (tokens, typography, wings
// backdrop, emblem, omnibox, shortcut tiles) and dropping every simulated
// surface. Source: `git show HEAD:ui/index.html`-equivalent prototype passed
// as argv[2]. Output: ui/index.html + ui/newtab.js.
const fs = require("fs");
const path = require("path");
const src = fs.readFileSync(process.argv[2], "utf8").replace(/\r\n/g, "\n").split("\n");
const ui = path.resolve(__dirname, "../../ui");
const lines = (from, to) => src.slice(from - 1, to).join("\n");
const find = (needle, from = 0) => { const i = src.findIndex((l, n) => n >= from && l.includes(needle)); if (i < 0) throw new Error("missing " + needle); return i + 1; };

const cssStart = find("<style>") + 1;
const cssSecondary = find("/* ================= SECONDARY VIEWS");
const symStart = find('<svg xmlns="http://www.w3.org/2000/svg" style="display:none">');
const symEnd = find("</svg>", symStart);
const wingsStart = find("const wings = (() => {");
const wingsEnd = find("})();", wingsStart);
const emblemStart = find('<svg class="emblem"');
const emblemEnd = find("</svg>", emblemStart);

const css = lines(cssStart, cssSecondary - 1);
const symbols = lines(symStart, symEnd);
const wings = lines(wingsStart, wingsEnd);
const emblem = lines(emblemStart, emblemEnd);

const extraCss = `
/* ================= NEW TAB — LIVE SECTIONS ================= */
html,body{height:100%}
#app{height:100%;display:flex;flex-direction:column}
.home-top{justify-content:space-between;align-items:center;gap:16px}
.agent-chip{display:inline-flex;align-items:center;gap:9px;height:30px;padding:0 13px;border:1px solid var(--line);border-radius:var(--r-pill);background:color-mix(in srgb,var(--surface) 70%,transparent);backdrop-filter:blur(10px);font:500 10px var(--font-m);letter-spacing:.14em;text-transform:uppercase;color:var(--muted);transition:border-color .2s,color .2s}
.agent-chip:hover{color:var(--ink);border-color:color-mix(in srgb,var(--accent) 50%,transparent)}
.agent-chip i{width:7px;height:7px;border-radius:50%;background:var(--muted);flex:none}
.agent-chip.live i{background:var(--ok);box-shadow:0 0 12px color-mix(in srgb,var(--ok) 70%,transparent)}
.agent-chip.paused i{background:var(--accent2)}
.agent-chip.stopped i{background:var(--bad)}
.omni input{flex:1;min-width:0;height:100%;outline:none;font:400 16px var(--font-b);color:var(--ink)}
.omni input::placeholder{color:var(--muted)}
.hint{font:500 9.5px var(--font-m);letter-spacing:.2em;text-transform:uppercase;color:var(--muted)}
.hint kbd{font:inherit;color:var(--ink);padding:2px 6px;border:1px solid var(--line);border-radius:6px;margin:0 3px}
.block-h{display:flex;align-items:baseline;justify-content:space-between;gap:16px;margin-bottom:14px}
.block-h h2{font:400 30px/1.1 var(--font-e);letter-spacing:-.02em}
.block-h h2 em{color:var(--accent)}
.block-h span{font:500 9.5px var(--font-m);letter-spacing:.2em;text-transform:uppercase;color:var(--muted)}
.rows{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1px;background:var(--line2);border:1px solid var(--line2);border-radius:var(--r-lg);overflow:hidden}
.row{display:flex;align-items:center;gap:12px;padding:13px 16px;background:var(--surface);min-width:0;transition:background .15s}
.row:hover{background:var(--surface2)}
.row .site-mark{width:26px;height:26px;border-radius:8px}
.row img{width:16px;height:16px;border-radius:4px}
.row-t{min-width:0;flex:1}
.row-t b{display:block;font:600 13px var(--font-b);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.row-t span{display:block;font:400 11px var(--font-m);color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.row time{font:500 10px var(--font-m);color:var(--muted);flex:none}
.cards{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px}
.card{display:flex;flex-direction:column;gap:12px;padding:22px;background:var(--surface);border:1px solid var(--line2);border-radius:var(--r-xl);box-shadow:var(--shadow-sm);min-width:0}
.card h3{font:600 15px var(--font-b)}
.card p{color:var(--muted);font-size:13px;line-height:1.6}
.card .tag{font:600 9.5px var(--font-m);letter-spacing:.18em;text-transform:uppercase;color:var(--accent)}
.code{position:relative;margin-top:auto;padding:13px 44px 13px 14px;background:var(--code-bg);color:var(--code-fg);border-radius:var(--r-md);font:400 11.5px/1.6 var(--font-m);white-space:pre-wrap;word-break:break-all;border:1px solid var(--line2)}
.copy{position:absolute;top:8px;right:8px;width:28px;height:28px;display:grid;place-items:center;border-radius:8px;color:var(--muted);transition:.15s}
.copy:hover{color:var(--ink);background:rgba(255,255,255,.08)}
.copy.done{color:var(--ok)}
.split{display:grid;grid-template-columns:minmax(0,1.15fr) minmax(0,1fr);gap:14px}
.sees{padding:22px;background:var(--code-bg);border:1px solid var(--line2);border-radius:var(--r-xl);font:400 12px/1.75 var(--font-m);color:var(--code-fg);overflow:hidden}
.sees .k{color:var(--accent)}
.sees .d{color:#7C8494}
.sees .s{color:#9FD8A8}
.trust{display:flex;flex-direction:column;gap:14px;padding:24px;background:var(--surface);border:1px solid var(--line2);border-radius:var(--r-xl)}
.trust h3{font:400 26px/1.15 var(--font-e)}
.trust ul{list-style:none;display:flex;flex-direction:column;gap:11px}
.trust li{display:flex;gap:11px;align-items:flex-start;font-size:13.5px;color:var(--muted);line-height:1.5}
.trust li b{color:var(--ink);font-weight:600}
.trust .chk{width:20px;height:20px;flex:none;display:grid;place-items:center;border-radius:50%;background:color-mix(in srgb,var(--ok) 14%,transparent);color:var(--ok);margin-top:1px}
.feed{display:flex;flex-direction:column;border:1px solid var(--line2);border-radius:var(--r-lg);overflow:hidden;background:var(--surface)}
.feed-row{display:grid;grid-template-columns:70px 150px minmax(0,1fr) auto;gap:14px;align-items:center;padding:10px 16px;font:400 11.5px var(--font-m);border-bottom:1px solid var(--line2)}
.feed-row:last-child{border-bottom:0}
.feed-row time,.feed-row .who{color:var(--muted)}
.feed-row .what{color:var(--accent)}
.feed-row .sum{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.feed-row .bad{color:var(--bad)}
.foot{display:flex;flex-wrap:wrap;gap:10px 18px;justify-content:center;padding-top:8px;font:500 10px var(--font-m);letter-spacing:.14em;text-transform:uppercase;color:var(--muted)}
.foot b{color:var(--ink);font-weight:500}
[hidden]{display:none!important}
html.no-motion *,html.no-motion *::before,html.no-motion *::after{animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important}
@media (max-width:900px){.cards,.split,.rows{grid-template-columns:1fr}.feed-row{grid-template-columns:60px minmax(0,1fr)}.feed-row .who,.feed-row .ok{display:none}}
@media (prefers-reduced-motion:reduce){*,*::before,*::after{animation-duration:.01ms!important;transition-duration:.01ms!important}}
`;

const html = `<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>New Tab — AgentGo</title>
<style>
${css}
${extraCss}
</style>
</head>
<body>
${symbols}
<div id="app">
  <main class="workspace">
    <section class="viewport" id="viewport">
      <div class="zoomwrap" id="zoomwrap">
        <section class="view view-home" id="view-home">
          <div class="home-fx" id="homeFx" aria-hidden="true">
            <div class="wing-glow"></div>
            <canvas id="wings"></canvas>
            <canvas id="dust"></canvas>
            <div class="spot" id="spot"></div>
            <div class="vignette"></div>
            <div class="grain"></div>
          </div>

          <div class="home">
            <header class="home-top rise" style="--i:0">
              <div class="greet"><span id="greetDay"></span> · <span id="greetTime"></span> · <b id="greetWord"></b></div>
              <a class="agent-chip" id="agentChip" href="#connect"><i></i><span id="agentChipText">Checking agent link…</span></a>
            </header>

            <div class="hero">
${emblem}
              <h1 class="headline-reveal"><span class="ln"><i>The browser your</i></span><span class="ln"><i><span class="hl">agent</span> drives.</i></span></h1>
              <p class="lede rise" style="--i:3">A real Chromium browser with one local control protocol. Any agent — Claude Code, Codex, OpenCode — reads pages as compact outlines and acts with real clicks and keys. <b>You watch, approve, and take over whenever you want.</b></p>

              <form class="omni rise" style="--i:4" id="omniForm" role="search">
                <svg class="ic omni-ic"><use href="#i-search"/></svg>
                <input id="omniInput" name="q" aria-label="Search Google or enter an address" placeholder="Search Google or enter an address" autocomplete="off" spellcheck="false" autofocus>
                <button class="go-btn" type="submit" title="Go" aria-label="Go"><svg class="ic"><use href="#i-arrow"/></svg></button>
              </form>
              <p class="hint rise" style="--i:5"><kbd>Enter</kbd> searches Google · an address opens directly</p>
            </div>

            <section class="shortcuts rise" style="--i:6" id="shortcuts" aria-label="Shortcuts"></section>

            <section class="rise" style="--i:7" id="bookmarksBlock" hidden>
              <div class="block-h"><h2>Bookmarks</h2><span id="bookmarksCount"></span></div>
              <div class="rows" id="bookmarkRows"></div>
            </section>

            <section class="rise" style="--i:7" id="recentBlock" hidden>
              <div class="block-h"><h2>Recently <em>visited</em></h2><span>from this profile</span></div>
              <div class="rows" id="recentRows"></div>
            </section>

            <section class="rise" style="--i:8" id="activityBlock" hidden>
              <div class="block-h"><h2>Agent <em>activity</em></h2><span>live · from the local audit log</span></div>
              <div class="feed" id="activityRows"></div>
            </section>

            <section class="rise" style="--i:8" id="connect">
              <div class="block-h"><h2>Connect <em>your agent</em></h2><span id="pipeName"></span></div>
              <div class="cards">
                <article class="card">
                  <span class="tag">Claude Code · any MCP host</span>
                  <h3>Add AgentGo as an MCP server</h3>
                  <p>Each host gets its own isolated browser instance and a small set of token-lean tools.</p>
                  <div class="code" data-copy>claude mcp add agentgo -- agentgo mcp</div>
                </article>
                <article class="card">
                  <span class="tag">Any agent with a shell</span>
                  <h3>Drive this window from the CLI</h3>
                  <p>Attaches to the running browser over its private named pipe. Tabs you did not open stay yours.</p>
                  <div class="code" data-copy>agentgo call navigate.url '{"url":"example.com"}'</div>
                </article>
                <article class="card">
                  <span class="tag">Custom integrations</span>
                  <h3>Speak the protocol directly</h3>
                  <p>Newline-delimited JSON over stdio or the pipe. One request can batch a whole form.</p>
                  <div class="code" data-copy>agentgo run --stdio</div>
                </article>
              </div>
            </section>

            <section class="rise" style="--i:9">
              <div class="split">
                <div class="sees" aria-label="Example of the compact page view an agent receives">
<span class="d"># what the agent reads instead of raw HTML</span>
<span class="k">observe.page</span> →
# Sign in
[1] textbox(email) <span class="s">"Email"</span> required
[2] textbox(password) <span class="s">"Password"</span> required
[3] checkbox <span class="s">"Keep me signed in"</span>
[4] button <span class="s">"Continue"</span>
[5] link <span class="s">"Forgot password?"</span>

<span class="k">act.fill</span> {ref: 1, text: "…"} →
~ [1] textbox(email) <span class="s">"Email"</span> value=<span class="s">"…"</span> required
<span class="d"># every action answers with only what changed</span></div>
                <div class="trust">
                  <h3>You stay the admin.</h3>
                  <ul>
                    <li><span class="chk"><svg class="ic xs"><use href="#i-check"/></svg></span><span><b>Payments always ask you.</b> Purchases, checkouts and card forms need your approval in every mode.</span></li>
                    <li><span class="chk"><svg class="ic xs"><use href="#i-check"/></svg></span><span><b>CAPTCHAs come to you.</b> The agent stops, Windows notifies you, and it resumes in the same tab when you are done.</span></li>
                    <li><span class="chk"><svg class="ic xs"><use href="#i-check"/></svg></span><span><b>Pause or stop in one click.</b> Stop revokes agent input until you turn it back on.</span></li>
                    <li><span class="chk"><svg class="ic xs"><use href="#i-check"/></svg></span><span><b>Every action is logged locally.</b> Typed text and cookie values never enter the log.</span></li>
                  </ul>
                </div>
              </div>
            </section>

            <footer class="foot rise" style="--i:10">
              <span>AgentGo <b id="footVersion">—</b></span>
              <span>Chromium <b id="footChromium">—</b></span>
              <span>Profile <b id="footProfile">—</b></span>
              <span>Mode <b id="footMode">—</b></span>
              <span>Search <b>Google</b></span>
            </footer>
          </div>
        </section>
      </div>
    </section>
  </main>
</div>
<script src="newtab.js"></script>
</body>
</html>
`;

const js = `// AgentGo New Tab page. Everything shown here is real: navigation goes to
// the actual destination, and the agent link, activity, bookmarks and recent
// pages come from the browser through the read-only \`newtab.state\` bridge.
(() => {
"use strict";
const $  = s => document.querySelector(s);
const $$ = s => [...document.querySelectorAll(s)];
const lerp = (a,b,t) => a+(b-a)*t;
const jr = i => Math.abs(Math.sin(i*127.1)*43758.5453 % 1);
const pad = n => String(n).padStart(2,'0');
const home = $('#view-home');

/* =====================================================
   HALFTONE WINGS + HALO — particle backdrop
===================================================== */
${wings}

/* pointer parallax */
let homePointerFrame=0, homePointerX=0, homePointerY=0;
home.addEventListener('pointermove', e=>{
  const r=home.getBoundingClientRect();
  homePointerX=(((e.clientX-r.left)/r.width-.5)*2).toFixed(3);
  homePointerY=(((e.clientY-r.top)/r.height-.5)*2).toFixed(3);
  if(homePointerFrame) return;
  homePointerFrame=requestAnimationFrame(()=>{
    homePointerFrame=0;
    home.style.setProperty('--px',homePointerX);
    home.style.setProperty('--py',homePointerY);
  });
});
home.addEventListener('pointerleave', ()=>{ home.style.setProperty('--px',0); home.style.setProperty('--py',0); });

/* scroll parallax on the backdrop */
(() => {
  const vp=$('#viewport'), fx=$('#homeFx'); let raf=0;
  vp.addEventListener('scroll', ()=>{
    if(raf) return;
    raf=requestAnimationFrame(()=>{
      raf=0;
      const y=vp.scrollTop;
      fx.style.transform=\`translateY(\${(y*0.22).toFixed(1)}px)\`;
      fx.style.opacity=Math.max(.25, 1 - y/1500).toFixed(2);
    });
  }, {passive:true});
})();

/* staged reveals, once */
(() => {
  const targets=$$('.rise,.headline-reveal');
  const reveal=new IntersectionObserver(entries=>{
    for(const entry of entries){
      if(!entry.isIntersecting) continue;
      entry.target.classList.add('is-visible');
      reveal.unobserve(entry.target);
    }
  },{root:$('#viewport'),threshold:.08,rootMargin:'0px 0px -4% 0px'});
  targets.forEach(el=>reveal.observe(el));
})();

/* ============ NAVIGATION (real) ============ */
// Same rule as the address bar: an address opens directly, anything else is
// a Google search.
const resolve = raw => {
  const text = raw.trim();
  if (!text) return null;
  if (/^(https?|file|about):/i.test(text)) return text;
  if (/^[^\\s]+\\.[^\\s]+$/.test(text) || /^localhost(:\\d+)?(\\/|$)/.test(text)) return 'https://' + text;
  return 'https://www.google.com/search?q=' + encodeURIComponent(text);
};
$('#omniForm').addEventListener('submit', e => {
  e.preventDefault();
  const url = resolve($('#omniInput').value);
  if (url) location.assign(url);
});

const SHORTCUTS=[
  ['GitHub','github','https://github.com/'],['Mail','mail','https://mail.google.com/'],
  ['Figma','figma','https://www.figma.com/files/'],['Maps','maps','https://maps.google.com/'],
  ['YouTube','youtube','https://www.youtube.com/'],['HN','hn','https://news.ycombinator.com/'],
  ['arXiv','arxiv','https://arxiv.org/'],['Docs','docs','https://docs.google.com/']
];
const mark = (brand, extra='') => \`<span class="site-mark brand-\${brand} \${extra}"><svg aria-hidden="true"><use href="#brand-\${brand}"/></svg></span>\`;
SHORTCUTS.forEach(([label,brand,url],slot)=>{
  const a=document.createElement('a'); a.className='scut'; a.href=url;
  a.style.setProperty('--slot',slot);
  a.innerHTML=\`\${mark(brand,'sg')}<span>\${label}</span>\`;
  $('#shortcuts').appendChild(a);
});

/* ============ CLOCK ============ */
const tick = () => {
  const d=new Date(), h=d.getHours();
  $('#greetTime').textContent=pad(h)+':'+pad(d.getMinutes());
  $('#greetDay').textContent=d.toLocaleDateString('en-US',{weekday:'short'}).toUpperCase();
  $('#greetWord').textContent=h<5?'Good night':h<12?'Good morning':h<18?'Good afternoon':'Good evening';
};
tick(); setInterval(tick, 15000);

/* ============ COPY ============ */
for (const block of $$('[data-copy]')) {
  const text = block.textContent;
  const button = document.createElement('button');
  button.className = 'copy'; button.type = 'button'; button.title = 'Copy'; button.setAttribute('aria-label', 'Copy command');
  button.innerHTML = '<svg class="ic sm"><use href="#i-copy"/></svg>';
  button.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(text); button.classList.add('done'); setTimeout(() => button.classList.remove('done'), 1400); } catch (_) {}
  });
  block.appendChild(button);
}

/* ============ LIVE STATE ============ */
const esc = value => String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
const host = url => { try { return new URL(url).host.replace(/^www\\./,''); } catch (_) { return url; } };
const ago = seconds => {
  const s = Math.max(0, Math.floor(Date.now()/1000 - seconds));
  if (s < 60) return 'now';
  if (s < 3600) return Math.floor(s/60)+'m';
  if (s < 86400) return Math.floor(s/3600)+'h';
  return Math.floor(s/86400)+'d';
};
const brandOf = url => {
  const u = String(url).toLowerCase();
  for (const [needle, brand] of [['github','github'],['mail.google','mail'],['figma','figma'],['maps.google','maps'],['youtube','youtube'],['ycombinator','hn'],['arxiv','arxiv'],['docs.google','docs']]) {
    if (u.includes(needle)) return brand;
  }
  return 'agentgo';
};
const siteRow = (item, time) => \`<a class="row" href="\${esc(item.url)}">\${mark(brandOf(item.url))}<span class="row-t"><b>\${esc(item.title || host(item.url))}</b><span>\${esc(host(item.url))}</span></span>\${time ? \`<time>\${esc(time)}</time>\` : ''}</a>\`;

let lastRender = '';
const render = state => {
  const key = JSON.stringify(state);
  if (key === lastRender) return;
  lastRender = key;

  document.documentElement.dataset.theme = state.theme === 'light' ? 'light' : state.theme === 'system' && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  if (state.accent) document.documentElement.style.setProperty('--accent', state.accent);
  document.documentElement.classList.toggle('no-motion', state.motion === false);
  wings.setRunning(state.motion !== false);
  wings.refreshColors();

  const agents = state.agents || [];
  const chip = $('#agentChip');
  chip.className = 'agent-chip' + (state.control === 'stopped' ? ' stopped' : state.control === 'paused' ? ' paused' : agents.length ? ' live' : '');
  $('#agentChipText').textContent = state.control === 'stopped' ? 'Agent control stopped'
    : state.control === 'paused' ? 'Agents paused'
    : agents.length ? \`\${agents.length} agent\${agents.length === 1 ? '' : 's'} connected · \${agents.map(a => a.client).join(', ')}\`
    : 'No agent connected — connect one';

  const bookmarks = state.bookmarks || [];
  $('#bookmarksBlock').hidden = !bookmarks.length;
  $('#bookmarksCount').textContent = bookmarks.length + ' saved';
  $('#bookmarkRows').innerHTML = bookmarks.map(item => siteRow(item)).join('');

  const recent = state.recent || [];
  $('#recentBlock').hidden = !recent.length;
  $('#recentRows').innerHTML = recent.map(item => siteRow(item, ago(item.visitedAt))).join('');

  const activity = state.activity || [];
  $('#activityBlock').hidden = !activity.length;
  $('#activityRows').innerHTML = activity.map(a => \`<div class="feed-row"><time>\${esc(new Date(a.at*1000).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false}))}</time><span class="who">\${esc(a.session)}</span><span class="sum"><span class="what">\${esc(a.method)}</span> \${esc(a.summary)}</span><span class="ok \${a.ok ? '' : 'bad'}">\${a.ok ? 'ok' : 'refused'}</span></div>\`).join('');

  $('#pipeName').textContent = state.pipe || '';
  $('#footVersion').textContent = state.version;
  $('#footChromium').textContent = state.chromium;
  $('#footProfile').textContent = \`\${state.profile.id}\${state.profile.ephemeral ? ' · private' : ''}\`;
  $('#footMode').textContent = state.controlMode === 'full' ? 'full access' : 'approval';
};

const refresh = async () => {
  if (document.hidden) return;
  try {
    const response = await fetch('agentgo://shell/__bridge?method=newtab.state&params=%7B%7D');
    const body = await response.json();
    if (body.ok) render(body.result);
  } catch (_) {
    $('#agentChipText').textContent = 'Agent link unavailable';
  }
};
refresh();
setInterval(refresh, 2500);
document.addEventListener('visibilitychange', refresh);
})();
`;

fs.writeFileSync(path.join(ui, "index.html"), html);
fs.writeFileSync(path.join(ui, "newtab.js"), js);
console.log("index.html", html.length, "bytes;", "newtab.js", js.length, "bytes");
