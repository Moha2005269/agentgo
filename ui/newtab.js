// AgentGo New Tab page. Everything shown here is real: navigation goes to
// the actual destination, and the agent link, activity, bookmarks and recent
// pages come from the browser through the read-only `newtab.state` bridge.
(() => {
"use strict";
const $  = s => document.querySelector(s);
const $$ = s => [...document.querySelectorAll(s)];
const pad = n => String(n).padStart(2,'0');
const still = matchMedia('(prefers-reduced-motion: reduce)');

/* =====================================================
   THE LIVE WIRE
   A glowing orange string across the hero. It hums on its own, bends toward
   the cursor, twangs where you click, and carries sparks of current; more
   and faster while an agent is connected.
===================================================== */
const wire = (() => {
  const canvas = $('#wire');
  const ctx = canvas.getContext('2d');
  const hero = canvas.parentElement;
  const N = 120;
  let w = 0, h = 0, dpr = 1, base = 0;
  const u = new Float32Array(N), v = new Float32Array(N);
  let sparks = [];
  let pointer = null;            // {x, y} in canvas pixels, or null
  let live = false, running = true, visible = true, raf = 0, t0 = performance.now();
  let color = { accent: '#FF6A2B', hi: '#FF9259', glow: 'rgba(255,106,43,.55)' };

  const readColors = () => {
    const css = getComputedStyle(document.documentElement);
    color = { accent: css.getPropertyValue('--accent').trim(), hi: css.getPropertyValue('--accent-hi').trim(), glow: css.getPropertyValue('--glow').trim() };
  };
  const resize = () => {
    dpr = Math.min(2, devicePixelRatio || 1);
    const r = canvas.getBoundingClientRect();
    w = r.width; h = r.height;
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
    // the wire runs in the gap between the text and the search bar
    const lede = $('.lede').getBoundingClientRect(), omni = $('#omniForm').getBoundingClientRect(), top = hero.getBoundingClientRect().top;
    base = Math.min(h * .8, (lede.bottom + omni.top) / 2 - top);
    if (!running) draw(performance.now());
  };
  const xAt = i => (i / (N - 1)) * w;

  const step = (now) => {
    const time = (now - t0) / 1000;
    // tension between neighbours, a pull home, damping
    for (let i = 1; i < N - 1; i++) {
      const lap = u[i - 1] + u[i + 1] - 2 * u[i];
      v[i] += lap * .32 - u[i] * .012;
      v[i] *= .965;
    }
    // the cursor bends the wire toward itself
    if (pointer) {
      const reach = 160;
      for (let i = 1; i < N - 1; i++) {
        const dx = xAt(i) - pointer.x;
        if (Math.abs(dx) > reach * 2.5) continue;
        const weight = Math.exp(-(dx * dx) / (2 * reach * reach));
        const target = Math.max(-140, Math.min(140, pointer.y - base));
        if (Math.abs(pointer.y - base) < 220) v[i] += (target - u[i]) * .018 * weight;
      }
    }
    for (let i = 1; i < N - 1; i++) u[i] += v[i];
    // current: sparks run along the wire
    const rate = live ? .09 : .025;
    if (Math.random() < rate) sparks.push({ p: 0, speed: (live ? .006 : .0032) * (.7 + Math.random() * .6), size: 1 + Math.random() * 1.6 });
    sparks = sparks.filter(s => (s.p += s.speed) < 1);
    return time;
  };

  const yAt = (p, time) => {
    const f = p * (N - 1), i = Math.floor(f), k = f - i;
    const a = u[i] ?? 0, b = u[Math.min(N - 1, i + 1)] ?? 0;
    return base + a + (b - a) * k + hum(p, time);
  };
  // a slow, quiet hum so the wire is never dead still
  const hum = (p, time) => Math.sin(p * 9 + time * 1.3) * 3 * Math.sin(p * Math.PI) + Math.sin(p * 23 - time * 2.1) * 1.2 * Math.sin(p * Math.PI);

  const draw = (now) => {
    const time = (now - t0) / 1000;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const fade = ctx.createLinearGradient(0, 0, w, 0);
    fade.addColorStop(0, color.accent + '00'); fade.addColorStop(.12, color.accent); fade.addColorStop(.5, color.hi); fade.addColorStop(.88, color.accent); fade.addColorStop(1, color.accent + '00');
    const path = () => {
      ctx.beginPath();
      for (let i = 0; i < N; i++) {
        const p = i / (N - 1), x = xAt(i), y = base + u[i] + hum(p, time);
        i ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
      }
    };
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    // glow as wide translucent strokes (canvas blur is costly on 4K screens),
    // then the bright core
    ctx.strokeStyle = fade;
    for (const [width, alpha] of [[22, .05], [12, .09], [6, .18]]) {
      ctx.globalAlpha = alpha * (live ? 1.5 : 1); ctx.lineWidth = width; path(); ctx.stroke();
    }
    ctx.globalAlpha = 1; ctx.lineWidth = live ? 2 : 1.5; path(); ctx.stroke();
    // sparks with short trails and a soft halo
    for (const s of sparks) {
      const edge = Math.sin(s.p * Math.PI);
      const hx = s.p * w, hy = yAt(s.p, time);
      ctx.globalAlpha = edge * .22; ctx.fillStyle = color.hi;
      ctx.beginPath(); ctx.arc(hx, hy, s.size * 5, 0, Math.PI * 2); ctx.fill();
      for (let k = 0; k < 6; k++) {
        const p = Math.max(0, s.p - k * .006), x = p * w, y = yAt(p, time);
        ctx.globalAlpha = edge * (1 - k / 6) * .95;
        ctx.fillStyle = k ? color.hi : '#fff';
        ctx.beginPath(); ctx.arc(x, y, s.size * (1 - k / 9), 0, Math.PI * 2); ctx.fill();
      }
    }
    ctx.globalAlpha = 1;
  };

  // the wire holds still while the theme circle plays, so the switch has
  // the whole frame budget
  let heldUntil = 0;
  document.addEventListener('agentgo:themeswitch', event => { heldUntil = performance.now() + event.detail + 60; });
  const loop = (now) => {
    raf = 0;
    if (!running || !visible || document.hidden) return;
    if (now < heldUntil) { raf = requestAnimationFrame(loop); return; }
    step(now); draw(now);
    raf = requestAnimationFrame(loop);
  };
  const start = () => { if (!raf && running && visible && !document.hidden) raf = requestAnimationFrame(loop); };

  hero.addEventListener('pointermove', e => {
    const r = canvas.getBoundingClientRect();
    pointer = { x: e.clientX - r.left, y: e.clientY - r.top };
  });
  hero.addEventListener('pointerleave', () => { pointer = null; });
  // a click plucks the wire where you click, harder the closer you are
  hero.addEventListener('pointerdown', e => {
    if (e.target.closest('input,button,a')) return;
    const r = canvas.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
    const i = Math.round((x / w) * (N - 1));
    const force = Math.max(18, 90 - Math.abs(y - base) * .25) * (y < base ? -1 : 1);
    for (let k = -3; k <= 3; k++) if (u[i + k] !== undefined && i + k > 0 && i + k < N - 1) v[i + k] += force * (1 - Math.abs(k) / 4);
    for (let n = 0; n < 4; n++) sparks.push({ p: Math.max(0, x / w - .01 * n), speed: .008 + Math.random() * .006, size: 1.4 + Math.random() });
    start();
  });
  new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; start(); }, { root: $('#viewport') }).observe(hero);
  document.addEventListener('visibilitychange', start);
  let rt; addEventListener('resize', () => { clearTimeout(rt); rt = setTimeout(resize, 120); });
  readColors(); resize(); start();
  setTimeout(resize, 1600); // after the text has risen into place
  return {
    setLive(on) { live = on; },
    setRunning(on) { running = on && !still.matches; if (running) start(); else draw(performance.now()); },
    refreshColors() { readColors(); if (!running) draw(performance.now()); },
    resize,
  };
})();

