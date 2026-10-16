// Command palette (Ctrl+K), first-run setup wizard (import and agent
// connection) and "Report a problem". Loaded after tools.js and shares its
// helpers (callBridge, esc, toolBody, shellData ...). Nothing here is
// simulated: every row and button calls a real browser command.

/* ---------- command palette ---------- */
let palFuse = null;
let palItems = [];
let palShown = [];
let palSel = 0;

const PAL_ICON = {
  tab: 'M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
  bookmark: 'M7 4h10a1 1 0 0 1 1 1v15l-6-4-6 4V5a1 1 0 0 1 1-1z',
  history: 'M12 7v5l3 2M4 12a8 8 0 1 0 3-6.2M4 4v4h4',
  setting: 'M4 7h9M17 7h3M4 17h3M11 17h9M13 5v4M7 15v4',
  action: 'M13 3 5 14h6l-1 7 8-11h-6z',
  ask: 'M4 5h16v11H9l-5 4z',
  web: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18',
};
const PAL_KIND = { tab: 'Tab', bookmark: 'Bookmark', history: 'History', setting: 'Setting', action: 'Action', ask: 'Agent', web: 'Web' };

const settingsPatch = settings => callBridge('settings.set', { settings });
let nativeClosed = false;
const act = (name, section = '') => { nativeClosed = true; return callBridge('ui.run_action', { name, section }); };
const lang = () => (navigator.language || 'en').split('-')[0];

const PAL_ACTIONS = () => {
  const s = settingsCache || {};
  const toggleSetting = (key, now) => settingsPatch({ [key]: !now });
  return [
    ['New tab', 'Ctrl+T', 'open create blank', () => callBridge('tabs.create', {})],
    ['Close tab', 'Ctrl+W', 'shut', () => act('close_tab')],
    ['Reopen closed tab', 'Ctrl+Shift+T', 'restore undo', () => act('reopen_tab')],
    ['Reload page', 'Ctrl+R', 'refresh', () => callBridge('tab.reload', {})],
    ['Go back', 'Alt+←', 'previous', () => callBridge('tab.back', {})],
    ['Go forward', 'Alt+→', 'next', () => callBridge('tab.forward', {})],
    ['Find in page', 'Ctrl+F', 'search text', () => act('find')],
    ['Print page', 'Ctrl+P', 'pdf paper save', () => act('print')],
    ['Bookmark this page', 'Ctrl+D', 'favorite star save', () => callBridge('bookmark.add', {})],
    ['Translate this page', '', 'language translate', () => act('tool:translate')],
    ['Inspect page (developer tools)', 'F12', 'devtools debug elements console', () => act('inspect')],
    ['Zoom in', 'Ctrl++', 'bigger larger', () => act('zoom_in')],
    ['Zoom out', 'Ctrl+−', 'smaller', () => act('zoom_out')],
    ['Reset zoom', 'Ctrl+0', '100 percent', () => act('zoom_reset')],
    ['Open history', 'Ctrl+H', 'visited', () => act('tool:history')],
    ['Open downloads', 'Ctrl+J', 'files', () => act('tool:downloads')],
    ['Open bookmarks', '', 'favorites', () => act('tool:bookmarks')],
    ['Open extensions', '', 'addons plugins', () => act('tool:extensions')],
    ['Open settings', '', 'preferences options', () => act('tool:settings')],
    ['Switch browser theme', '', 'dark light appearance night', () => settingsPatch({ theme: (s.theme || 'dark') === 'light' ? 'dark' : 'light' })],
    [s.siteDark === true ? 'Turn off dark mode for websites' : 'Turn on dark mode for websites', '', 'night sites dark reader websites', () => callBridge('ui.site_dark', { all: s.siteDark !== true })],
    ['Dark mode: toggle for this site', '', 'night site exclude dark reader', () => callBridge('ui.site_dark', {})],
    [s.adBlock === false ? 'Turn on ad blocker' : 'Turn off ad blocker', '', 'ads trackers block privacy', () => toggleSetting('adBlock', s.adBlock !== false)],
    [s.favoritesBar === false ? 'Show favorites bar' : 'Hide favorites bar', 'Ctrl+Shift+B', 'bookmarks bar toolbar', () => toggleSetting('favoritesBar', s.favoritesBar !== false)],
    ['Clear browsing data', '', 'cookies cache delete privacy', () => act('tool:settings', 'privacy')],
    ['Import from Chrome, Edge or Firefox', '', 'bookmarks history passwords migrate switch', () => act('tool:setup', 'import')],
    ['Connect an agent (Claude Code, Codex…)', '', 'mcp assistant ai connect setup', () => act('tool:setup', 'agent')],
    ['Report a problem', '', 'bug issue feedback help logs', () => act('tool:report')],
  ];
};

const PAL_SETTINGS = [
  ['Appearance and theme', 'appearance', 'dark light motion animation'],
  ['Toolbar buttons and favorites bar', 'toolbar', 'home extensions bookmarks bar'],
  ['Agent control mode', 'agent', 'approval full access downloads passwords captcha'],
  ['On startup', 'startup', 'restore tabs new tab'],
  ['Search engine', 'search', 'google bing duckduckgo'],
  ['Privacy and ad blocking', 'privacy', 'ads trackers cookies permissions'],
  ['Dark mode for websites', 'appearance', 'sites night dark reader exclude'],
  ['Languages', 'languages', 'accept-language translate'],
  ['Download folder', 'downloads', 'files'],
  ['Setup and import', 'data', 'import chrome edge firefox report problem wizard'],
  ['System and hardware acceleration', 'system', 'gpu rendering'],
  ['About AgentGo', 'about', 'version chromium update'],
];

const buildPalette = async () => {
  const [tabs, marks, hist, settings] = await Promise.all([
    callBridge('tabs.list').catch(() => ({})),
    callBridge('bookmark.list').catch(() => ({})),
    callBridge('history.list', { limit: 100 }).catch(() => ({})),
    callBridge('settings.get').catch(() => ({})),
  ]);
  settingsCache = settings;
  const items = [];
  for (const tab of tabs.tabs || []) {
    items.push({ kind: 'tab', title: tab.title || tab.url, sub: tab.url, run: () => callBridge('tabs.activate', { tabId: tab.tabId }), active: tab.tabId === tabs.activeTabId });
  }
  for (const mark of marks.items || []) {
    items.push({ kind: 'bookmark', title: mark.title || mark.url, sub: mark.url, run: () => callBridge('tabs.create', { url: mark.url }) });
  }
  for (const entry of hist.items || []) {
    items.push({ kind: 'history', title: entry.title || entry.url, sub: entry.url, run: () => callBridge('tabs.create', { url: entry.url }) });
  }
  for (const [title, section, words] of PAL_SETTINGS) {
    items.push({ kind: 'setting', title, sub: 'Settings', words, run: () => act('tool:settings', section) });
  }
  for (const [title, hint, words, run] of PAL_ACTIONS()) {
    items.push({ kind: 'action', title, sub: hint, words, run });
  }
  palItems = items;
  palFuse = new Fuse(items, {
    keys: [{ name: 'title', weight: 3 }, { name: 'words', weight: 1.2 }, { name: 'sub', weight: 0.8 }],
    threshold: 0.28, ignoreLocation: true, includeMatches: true, minMatchCharLength: 2,
  });
};

const searchUrl = text => {
  const engine = settingsCache.searchEngine || 'Google';
  const base = engine === 'Bing' ? 'https://www.bing.com/search?q=' : engine === 'DuckDuckGo' ? 'https://duckduckgo.com/?q=' : 'https://www.google.com/search?q=';
  return base + encodeURIComponent(text);
};

const markMatch = (text, matches, key) => {
  const hit = (matches || []).find(m => m.key === key);
  if (!hit) return esc(text);
  let out = '', last = 0;
  for (const [a, b] of hit.indices.filter(([a, b]) => b - a >= 1)) {
    if (a < last) continue;
    out += esc(text.slice(last, a)) + '<mark>' + esc(text.slice(a, b + 1)) + '</mark>';
    last = b + 1;
  }
  return out + esc(text.slice(last));
};