/* the soft light that follows the cursor, and spotlights inside tiles */
(() => {
  const spot = $('#spot'); let x = -9999, y = -9999, f = 0;
  addEventListener('pointermove', e => {
    x = e.clientX; y = e.clientY;
    const tile = e.target.closest('.scut,.row,.card');
    if (tile) { const r = tile.getBoundingClientRect(); tile.style.setProperty('--mx', `${e.clientX - r.left}px`); tile.style.setProperty('--my', `${e.clientY - r.top}px`); }
    if (!f) f = requestAnimationFrame(() => { f = 0; spot.style.transform = `translate3d(${x}px,${y}px,0)`; });
  }, { passive: true });
  document.addEventListener('pointerleave', () => { spot.style.transform = 'translate3d(-9999px,-9999px,0)'; });
})();

/* reveals: once, as each part comes into view */
(() => {
  const reveal = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add('is-visible');
      reveal.unobserve(entry.target);
    }
  }, { root: $('#viewport'), threshold: .08, rootMargin: '0px 0px -4% 0px' });
  $$('.rise,#headline').forEach(el => reveal.observe(el));
})();

/* ============ NAVIGATION (real) ============ */
// Same rule as the address bar: an address opens directly, anything else is
// a search with the provider chosen in Settings (sent in newtab.state).
let searchBase = 'https://www.google.com/search?q=';
const resolve = raw => {
  const text = raw.trim();
  if (!text) return null;
  if (/^(https?|file|about):/i.test(text)) return text;
  if (/^[^\s]+\.[^\s]+$/.test(text) || /^localhost(:\d+)?(\/|$)/.test(text)) return 'https://' + text;
  // A custom engine is a template with %s where the words go.
  return searchBase.includes('%s') ? searchBase.replace('%s', encodeURIComponent(text)) : searchBase + encodeURIComponent(text);
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
const mark = (brand, extra='') => `<span class="site-mark brand-${brand} ${extra}"><svg aria-hidden="true"><use href="#brand-${brand}"/></svg></span>`;
SHORTCUTS.forEach(([label,brand,url],slot)=>{
  const a=document.createElement('a'); a.className='scut'; a.href=url;
  a.style.setProperty('--slot',slot);
  a.innerHTML=`${mark(brand,'sg')}<span>${label}</span>`;
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
const host = url => { try { return new URL(url).host.replace(/^www\./,''); } catch (_) { return url; } };
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
const siteRow = (item, time) => `<a class="row" href="${esc(item.url)}">${mark(brandOf(item.url))}<span class="row-t"><b>${esc(item.title || host(item.url))}</b><span>${esc(host(item.url))}</span></span>${time ? `<time>${esc(time)}</time>` : ''}</a>`;

// The animated switch arrives from the browser (__agentgoThemeSync); the
// wire takes the new colors once it is applied.
window.__agentgoThemeApplied = () => wire.refreshColors();
const applyTheme = setting => window.agentgoSetTheme(setting);
let lastRender = '';
const render = state => {
  const key = JSON.stringify(state);
  if (key === lastRender) return;
  lastRender = key;

  window.agentgoSetAccent(state.accent);
  applyTheme(state.theme || 'dark');
  wire.refreshColors();
  document.documentElement.classList.toggle('no-motion', state.motion === false);
  wire.setRunning(state.motion !== false);

  const agents = state.agents || [];
  wire.setLive(agents.length > 0 && state.control !== 'stopped');
  const chip = $('#agentChip');
  chip.className = 'agent-chip' + (state.control === 'stopped' ? ' stopped' : state.control === 'paused' ? ' paused' : agents.length ? ' live' : '');
  $('#agentChipText').textContent = state.control === 'stopped' ? 'Agent control stopped'
    : state.control === 'paused' ? 'Agents paused'
    : agents.length ? `${agents.map(a => a.client).join(', ')} connected`
    : 'No agent connected';

  const bookmarks = state.bookmarks || [];
  $('#bookmarksBlock').hidden = !bookmarks.length;
  $('#bookmarksCount').textContent = bookmarks.length + ' saved';
  $('#bookmarkRows').innerHTML = bookmarks.map(item => siteRow(item)).join('');

  const recent = state.recent || [];
  $('#recentBlock').hidden = !recent.length;
  $('#recentRows').innerHTML = recent.map(item => siteRow(item, ago(item.visitedAt))).join('');

  const activity = state.activity || [];
  $('#activityBlock').hidden = !activity.length;
  $('#activityRows').innerHTML = activity.map(a => `<div class="feed-row"><time>${esc(new Date(a.at*1000).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false}))}</time><span class="who">${esc(a.session)}</span><span class="sum"><span class="what">${esc(a.method)}</span> ${esc(a.summary)}</span><span class="ok ${a.ok ? '' : 'bad'}">${a.ok ? 'ok' : 'refused'}</span></div>`).join('');

  $('#pipeName').textContent = state.pipe || '';
  $('#footVersion').textContent = state.version;
  $('#footChromium').textContent = state.chromium;
  $('#footProfile').textContent = `${state.profile.id}${state.profile.ephemeral ? ' · private' : ''}`;
  $('#footMode').textContent = state.controlMode === 'full' ? 'full access' : 'approval';
};

const refresh = async () => {
  if (document.hidden) return;
  try {
    const response = await fetch('agentgo://shell/__bridge?method=newtab.state&params=%7B%7D');
    const body = await response.json();
    if (body.ok) { if (body.result.searchBase) searchBase = body.result.searchBase; render(body.result); }
  } catch (_) {
    $('#agentChipText').textContent = 'Agent link unavailable';
  }
};
refresh();
setInterval(refresh, 2500);
document.addEventListener('visibilitychange', refresh);
})();