const palRows = query => {
  const text = query.trim();
  let rows;
  if (!text) {
    const tabs = palItems.filter(i => i.kind === 'tab').slice(0, 6);
    const some = palItems.filter(i => i.kind === 'action').slice(0, 8);
    rows = [...tabs, ...some].map(item => ({ item }));
  } else if (text.startsWith('>')) {
    const q = text.slice(1).trim();
    const only = palItems.filter(i => i.kind === 'action' || i.kind === 'setting');
    rows = q ? new Fuse(only, palFuse.options).search(q, { limit: 14 }) : only.slice(0, 14).map(item => ({ item }));
  } else {
    rows = palFuse.search(text, { limit: 14 });
  }
  if (text && !text.startsWith('>')) {
    rows.push({ item: { kind: 'ask', title: `Ask your agent: “${text}”`, sub: 'Copies the task so you can paste it to your connected agent', run: () => callBridge('ui.context_action', { action: 'copy_translation', text }) }, fixed: true });
    rows.push({ item: { kind: 'web', title: `Search the web for “${text}”`, sub: settingsCache.searchEngine || 'Google', run: () => callBridge('tabs.create', { url: searchUrl(text) }) }, fixed: true });
  }
  return rows;
};

const paintPalette = () => {
  const input = $('#palInput');
  const list = $('#palList');
  if (!input || !list) return;
  palShown = palRows(input.value);
  palSel = Math.min(palSel, Math.max(0, palShown.length - 1));
  let last = '';
  list.innerHTML = palShown.map(({ item, matches }, index) => {
    const label = !input.value.trim() ? (item.kind === 'tab' ? 'Open tabs' : 'Suggested') : '';
    const head = label && label !== last ? `<div class="pal-group">${label}</div>` : '';
    last = label || last;
    return `${head}<button type="button" class="pal-row${index === palSel ? ' sel' : ''}" data-i="${index}" style="--r:${Math.min(index, 12)}">
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="${PAL_ICON[item.kind]}"/></svg>
      <span class="pal-main"><b>${markMatch(item.title, matches, 'title')}</b>${item.sub ? `<em>${esc(item.sub)}</em>` : ''}</span>
      <i class="pal-kind">${item.active ? 'Current' : PAL_KIND[item.kind]}</i></button>`;
  }).join('') || '<div class="pal-none">Nothing found</div>';
  list.querySelector('.sel')?.scrollIntoView({ block: 'nearest' });
};

const palRun = async index => {
  const row = palShown[index];
  if (!row) return;
  const isAction = row.item.kind === 'action' || row.item.kind === 'setting';
  if (!isAction && row.item.kind !== 'ask') window.__agentgoRequestClose();
  nativeClosed = false;
  try {
    await row.item.run();
    if (row.item.kind === 'ask') {
      const list = $('#palList');
      list.innerHTML = '<div class="pal-done"><b>Copied.</b><span>Paste it into Claude Code, Codex or whichever agent is connected.</span></div>';
      setTimeout(() => window.__agentgoRequestClose(), 1300);
    } else if (row.item.kind === 'action' && !nativeClosed) {
      window.__agentgoRequestClose();
    }
  } catch (error) {
    $('#palList').innerHTML = `<div class="pal-none">${esc(error.message)}</div>`;
  }
};
// Actions that open another panel close this one natively; the rest close it here.
const isNavigationalAction = async () => false;

const renderPalette = async () => {
  document.body.classList.add('pal');
  toolBody.innerHTML = `<div class="pal-box">
    <div class="pal-top"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/></svg>
      <input id="palInput" type="text" placeholder="Search tabs, history, bookmarks, settings, actions… or ask your agent" autocomplete="off" spellcheck="false" aria-label="Command palette">
      <kbd>Esc</kbd></div>
    <div id="palList" class="pal-list" role="listbox"></div>
    <div class="pal-foot"><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>Enter</kbd> run</span><span><kbd>&gt;</kbd> actions only</span></div></div>`;
  await buildPalette();
  const input = $('#palInput');
  palSel = 0;
  paintPalette();
  input.focus();
  input.addEventListener('input', () => { palSel = 0; paintPalette(); });
  input.addEventListener('keydown', event => {
    if (event.key === 'ArrowDown') { event.preventDefault(); palSel = (palSel + 1) % Math.max(1, palShown.length); paintPalette(); }
    else if (event.key === 'ArrowUp') { event.preventDefault(); palSel = (palSel - 1 + palShown.length) % Math.max(1, palShown.length); paintPalette(); }
    else if (event.key === 'Enter') { event.preventDefault(); palRun(palSel); }
  });
  $('#palList').addEventListener('click', event => {
    const row = event.target.closest('.pal-row');
    if (row) palRun(Number(row.dataset.i));
  });
  $('#palList').addEventListener('pointermove', event => {
    const row = event.target.closest('.pal-row');
    if (row && Number(row.dataset.i) !== palSel) {
      palSel = Number(row.dataset.i);
      document.querySelectorAll('.pal-row.sel').forEach(el => el.classList.remove('sel'));
      row.classList.add('sel');
    }
  });
};

/* ---------- setup wizard ---------- */
const STEPS = ['welcome', 'import', 'agent', 'done'];
let wizStep = 0;
let wizScan = null;
let wizPick = 0;
let wizOpts = { bookmarks: true, history: true, logins: true, tabs: false };
let wizResult = null;
let wizBusy = false;
let wizAgent = {};

const BROWSER_GLYPH = { Chrome: 'C', Edge: 'E', Brave: 'B', Firefox: 'F' };
const flowArt = () => `<svg class="flow" viewBox="0 0 320 90" aria-hidden="true">
  <defs><linearGradient id="fw" x1="0" x2="1"><stop offset="0" stop-color="var(--accent-deep)"/><stop offset="1" stop-color="var(--accent-hi)"/></linearGradient></defs>
  <path id="fp" d="M62 45 C120 4, 200 86, 258 45" fill="none" stroke="url(#fw)" stroke-width="2" stroke-linecap="round" stroke-dasharray="3 7"/>
  <circle class="orb a" cx="44" cy="45" r="26"/><circle class="orb b" cx="276" cy="45" r="26"/>
  <text x="44" y="51" text-anchor="middle" class="orb-t">⇄</text><text x="276" y="52" text-anchor="middle" class="orb-t big">A</text>
  <circle r="4" fill="var(--accent-hi)"><animateMotion dur="2.4s" repeatCount="indefinite"><mpath href="#fp"/></animateMotion></circle>
  <circle r="3" fill="var(--accent)" opacity=".7"><animateMotion dur="2.4s" begin="-.8s" repeatCount="indefinite"><mpath href="#fp"/></animateMotion></circle>
  <circle r="2.5" fill="var(--accent-hi)" opacity=".5"><animateMotion dur="2.4s" begin="-1.6s" repeatCount="indefinite"><mpath href="#fp"/></animateMotion></circle></svg>`;

const dots = () => `<div class="dots">${STEPS.map((_, i) => `<i class="${i === wizStep ? 'on' : i < wizStep ? 'past' : ''}"></i>`).join('')}</div>`;

const stepWelcome = () => `
  <p class="wiz-kicker">Welcome</p><h2 class="wiz-title">A browser your agents can <em>drive</em>.</h2>
  ${flowArt()}<p class="wiz-lead">Two quick things and you are set: bring your stuff over from your old browser, and connect the agent you already use. Everything stays on this computer.</p>
  <div class="wiz-theme"><span>Look</span>${['dark', 'light', 'system'].map(v => `<button type="button" class="pill${(settingsCache.theme || 'dark') === v ? ' on' : ''}" data-f="theme" data-v="${v}">${v}</button>`).join('')}</div>`;

const stepImport = () => {
  if (!wizScan) return `<p class="wiz-kicker">Step 1 of 2</p><h2 class="wiz-title">Bring your <em>stuff</em></h2>${flowArt()}<p class="wiz-lead">Looking for Chrome, Edge, Brave and Firefox on this computer…</p>`;
  const profiles = wizScan.profiles || [];
  if (wizResult) {
    const c = wizResult.imported || {};
    return `<p class="wiz-kicker">Imported</p><h2 class="wiz-title">All <em>moved in</em>.</h2>
      <div class="wiz-stats">${[['Bookmarks', c.bookmarks], ['History', c.history], ['Passwords', c.logins], ['Tabs opened', c.tabs]].map(([n, v], i) => `<div style="--r:${i}"><b>${v ?? 0}</b><span>${n}</span></div>`).join('')}</div>
      ${(wizResult.notes || []).map(n => `<p class="wiz-note">${esc(n)}</p>`).join('')}`;
  }
  if (!profiles.length) return `<p class="wiz-kicker">Step 1 of 2</p><h2 class="wiz-title">Bring your <em>stuff</em></h2>${flowArt()}<p class="wiz-lead">No other browser was found on this computer, so there is nothing to import right now. You can come back any time from Settings.</p>`;
  const p = profiles[Math.min(wizPick, profiles.length - 1)];
  const c = p.counts || {};
  const box = (key, label, n, note = '') => `<label class="wiz-opt${n ? '' : ' none'}"><input type="checkbox" data-f="opt" data-k="${key}"${wizOpts[key] && n ? ' checked' : ''}${n ? '' : ' disabled'}><i></i><span><b>${label}</b><em>${n ? `${n.toLocaleString()} found${note}` : 'none found'}</em></span></label>`;
  return `<p class="wiz-kicker">Step 1 of 2</p><h2 class="wiz-title">Bring your <em>stuff</em></h2>
    <div class="wiz-profiles">${profiles.map((q, i) => `<button type="button" class="wiz-prof${i === wizPick ? ' on' : ''}" data-f="pick" data-i="${i}" style="--r:${i}"><span class="glyph">${BROWSER_GLYPH[q.browser] || '•'}</span><span><b>${esc(q.browser)}</b><em>${esc(q.name)}</em></span></button>`).join('')}</div>
    <div class="wiz-opts">${box('bookmarks', 'Bookmarks', c.bookmarks)}${box('history', 'History', c.history, ' (latest 5,000 used)')}${box('logins', 'Saved passwords', c.logins, ' (encrypted again for this Windows account)')}${box('tabs', 'Open tabs', c.tabs, ' (opened as new tabs)')}</div>
    ${wizBusy ? `<div class="wiz-busy"><i></i><i></i><i></i> Importing…</div>` : ''}
    <p class="wiz-fine">Close ${esc(p.browser)} first for the most complete copy. Nothing leaves this computer.</p>`;
};

const stepAgent = () => `
  <p class="wiz-kicker">Step 2 of 2</p><h2 class="wiz-title">Connect your <em>agent</em></h2>
  <p class="wiz-lead">One click adds AgentGo to your agent's tool list. You can do this later from the terminal with <code>agentgo setup</code>.</p>
  <div class="wiz-hosts">${[['claude', 'Claude Code', 'C'], ['codex', 'Codex CLI', 'X'], ['droid', 'Factory Droid', 'D'], ['json', 'Other (copy config)', '{ }']].map(([id, name, g], i) => `
    <button type="button" class="wiz-host${wizAgent[id]?.ok ? ' ok' : ''}" data-f="host" data-h="${id}" style="--r:${i}"><span class="glyph">${g}</span><span><b>${name}</b><em>${esc(wizAgent[id]?.text || (id === 'json' ? 'Shows the config to paste' : 'Adds the MCP server for you'))}</em></span><svg viewBox="0 0 24 24"><path d="m6 12.5 4 4 8-9"/></svg></button>`).join('')}</div>`;

const stepDone = () => `
  <p class="wiz-kicker">Ready</p><h2 class="wiz-title">You are <em>set</em>.</h2>
  <div class="wiz-tips">
    <div style="--r:0"><kbd>Ctrl</kbd><kbd>K</kbd><span>Search tabs, history, bookmarks, settings and every action, or hand a task to your agent.</span></div>
    <div style="--r:1"><svg viewBox="0 0 24 24"><path d="M4 5h8M8 3v2M6 5c0 4 3 7 6 8M13 20l4-9 4 9M14.5 17h5"/></svg><span>Right-click any page to translate it, or select text to translate just that.</span></div>
    <div style="--r:2"><svg viewBox="0 0 24 24"><path d="M12 3a7 7 0 0 0-4 12.7V18h8v-2.3A7 7 0 0 0 12 3zM9 21h6"/></svg><span>Dark mode for websites and the ad blocker live in Settings.</span></div>
  </div>`;

const renderWizardNow = () => {
  const body = [stepWelcome, stepImport, stepAgent, stepDone][wizStep]();
  const last = wizStep === STEPS.length - 1;
  const importDone = wizStep === 1 && wizResult;
  const canImport = wizStep === 1 && wizScan?.profiles?.length && !wizResult;
  toolBody.innerHTML = `<div class="wiz">${dots()}<div class="wiz-main" data-step="${wizStep}">${body}</div>
    <div class="wiz-nav">
      ${wizStep > 0 && !last ? '<button class="btn" type="button" data-f="back">Back</button>' : '<span></span>'}
      <span class="wiz-nav-r">
        ${!last ? `<button class="btn" type="button" data-f="skip">${wizStep === 0 ? 'Skip setup' : 'Skip'}</button>` : ''}
        ${canImport ? `<button class="btn primary" type="button" data-f="import"${wizBusy ? ' disabled' : ''}>Import</button>` : ''}
        <button class="btn primary${canImport ? ' alt' : ''}" type="button" data-f="${last ? 'finish' : 'next'}">${last ? 'Start browsing' : importDone ? 'Continue' : wizStep === 0 ? 'Get started' : 'Next'}</button>
      </span></div></div>`;
};

const scanForImport = async () => {
  try { wizScan = await callBridge('ui.import_scan'); } catch (error) { wizScan = { profiles: [], error: error.message }; }
  if (activeTool === 'setup') renderWizardNow();
};

const renderSetup = async () => {
  document.body.classList.add('wizard');
  settingsCache = await callBridge('settings.get').catch(() => ({}));
  const want = shellData().section;
  wizStep = want === 'import' ? 1 : want === 'agent' ? 2 : 0;
  wizResult = null; wizBusy = false;
  renderWizardNow();
  if (!wizScan) scanForImport(); else if (wizStep === 1) scanForImport();
};

const wizGo = step => {
  wizStep = Math.max(0, Math.min(STEPS.length - 1, step));
  renderWizardNow();
  if (wizStep === 1 && !wizScan) scanForImport();
};

const finishSetup = async () => {
  await callBridge('ui.setup_done').catch(() => {});
  window.__agentgoRequestClose();
};

const wizClick = async event => {
  const el = event.target.closest('[data-f]');
  if (!el || activeTool !== 'setup') return;
  const f = el.dataset.f;
  if (f === 'next') wizGo(wizStep + 1);
  else if (f === 'back') wizGo(wizStep - 1);
  else if (f === 'skip') { if (wizStep === 0) await finishSetup(); else wizGo(wizStep + 1); }
  else if (f === 'finish') await finishSetup();
  else if (f === 'theme') { await settingsPatch({ theme: el.dataset.v }); settingsCache.theme = el.dataset.v; renderWizardNow(); }
  else if (f === 'pick') { wizPick = Number(el.dataset.i); renderWizardNow(); }
  else if (f === 'opt') { wizOpts[el.dataset.k] = el.checked; }
  else if (f === 'import') {
    const p = wizScan.profiles[wizPick];
    wizBusy = true; renderWizardNow();
    try {
      wizResult = await callBridge('ui.import_run', { browser: p.browser, id: p.id, options: wizOpts });
    } catch (error) {
      wizResult = { imported: {}, notes: [error.message] };
    }
    wizBusy = false; renderWizardNow();
  } else if (f === 'host') {
    const id = el.dataset.h;
    wizAgent[id] = { text: 'Connecting…' }; renderWizardNow();
    try {
      const out = await callBridge('ui.import_run', { connect: id });
      wizAgent[id] = { ok: out.ok !== false, text: out.text || 'Connected' };
      if (id === 'json' && out.config) {
        await callBridge('ui.context_action', { action: 'copy_translation', text: out.config });
        wizAgent[id].text = 'Config copied. Paste it into your agent’s MCP settings';
      }
    } catch (error) {
      wizAgent[id] = { ok: false, text: error.message };
    }
    renderWizardNow();
  }
};

/* ---------- report a problem ---------- */
let reportData = null;
const renderReportNow = (state = {}) => {
  document.body.classList.add('rep');
  if (state.done) {
    toolBody.innerHTML = `<div class="rep-done"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="m8 12.5 3 3 5-6"/></svg><h3>Saved</h3>
      <p>The redacted report is in <code>${esc(state.done.path)}</code>.${state.done.issueOpened ? ' An issue form with the summary opened in a new tab; drag the zip into it.' : ' No issue tracker is set up for this build, so attach the zip wherever you report the problem.'}</p></div>`;
    return;
  }
  if (!reportData) { toolBody.innerHTML = '<div class="rep-load"><i></i><i></i><i></i><p>Collecting logs and a screenshot…</p></div>'; return; }
  const d = reportData;
  toolBody.innerHTML = `<div class="rep">
    <p class="rep-lead">Here is exactly what will be saved. Passwords, tokens, emails and long numbers are hidden, and web addresses lose everything after the “?”.</p>
    <label class="rep-label" for="repNote">What went wrong?</label>
    <textarea id="repNote" rows="3" placeholder="Describe what you did and what you expected" maxlength="1500">${esc(state.note || '')}</textarea>
    <div class="rep-chips"><span class="pill on">${d.redactions} item${d.redactions === 1 ? '' : 's'} hidden</span>${d.tracker ? '<span class="pill">Opens a pre-filled issue</span>' : '<span class="pill">Saved to disk only</span>'}</div>
    ${d.screenshot ? `<label class="rep-shot"><input type="checkbox" id="repShot" ${state.shot === false ? '' : 'checked'}><i></i><img alt="Screenshot of the current page" src="data:image/jpeg;base64,${d.screenshot}"><span>Include this screenshot</span></label>` : ''}
    <details class="rep-pre" open><summary>Report text</summary><pre>${esc(d.text)}</pre></details>
    <div class="rep-acts"><button class="btn" type="button" data-f="cancel">Cancel</button><button class="btn primary" type="button" data-f="send">${d.tracker ? 'Save and open issue' : 'Save report'}</button></div></div>`;
};

const renderReport = async () => {
  reportData = null;
  renderReportNow();
  try { reportData = await callBridge('ui.report_collect'); } catch (error) { reportData = { text: error.message, redactions: 0, tracker: false }; }
  if (activeTool === 'report') renderReportNow();
};

const reportClick = async event => {
  const el = event.target.closest('[data-f]');
  if (!el || activeTool !== 'report') return;
  if (el.dataset.f === 'cancel') window.__agentgoRequestClose();
  else if (el.dataset.f === 'send') {
    const note = $('#repNote')?.value || '';
    const shot = $('#repShot')?.checked !== false;
    el.disabled = true; el.textContent = 'Saving…';
    try {
      const done = await callBridge('ui.report_submit', { description: note, includeScreenshot: shot && Boolean(reportData?.screenshot) });
      renderReportNow({ done });
    } catch (error) {
      renderReportNow({ note, shot });
      toolBody.querySelector('.rep-lead').textContent = error.message;
    }
  }
};

toolBody.addEventListener('click', event => { wizClick(event); reportClick(event); });
toolBody.addEventListener('change', event => { if (event.target.dataset?.f === 'opt') wizOpts[event.target.dataset.k] = event.target.checked; });

toolBody.addEventListener('click', async event => {
  const el = event.target.closest('[data-f2]');
  if (!el) return;
  const f = el.dataset.f2;
  if (f === 'import') await act('tool:setup', 'import');
  else if (f === 'agent') await act('tool:setup', 'agent');
  else if (f === 'palette') await act('tool:palette');
  else if (f === 'report') await act('tool:report');
  else if (f === 'clear-dark') { await settingsPatch({ siteDarkExclude: [] }); el.textContent = 'Cleared'; el.disabled = true; }
});
