// Trusted tools panel: history, downloads, settings and the agent control
// panel. Every list and switch reflects native state and every button calls
// a real command; nothing here is simulated.
const $ = selector => document.querySelector(selector);
const toolBody = $('#toolBody');
// Panels redraw on a timer (downloads every second). A redraw identical to
// what is shown is skipped, so nothing flickers or replays its entrance.
{
  const native = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
  let shown = null;
  Object.defineProperty(toolBody, 'innerHTML', {
    get() { return native.get.call(this); },
    set(html) {
      if (html === shown && this.childElementCount) return;
      shown = html;
      native.set.call(this, html);
    },
  });
}
const toolFoot = $('#toolFoot');
const toolFilter = $('#toolFilter');

let activeTool = null;
let openSequence = 0;
let closing = false;
let armed = null;
let historyItems = [];

const TITLES = { history: ['Browser', 'History'], downloads: ['Browser', 'Downloads'], settings: ['Browser', 'Settings'], agent: ['Agent control', 'Agents'], accounts: ['Agent control', 'Account Center'], savelogin: ['Passwords', 'Save sign-in?'], find: ['Page', 'Find'], menu: ['Browser', 'Menu'], bookmarks: ['Browser', 'Bookmarks'], extensions: ['Browser', 'Extensions'], suggest: ['Address', 'Suggestions'], siteinfo: ['Site', 'Connection'], ctxmenu: ['Page', 'Menu'], palette: ['Command', 'Search everything'], setup: ['Welcome', 'Set up AgentGo'], report: ['Help', 'Report a problem'], translate: ['Page', 'Translate'], permission: ['Site', 'Permission request'] };

const callBridge = async (method, params = {}) => {
  const encoded = encodeURIComponent(JSON.stringify(params));
  const response = await fetch(`agentgo://shell/__bridge?method=${method}&params=${encoded}`);
  const body = await response.json();
  if (!response.ok || body.ok === false) {
    throw new Error(body.error?.message || `Bridge request failed: ${method}`);
  }
  return body.result;
};

const esc = value => String(value ?? '')
  .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
  .replaceAll('"', '&quot;').replaceAll("'", '&#039;');
const when = seconds => {
  const date = new Date(Number(seconds) * 1000);
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
};
const clock = seconds => new Date(Number(seconds) * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
const bytes = value => {
  const n = Number(value);
  if (!Number.isFinite(n) || n < 0) return '';
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
};
const empty = (title, note) => { toolBody.innerHTML = `<div class="empty"><div><strong>${esc(title)}</strong><p>${esc(note)}</p></div></div>`; };
const clearFoot = (label, target) => {
  toolFoot.hidden = false;
  toolFoot.innerHTML = `<span class="note"></span><button class="btn" type="button" data-clear="${target}">${esc(label)}</button>`;
};

/* ---------- history ---------- */
const renderHistory = () => {
  const needle = toolFilter.value.trim().toLowerCase();
  const items = historyItems.filter(item => !needle || `${item.title} ${item.url}`.toLowerCase().includes(needle));
  if (!historyItems.length) empty('No history yet', 'Pages you visit in this profile will appear here.');
  else if (!items.length) empty('No matches', 'Nothing in history matches that filter.');
  else toolBody.innerHTML = items.map(item => `
    <button class="row" type="button" data-open-url="${esc(item.url)}">
      <span><span class="row-title">${esc(item.title || item.url)}</span><span class="row-meta">${esc(item.url)}</span></span>
      <span class="row-side">${esc(when(item.visitedAt))}</span>
    </button>`).join('');
  clearFoot('Clear history', 'history');
  // How far back Clear goes, as in Chrome; the last choice is kept.
  toolFoot.firstElementChild.innerHTML = `<select class="btn sm" data-history-range aria-label="Time range to clear">${HISTORY_RANGES.map(([value, label]) => `<option value="${value}"${value === historyRange ? ' selected' : ''}>${esc(label)}</option>`).join('')}</select>`;
};
const HISTORY_RANGES = [['60', 'Last hour'], ['1440', 'Last 24 hours'], ['10080', 'Last 7 days'], ['40320', 'Last 4 weeks'], ['all', 'All time']];
let historyRange = '60';

/* ---------- downloads ---------- */
const renderDownloads = (items, enabled) => {
  if (!items.length) {
    empty('No downloads', enabled ? 'Files you or an agent download will appear here.' : 'Downloads are off. Turn on “Allow downloads” in Settings to let pages save files.');
  } else {
    toolBody.innerHTML = items.map(item => {
      const active = item.inProgress;
      const size = item.totalBytes > 0 ? `${bytes(item.receivedBytes)} of ${bytes(item.totalBytes)}` : bytes(item.receivedBytes);
      const controls = active
        ? `<button class="btn sm" type="button" data-download="${item.paused ? 'resume' : 'pause'}" data-id="${item.id}">${item.paused ? 'Resume' : 'Pause'}</button><button class="btn sm" type="button" data-download="cancel" data-id="${item.id}">Cancel</button>`
        : esc(item.status);
      return `<div class="row"><div><div class="row-title">${esc(item.suggestedName || 'Download')}</div><div class="row-meta">${esc(size)} · ${esc(item.path || item.url)}</div>${active ? `<div class="meter"><i style="width:${Math.max(0, Math.min(100, Number(item.percent) || 0))}%"></i></div>` : ''}</div><span class="row-side">${controls}</span></div>`;
    }).join('');
  }
  clearFoot('Clear finished', 'downloads');
  toolFoot.firstElementChild.textContent = enabled ? 'Files are never opened automatically.' : 'Downloads are off.';
};

/* ---------- settings ---------- */
const toggle = (key, value, title, note) => `
  <div class="setting"><div><b>${esc(title)}</b><span>${esc(note)}</span></div>
  <button class="switch${value ? ' on' : ''}" type="button" role="switch" aria-checked="${Boolean(value)}" aria-label="${esc(title)}" data-setting="${key}" data-kind="boolean"></button></div>`;

/* Ad and tracker blocking. The row flips in place (the panel redraws every
   other switch from scratch, which would cut the animation short). */
const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
const adBlockCountHtml = settings => {
  if (settings.adBlock === false) return 'Paused';
  if (settings.adBlockReady === false) return 'Warming up…';
  const blocked = Number(settings.adBlockBlocked) || 0;
  return `<data value="${blocked}">${blocked.toLocaleString()}</data> blocked this session`;
};
const adBlockRow = settings => {
  const on = settings.adBlock !== false;
  return `
  <div class="setting adblock${on ? ' on' : ''}${settings.motion === false ? ' still' : ''}" data-adblock>
    <div class="ab-main">
      <i class="ab-shield" aria-hidden="true"><svg viewBox="0 0 24 24"><path class="ab-outline" d="M12 2.5 4.5 5.6v5.7c0 4.6 3.1 8.5 7.5 10.2 4.4-1.7 7.5-5.6 7.5-10.2V5.6z"/><path class="ab-check" d="m8.6 12.2 2.5 2.5 4.4-5"/><path class="ab-slash" d="M5 4.5 19.5 19.5"/></svg><i class="ab-ring"></i></i>
      <div><b>Block ads and trackers</b><span>Built-in lists (EasyList, uBlock Origin, AdGuard and more) cancel ad and tracking requests before they load, hide leftover ad boxes and defuse anti-adblock scripts, so pages are quicker and agents read less noise.</span>
        <em class="ab-count" data-adblock-count>${adBlockCountHtml(settings)}</em></div>
    </div>
    <button class="switch${on ? ' on' : ''}" type="button" role="switch" aria-checked="${on}" aria-label="Block ads and trackers" data-setting="adBlock" data-kind="boolean"></button></div>`;
};
/* GoMini, the local choice model. Not downloaded: a download button that
   becomes its own progress bar. Downloaded: the switch that activates it,
   and where it runs. */
const mb = bytes => `${Math.round((Number(bytes) || 0) / 1e6)} MB`;
const gominiCard = gomini => {
  if (!gomini) return '';
  const job = gomini.download || {};
  const progress = job.progress || {};
  const pct = progress.total ? Math.min(100, Math.floor((progress.done / progress.total) * 100)) : 0;
  const head = `<div class="gm-top"><b>GoMini</b><em class="gm-tag">${gomini.installed ? (gomini.active ? 'Active' : 'Downloaded') : 'Local model'}</em></div>
      <p>A small model on this computer takes the routine steps of a task (fill, click, scroll) so your agent reads fewer pages and spends fewer tokens. It only picks between steps the browser offers; payments, sign-ins and final submissions always come back to your agent. Nothing leaves this computer.</p>`;
  let body;
  if (job.running) {
    body = `<div class="gm-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}" aria-label="Downloading GoMini">
        <i style="--p:${pct}%"></i><span>${esc(progress.text || 'Downloading')} · ${pct}%</span></div>
      <div class="gm-row"><small>${mb(progress.done)} of ${mb(progress.total)}</small><button class="btn" type="button" data-gomini="cancel">Cancel</button></div>`;
  } else if (!gomini.installed) {
    body = `<button class="gm-download" type="button" data-gomini="download"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m0 0-5-5m5 5 5-5M5 20h14"/></svg><span>Download GoMini</span><small>${mb(gomini.downloadBytes)}</small><i class="gm-sheen" aria-hidden="true"></i></button>
      ${job.error && job.error !== 'cancelled' ? `<p class="gm-error">The download stopped: ${esc(job.error)}. Press Download to try again; it continues where it stopped.</p>` : ''}
      <small class="gm-note">Picks your graphics card if it can use it, otherwise the processor, and checks every file before it is used.</small>`;
  } else {
    const where = gomini.installation || {};
    const device = where.runtime === 'vulkan' ? (where.deviceName || 'your graphics card').replace(/\s*\(.*$/, '') : 'the processor';
    body = `<div class="setting gm-switch"><div><b>Activate GoMini</b><span>Runs on ${esc(device)} · about ${esc(String(where.decisionMs || '?'))} ms per step${gomini.error ? ` · ${esc(gomini.error)}` : ''}</span></div>
        <button class="switch${gomini.enabled ? ' on' : ''}" type="button" role="switch" aria-checked="${gomini.enabled === true}" aria-label="Activate GoMini" data-setting="gominiEnabled" data-kind="boolean"></button></div>
      <div class="gm-row"><small>${esc(gomini.model)} · ${esc(where.llamaBuild || '')}</small><button class="btn" type="button" data-gomini="remove">Remove</button></div>`;
  }
  return `<div class="setting gomini${job.running ? ' busy' : ''}${gomini.active ? ' live' : ''}" data-gomini-card>
    <div class="gm-orb" aria-hidden="true"><svg viewBox="0 0 48 48"><circle class="g1" cx="24" cy="24" r="20"/><path class="g2" d="M14 30l7-8 6 5 8-11"/><circle class="g3" cx="35" cy="16" r="2.6"/></svg></div>
    <div class="gm-main">${head}${body}</div></div>`;
};
let gominiTimer = 0;
const watchGomini = gomini => {
  clearTimeout(gominiTimer);
  if (!gomini?.download?.running && !gomini?.starting) return;
  gominiTimer = setTimeout(async () => {
    if (activeTool !== 'settings') return;
    try {
      const next = await callBridge('gomini.status');
      const card = toolBody.querySelector('[data-gomini-card]');
      if (card) card.outerHTML = gominiCard(next);
      watchGomini(next);
    } catch (error) { console.error(error.message); }
  }, 350);
};

/* The VPN is a feature we are still building: a card, not a switch. */
const vpnRow = () => `
  <div class="setting soon" data-vpn>
    <div class="soon-orb" aria-hidden="true">
      <svg viewBox="0 0 48 48"><circle class="o1" cx="24" cy="24" r="21"/><ellipse class="o2" cx="24" cy="24" rx="9" ry="21"/><path class="o3" d="M3 24h42M7 14h34M7 34h34"/><circle class="dot" cx="24" cy="3" r="2.6"/></svg>
    </div>
    <div class="soon-main">
      <div class="soon-top"><b>VPN</b><em class="soon-tag"><i></i>Coming soon</em></div>
      <p>We are working on it to delivery it to you in best way possible :)</p>
      <div class="soon-bar" aria-hidden="true"><i></i></div>
      <small>Building it in</small>
    </div>
  </div>`;let countFrame = 0;
const paintAdBlock = settings => {
  const label = toolBody.querySelector('[data-adblock-count]');
  if (!label) return;
  const button = toolBody.querySelector('[data-setting="adBlock"]');
  const wanted = settings.adBlock !== false;
  if (button && (button.getAttribute('aria-checked') === 'true') !== wanted) setAdBlockLook(button, wanted);
  const num = label.querySelector('data');
  const next = Number(settings.adBlockBlocked) || 0;
  if (settings.adBlock === false || settings.adBlockReady === false || !num) {
    label.innerHTML = adBlockCountHtml(settings);
    return;
  }
  const from = Number(num.value) || 0;
  num.value = String(next);
  cancelAnimationFrame(countFrame);
  if (from === next || reducedMotion() || label.closest('.still')) { num.textContent = next.toLocaleString(); return; }
  const start = performance.now();
  const step = now => {
    const t = Math.min(1, (now - start) / 700);
    num.textContent = Math.round(from + (next - from) * (1 - (1 - t) ** 3)).toLocaleString();
    if (t < 1) countFrame = requestAnimationFrame(step);
  };
  countFrame = requestAnimationFrame(step);
};
const refreshAdBlockCount = async () => {
  try {
    const settings = await callBridge('settings.get');
    paintAdBlock(settings);
  } catch (error) { console.error(error.message); }
};
const setAdBlockLook = (button, on) => {
  const row = button.closest('.adblock');
  button.classList.toggle('on', on);
  button.setAttribute('aria-checked', String(on));
  row.classList.toggle('on', on);
  return row;
};
const toggleAdBlock = async button => {
  const on = button.getAttribute('aria-checked') !== 'true';
  const row = setAdBlockLook(button, on);
  row.classList.remove('firing', 'dimming');
  void row.offsetWidth;
  row.classList.add(on ? 'firing' : 'dimming');
  try {
    await callBridge('settings.set', { settings: { adBlock: on } });
    await refreshAdBlockCount();
  } catch (error) {
    setAdBlockLook(button, !on);
    console.error(error.message);
  }
};

// The animated switch arrives from the browser (__agentgoThemeSync) with the
// point the person clicked; this only makes sure the theme is right.
let lastPointer = null;
addEventListener('pointerdown', event => { lastPointer = [event.clientX, event.clientY]; }, true);
const applyTheme = settings => window.agentgoSetTheme(settings.theme);const seg = (key, current, values, labels = values) => `<div class="seg">${values.map((v, i) => `<button type="button" class="${current === v ? 'on' : ''}" data-setting="${key}" data-value="${v}">${esc(labels[i])}</button>`).join('')}</div>`;
const LANGUAGES = [['', 'Windows default'], ['en-US,en', 'English (US)'], ['en-GB,en', 'English (UK)'], ['ar,en', 'العربية'], ['fr,en', 'Français'], ['de,en', 'Deutsch'], ['es,en', 'Español'], ['tr,en', 'Türkçe'], ['ru,en', 'Русский'], ['zh-CN,zh,en', '中文'], ['ja,en', '日本語']];
const renderSettings = (settings, version, gomini) => {
  applyTheme(settings);
  const language = settings.languages || '';
  toolBody.innerHTML = `
    <section class="section" data-section="appearance"><h3>Appearance</h3>
      <div class="setting"><div><b>Theme</b><span>Applies to the browser chrome, panels and the New Tab page.</span></div>
        <div class="seg">${['light', 'dark', 'system'].map(v => `<button type="button" class="${(settings.theme || 'dark') === v ? 'on' : ''}" data-setting="theme" data-value="${v}">${v}</button>`).join('')}</div></div>
      ${siteDarkCard(settings)}
      ${toggle('motion', settings.motion !== false, 'Motion', 'The live wire, sparks and transitions. Off keeps everything still.')}
    </section>
    <section class="section" data-section="toolbar"><h3>Toolbar</h3>
      ${toggle('favoritesBar', settings.favoritesBar !== false, 'Favorites bar', 'Your starred sites in a row under the address bar (Ctrl+Shift+B).')}
      ${toggle('showHome', settings.showHome !== false, 'Home button', 'A button that opens the New Tab page.')}
      ${toggle('showExtensions', settings.showExtensions !== false, 'Extensions button', 'The puzzle piece with your extensions. Pin an extension there to give it its own button.')}
      <div class="setting"><div><b>Manage extensions</b><span>Turn extensions on or off, choose which ones show in the toolbar, or remove them.</span></div>
        <button class="btn" type="button" data-open-extensions>Manage</button></div>
    </section>
    <section class="section" data-section="agent"><h3>Agent control</h3>
      <div class="setting"><div><b>Control mode</b><span>Approval asks you before final form submissions. Full access does not. Purchases and payments ask you in both.</span></div>
        <div class="seg">${['approval', 'full'].map(v => `<button type="button" class="${(settings.controlMode || 'approval') === v ? 'on' : ''}" data-setting="controlMode" data-value="${v}">${v}</button>`).join('')}</div></div>
      <div class="setting"><div><b>Tabs agents open</b><span>Auto keeps them behind your tab while you are using the window. Background always does. Follow always shows them.</span></div>
        <div class="seg">${['auto', 'background', 'follow'].map(v => `<button type="button" class="${(settings.agentTabs || 'auto') === v ? 'on' : ''}" data-setting="agentTabs" data-value="${v}">${v}</button>`).join('')}</div></div>
      ${toggle('agentAnimations', settings.agentAnimations !== false, 'Show what agents do', 'An agent pointer, live typing and short notices on the page. Turn off for the fastest runs.')}
      ${toggle('offerToSavePasswords', settings.offerToSavePasswords !== false, 'Offer to save sign-ins', 'Ask after you or an agent signs in or signs up on a site.')}
      ${toggle('captchaNotifications', settings.captchaNotifications !== false, 'Notify me when an agent needs me', 'Windows notification for CAPTCHAs, logins and approvals.')}
      ${toggle('downloadsEnabled', settings.downloadsEnabled === true, 'Allow downloads', 'Off by default. Files go to a scoped folder and are never opened automatically.')}
    </section>
    <section class="section" data-section="gomini"><h3>GoMini</h3>
      ${gominiCard(gomini)}
    </section>
    <section class="section" data-section="startup"><h3>On startup</h3>
      <div class="setting"><div><b>When you open AgentGo</b><span>Continue where you left off reopens the tabs of your last session.</span></div>
        ${seg('startup', settings.startup === 'newtab' ? 'newtab' : 'restore', ['restore', 'newtab'], ['continue', 'new tab'])}</div>
    </section>
    <section class="section" data-section="search"><h3>Search engine</h3>
      <div class="setting"><div><b>Address bar and New Tab</b><span>What a typed search opens. Agents use the same engine for navigate.search.</span></div>
        ${seg('searchEngine', ['Bing', 'DuckDuckGo'].includes(settings.searchEngine) ? settings.searchEngine : 'Google', ['Google', 'Bing', 'DuckDuckGo'])}</div>
    </section>
    <section class="section" data-section="privacy"><h3>Privacy</h3>
      ${adBlockRow(settings)}
      ${toggle('dangerousSiteProtection', settings.dangerousSiteProtection !== false, 'Block dangerous sites', 'Warns before malware, phishing and look-alike sites open. Checked on this computer; no address is sent anywhere.')}
      ${toggle('blockThirdPartyCookies', settings.blockThirdPartyCookies !== false, 'Block third-party cookies', 'Sites you visit cannot read or set the cookies of other sites embedded in them, the classic cross-site tracker.')}
      ${toggle('stripTracking', settings.stripTracking !== false, 'Remove tracking from links', 'Click IDs such as fbclid, gclid and utm_ tags are removed from addresses before a page is requested.')}
      ${toggle('fingerprintProtection', settings.fingerprintProtection !== false, 'Fingerprint protection', 'New noise every session and on every site while you browse; an agent gets its own fingerprint that never matches yours.')}
      ${vpnRow(settings)}
      <div class="setting"><div><b>Clear browsing data</b><span>Deletes cookies (signs you out of sites), the cache and the storage of sites you visited recently. History has its own button in History.</span></div>
        <button class="btn" type="button" data-clear-data>Clear data</button></div>
      ${permissionRow()}
    </section>
    <section class="section" data-section="data"><h3>Setup and help</h3>
      <div class="setting"><div><b>Import from another browser</b><span>Bring bookmarks, history, saved passwords and open tabs over from Chrome, Edge, Brave or Firefox.</span></div>
        <button class="btn" type="button" data-f2="import">Import</button></div>
      <div class="setting"><div><b>Connect an agent</b><span>Add AgentGo to Claude Code, Codex or Factory Droid with one click.</span></div>
        <button class="btn" type="button" data-f2="agent">Connect</button></div>
      <div class="setting"><div><b>Command palette</b><span>Search tabs, history, bookmarks, settings and every browser action, or hand a task to your agent.</span></div>
        <button class="btn" type="button" data-f2="palette">Ctrl+K</button></div>
      <div class="setting"><div><b>Report a problem</b><span>Bundles recent logs and a screenshot, hides secrets, and shows you everything before it is saved.</span></div>
        <button class="btn" type="button" data-f2="report">Report</button></div>
    </section>
    <section class="section" data-section="languages"><h3>Languages</h3>
      <div class="setting"><div><b>Preferred language for pages</b><span>Sent to sites as Accept-Language. Takes effect the next time AgentGo starts.</span></div>
        <select class="btn" data-select="languages" aria-label="Preferred language">${LANGUAGES.map(([value, label]) => `<option value="${esc(value)}"${value === language ? ' selected' : ''}>${esc(label)}</option>`).join('')}</select></div>
      ${translateRow(settings)}
    </section>
    <section class="section" data-section="downloads"><h3>Downloads</h3>
      <div class="setting"><div><b>Download folder</b><span>Files go to AgentGo's own folder for this profile and are never opened automatically.</span></div>
        <button class="btn" type="button" data-open-folder>Open folder</button></div>
    </section>
    <section class="section" data-section="system"><h3>System</h3>
      ${toggle('hardwareAcceleration', settings.hardwareAcceleration !== false, 'Hardware acceleration', 'Uses the GPU for rendering. Takes effect the next time AgentGo starts.')}
    </section>
    <section class="section" data-section="about"><h3>About</h3>
      <div class="setting"><div><b>AgentGo ${esc(version.agentgo)}</b><span>Chromium ${esc(version.chromium)} · protocol ${esc(version.protocol)} · no telemetry, no model calls.</span></div><span class="value">Local only</span></div>
      ${toggle('autoUpdate', settings.autoUpdate !== false, 'Update automatically', 'New versions download in the background, are checked against the release signature and install when you close AgentGo. Off: an Update button appears in the toolbar when a new version is out.')}
      <div class="setting"><div><b>Check for updates</b><span id="updateStatus">Releases come signed from github.com/Moha2005269/agentgo.</span></div>
        <button class="btn" type="button" data-update-check>Check now</button></div>
    </section>`;
  toolFoot.hidden = true;
  settingsCache = settings;
  buildSearchIndex();
  applySearch(false);
  if (focusedFor !== openSequence) {
    focusedFor = openSequence;
    toolFilter.focus({ preventScroll: true });
  }
};

/* ---------- settings search ---------- */
// Typing finds settings by what they are called, what they do, and the words
// people use for them (ads, vpn, dark, passwords). Near misses and typos still
// land on the right row.
let settingsCache = {};
let searchIndex = [];
let lastQuery = '';
let focusedFor = -1;
const searchBox = $('#toolSearch');
const searchCount = $('#searchCount');
const SEARCH_KEYWORDS = [
  [/theme/i, 'dark light mode color colour appearance night day look'],
  [/motion/i, 'animation animations reduce effects transitions movement'],
  [/control mode/i, 'agent permission approval full access ai autonomy confirm'],
  [/tabs agents/i, 'background foreground window follow'],
  [/gomini|activate gomini/i, 'model local ai tokens laya small model download choice faster cheaper'],
  [/show what agents/i, 'pointer cursor typing animation overlay'],
  [/sign-ins/i, 'password passwords save login logins credentials remember'],
  [/notify/i, 'notification notifications captcha alert alerts toast'],
  [/allow downloads/i, 'files save download'],
  [/when you open/i, 'startup start launch restore tabs session continue'],
  [/address bar/i, 'search engine google bing duckduckgo omnibox'],
  [/block ads/i, 'adblock adblocker ad blocker ads advert advertising popups popup trackers tracking easylist easyprivacy privacy clean'],
  [/favorites bar/i, 'bookmarks bookmark favourites favorite star starred sites links shortcuts toolbar row'],
  [/home button/i, 'home start page new tab button toolbar'],
  [/extensions button/i, 'addons add-ons plugins puzzle chrome web store pin toolbar'],
  [/manage extensions/i, 'addons add-ons plugins chrome web store install remove disable enable pin show hide toolbar'],
  [/clear browsing/i, 'cookies cache history delete wipe clean storage forget erase'],
  [/site permissions/i, 'camera microphone mic location notifications geolocation block'],
  [/preferred language/i, 'language locale translate accept-language english arabic'],
  [/download folder/i, 'files directory path location'],
  [/hardware accel/i, 'gpu graphics performance rendering speed render'],
  [/^agentgo \d/i, 'about version update chromium build info'],
];
const SEARCH_HINTS = ['ads and trackers', 'dark theme', 'passwords', 'language', 'what agents can do', 'downloads'];
const norm = text => String(text).toLowerCase().normalize('NFKD').replace(/[\u0300-\u036f]/g, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
const wordsOf = text => norm(text).split(' ').filter(Boolean);
// Edit distance that counts a swapped pair of letters as one slip.
const distance = (a, b) => {
  const m = a.length, n = b.length;
  if (Math.abs(m - n) > 2) return 3;
  const d = Array.from({ length: m + 1 }, (_, i) => { const row = new Array(n + 1).fill(0); row[0] = i; return row; });
  for (let j = 0; j <= n; j += 1) d[0][j] = j;
  for (let i = 1; i <= m; i += 1) for (let j = 1; j <= n; j += 1) {
    d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
  }
  return d[m][n];
};
// 1 is the same word, .88 a prefix, .62 inside a word, below .6 a near miss.
const tokenScore = (token, words) => {
  let best = 0;
  for (const word of words) {
    let score = 0;
    if (word === token) score = 1;
    else if (word.startsWith(token)) score = .88 - Math.min(.18, (word.length - token.length) * .015);
    else if (token.length >= 3 && word.includes(token)) score = .62;
    else if (token.length >= 3) {
      const limit = token.length <= 4 ? 1 : 2;
      const slip = Math.min(distance(token, word), token.length >= 4 ? distance(token, word.slice(0, token.length)) + .5 : 9);
      if (slip <= limit) score = .56 - .13 * slip;
    }
    if (score > best) best = score;
  }
  return best;
};
const FIELDS = [['tw', 1], ['kw', .85], ['sw', .5], ['dw', .35]];
const scoreEntry = (entry, tokens) => {
  let total = 0, exact = true;
  for (const token of tokens) {
    let best = 0, raw = 0;
    for (const [field, weight] of FIELDS) {
      const s = tokenScore(token, entry[field]);
      raw = Math.max(raw, s);
      best = Math.max(best, s * weight);
    }
    if (raw < .6) exact = false;
    total += best;
  }
  return { score: total / tokens.length, exact };
};
const buildSearchIndex = () => {
  searchIndex = [];
  let n = 0;
  toolBody.querySelectorAll(':scope > .section').forEach((section, number) => {
    const sectionName = section.querySelector('h3')?.textContent || '';
    section.querySelectorAll(':scope > .setting').forEach(row => {
      const title = row.querySelector('b')?.textContent || '';
      const desc = row.querySelector('span')?.textContent || '';
      const keywords = SEARCH_KEYWORDS.filter(([pattern]) => pattern.test(title)).map(([, words]) => words).join(' ');
      row.dataset.sid = String(n);
      searchIndex.push({ sid: n, row, section: sectionName, number: number + 1, title, tw: wordsOf(title), dw: wordsOf(desc), kw: wordsOf(keywords), sw: wordsOf(sectionName) });
      n += 1;
    });
  });
};
const markText = (root, tokens) => {
  const wanted = tokens.filter(token => token.length >= 2);
  if (!wanted.length) return;
  const pattern = new RegExp(`(${wanted.join('|')})`, 'giu');
  const holders = root.querySelectorAll('.setting b, .setting > div > span, .setting > .ab-main span');
  for (const holder of holders) {
    if (holder.closest('button') || holder.classList.contains('value')) continue;
    const walker = document.createTreeWalker(holder, NodeFilter.SHOW_TEXT);
    const nodes = [];
    while (walker.nextNode()) nodes.push(walker.currentNode);
    for (const node of nodes) {
      if (!pattern.test(node.nodeValue)) continue;
      pattern.lastIndex = 0;
      const fragment = document.createDocumentFragment();
      node.nodeValue.split(pattern).forEach((piece, i) => {
        if (i % 2) { const mark = document.createElement('mark'); mark.textContent = piece; fragment.append(mark); }
        else if (piece) fragment.append(piece);
      });
      node.replaceWith(fragment);
    }
  }
};
const searchIsStill = () => reducedMotion() || settingsCache.motion === false;
const setCount = (text) => {
  if (searchCount.textContent === text) return;
  searchCount.textContent = text;
  searchCount.classList.remove('pop');
  void searchCount.offsetWidth;
  if (text) searchCount.classList.add('pop');
};
const hitHtml = (entry, index, strength) => {
  const clone = entry.row.cloneNode(true);
  clone.removeAttribute('data-sid');
  const hit = document.createElement('div');
  hit.className = 'hit';
  hit.style.setProperty('--i', String(index));
  hit.append(clone);
  const tag = document.createElement('div');
  tag.className = 'hit-tag';
  tag.innerHTML = `<span>${String(entry.number).padStart(2, '0')} · ${esc(entry.section)}</span><span class="meter-s" style="--m:${Math.round(strength * 100)}%"><u></u></span><button type="button" data-jump="${entry.sid}">Show in settings ↗</button>`;
  hit.append(tag);
  return hit;
};
const emptyHtml = query => `
  <div class="res-empty"><svg viewBox="0 0 54 54"><circle cx="23" cy="23" r="15"/><path d="m34 34 14 14"/><path d="M17 27c3 3 9 3 12 0"/><path d="M18 19v2M28 19v2"/></svg>
    <b>Nothing close to “${esc(query)}”</b>Try one of these instead
    <div class="chips">${['ads', 'dark', 'passwords', 'language', 'agents'].map(word => `<button class="chip" type="button" data-chip="${word}">${word}</button>`).join('')}</div></div>`;
const applySearch = (animate = true) => {
  const query = toolFilter.value.trim();
  toolBody.querySelector(':scope > .results')?.remove();
  searchBox.classList.toggle('has', query.length > 0);
  searchBox.classList.toggle('still', searchIsStill());
  const previous = lastQuery;
  lastQuery = query;
  if (!query) {
    toolBody.classList.remove('searching');
    setCount('');
    // Switches flipped inside the results are stale in the hidden list.
    if (previous) refresh();
    return;
  }
  const tokens = wordsOf(query);
  if (!tokens.length) { toolBody.classList.remove('searching'); setCount(''); return; }
  const scored = searchIndex.map(entry => ({ entry, ...scoreEntry(entry, tokens) }));
  const exact = scored.filter(item => item.exact).sort((a, b) => b.score - a.score);
  const related = scored.filter(item => !item.exact && item.score >= .3).sort((a, b) => b.score - a.score).slice(0, exact.length ? 4 : 6);
  const results = document.createElement('div');
  results.className = `results${animate ? '' : ' quiet'}${searchIsStill() ? ' still' : ''}`;
  let index = 0;
  const addGroup = (items, heading) => {
    if (heading) results.insertAdjacentHTML('beforeend', heading);
    for (const { entry, score } of items) results.append(hitHtml(entry, index++, Math.min(1, .25 + score)));
  };
  if (exact.length) {
    addGroup(exact, `<div class="res-head"><span>Best <em>${exact.length === 1 ? 'match' : 'matches'}</em></span></div>`);
    if (related.length) addGroup(related, '<div class="res-head"><span>Related</span></div>');
  } else if (related.length) {
    results.insertAdjacentHTML('beforeend', `<div class="res-did">No exact match for <b>“${esc(query)}”</b>. Closest settings:</div>`);
    addGroup(related, '');
  } else {
    results.insertAdjacentHTML('beforeend', emptyHtml(query));
  }
  markText(results, tokens);
  results.querySelector('.hit')?.classList.add('sel');
  toolBody.classList.add('searching');
  toolBody.prepend(results);
  if (animate || query !== previous) toolBody.scrollTop = 0;
  const total = exact.length + related.length;
  setCount(!total ? 'none' : exact.length ? `${exact.length} found` : `${total} close`);
};
const jumpToSetting = async sid => {
  toolFilter.value = '';
  applySearch(false);
  await refresh();
  const row = toolBody.querySelector(`[data-sid="${CSS.escape(String(sid))}"]`);
  if (!row) return;
  row.scrollIntoView({ block: 'center', behavior: searchIsStill() ? 'auto' : 'smooth' });
  row.classList.remove('flash');
  void row.offsetWidth;
  row.classList.add('flash');
};
const moveSelection = step => {
  const hits = [...toolBody.querySelectorAll('.results .hit')];
  if (!hits.length) return;
  const at = hits.findIndex(hit => hit.classList.contains('sel'));
  hits.forEach(hit => hit.classList.remove('sel'));
  const next = hits[(at + step + hits.length) % hits.length];
  next.classList.add('sel');
  next.scrollIntoView({ block: 'nearest', behavior: searchIsStill() ? 'auto' : 'smooth' });
};
// The placeholder types out a few ideas while the box is idle.
let hintTimer = 0;
const runHints = sequence => {
  clearTimeout(hintTimer);
  const idle = () => sequence === openSequence && activeTool === 'settings' && !toolFilter.value && document.activeElement !== toolFilter;
  toolFilter.placeholder = 'Search settings';
  if (searchIsStill()) return;
  let word = 0, at = 0, deleting = false;
  const tick = () => {
    if (sequence !== openSequence || activeTool !== 'settings') return;
    if (!idle()) { toolFilter.placeholder = 'Search settings'; hintTimer = setTimeout(tick, 600); return; }
    const text = SEARCH_HINTS[word];
    if (!deleting) {
      at += 1;
      toolFilter.placeholder = `Search settings: ${text.slice(0, at)}`;
      if (at === text.length) { deleting = true; hintTimer = setTimeout(tick, 1500); return; }
      hintTimer = setTimeout(tick, 55);
    } else {
      at -= 1;
      toolFilter.placeholder = at ? `Search settings: ${text.slice(0, at)}` : 'Search settings';
      if (!at) { deleting = false; word = (word + 1) % SEARCH_HINTS.length; hintTimer = setTimeout(tick, 350); return; }
      hintTimer = setTimeout(tick, 22);
    }
  };
  hintTimer = setTimeout(tick, 1200);
};
$('#searchClear').addEventListener('click', event => {
  event.preventDefault();
  toolFilter.value = '';
  applySearch(true);
  toolFilter.focus();
});
toolFilter.addEventListener('keydown', event => {
  if (activeTool !== 'settings') return;
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); moveSelection(event.key === 'ArrowDown' ? 1 : -1); }
  else if (event.key === 'Enter') {
    const sid = toolBody.querySelector('.results .hit.sel [data-jump]')?.dataset.jump;
    if (sid !== undefined) { event.preventDefault(); jumpToSetting(sid); }
  } else if (event.key === 'Escape' && toolFilter.value) {
    event.preventDefault();
    event.stopPropagation();
    toolFilter.value = '';
    applySearch(true);
  }
}, true);
// Typing anywhere in Settings starts a search.
addEventListener('keydown', event => {
  if (activeTool !== 'settings' || closing || event.ctrlKey || event.metaKey || event.altKey || event.key.length !== 1) return;
  if (event.target.closest?.('input, select, textarea')) return;
  if (event.key === ' ' && document.activeElement?.closest?.('button')) return;
  toolFilter.focus({ preventScroll: true });
});

/* ---------- agent ---------- */
// What a shell.* command asked the panel to show: {section, data}.
const shellData = () => window.__agentgoShellData || {};
const STEP_MARK = { done: '✓', current: '›', failed: '×', pending: '·' };
const renderTask = task => {
  if (!task || !task.taskId) return '';
  const plan = task.plan || [];
  const results = task.results || [];
  const failedAt = task.status === 'failed' ? task.cursor : -1;
  const steps = plan.map((step, index) => {
    const state = index === failedAt ? 'failed' : index < task.cursor ? 'done' : index === task.cursor && task.status === 'running' ? 'current' : 'pending';
    const removable = index >= task.cursor && task.status !== 'running';
    return `<li class="step ${state}"><span>${STEP_MARK[state]}</span><span><em>${esc(step.method)}</em> ${esc(step.risk)}</span>${removable ? `<button type="button" title="Remove this step" aria-label="Remove step ${index + 1}" data-plan-remove="${index}" data-task="${esc(task.taskId)}">×</button>` : '<span></span>'}</li>`;
  }).join('');
  const budget = task.budget || {};
  const used = task.used || {};
  const cost = `${used.steps || 0}${budget.maxSteps ? `/${budget.maxSteps}` : ''} steps · ${((used.ms || 0) / 1000).toFixed(1)}${budget.maxMs ? `/${(budget.maxMs / 1000).toFixed(0)}` : ''} s`;
  const note = task.note ? `<small>Your note: ${esc(task.note)}</small>` : '';
  const error = task.error ? `<div class="task-error"><b>${esc(task.error.code)}</b> at step ${Number(task.error.step) + 1}: ${esc(task.error.message)}${note}</div>` : (note ? `<div class="task-error">${note}</div>` : '');
  const retry = ['failed', 'budget', 'interrupted'].includes(task.status) ? `<button class="btn primary" type="button" data-task-retry="${esc(task.taskId)}">Retry</button>` : '';
  const acts = task.status === 'running' ? '' : `<div class="acts">${retry}<button class="btn" type="button" data-task-replan="${esc(task.taskId)}">Ask the agent to replan</button></div>`;
  return `<div class="task" data-section="task">
    <div class="task-goal">${esc(task.goal || 'Untitled task')}</div>
    <div class="task-meta"><span>${esc(task.status)}</span><span>step ${Math.min(task.cursor + 1, plan.length)} of ${plan.length}</span><span>${esc(cost)}</span><span>${results.filter(r => r.tries > 1).length} retried</span></div>
    <div class="meter"><i style="width:${plan.length ? Math.round(100 * task.cursor / plan.length) : 0}%"></i></div>
    <ol class="steps" style="margin-top:10px">${steps}</ol>${error}${acts}</div>`;
};
const latestTask = async () => {
  const wanted = shellData().data?.taskId;
  const list = (await callBridge('agent.monitor')).tasks || [];
  const pick = wanted ? list.find(t => t.taskId === wanted)
    : list.find(t => t.status === 'running') || [...list].sort((a, b) => String(b.taskId).localeCompare(String(a.taskId)))[0];
  return pick ? callBridge('agent.monitor', { taskId: pick.taskId }) : null;
};
const fileName = path => String(path || '').split(/[\\/]/).pop();
const renderExtras = () => {
  const { section, data } = shellData();
  if (section === 'diff') return `<div class="label" data-section="diff">Last change on the page</div><div class="diff">${esc(data?.changes || 'Nothing changed since the last action.')}</div>`;
  if (section === 'artifacts') return `<div class="label" data-section="artifacts">Artifacts</div>${(data?.artifacts || []).map(a => `<div class="row"><span><span class="row-title">${esc(fileName(a.path))}</span><span class="row-meta">${esc(bytes(a.bytes ?? a.size))} · ${esc(a.path)}</span></span></div>`).join('') || '<p class="acc-note">No artifacts yet.</p>'}`;
  if (section === 'workflows') return `<div class="label" data-section="workflows">Recorded workflows</div>${(data?.workflows || []).map(w => `<div class="row"><span><span class="row-title">${esc(w.name)}</span><span class="row-meta">${w.steps} steps${w.disabled ? ' · disabled' : ''}</span></span><span class="row-side"><span class="tag ${w.approved ? 'on' : ''}">${w.approved ? 'Approved' : 'Not approved'}</span>${w.approved || w.disabled ? '' : `<button class="btn sm" type="button" data-wf-approve="${esc(w.name)}">Approve</button>`}</span></div>`).join('') || '<p class="acc-note">No recordings yet. Agents record with workflow.record.</p>'}`;
  if (section === 'skills') return `<div class="label" data-section="skills">Local skills</div>${(data?.skills || []).map(k => `<div class="row"><span><span class="row-title">${esc(k.name)} · ${esc(k.version)}</span><span class="row-meta">${esc(k.description || '')}${k.domain ? ` · ${esc(k.domain)}` : ''} · ${k.steps} steps</span></span><span class="row-side"><button class="btn sm" type="button" data-skill-toggle="${esc(k.name)}" data-enabled="${k.enabled !== false}">${k.enabled !== false ? 'Disable' : 'Enable'}</button></span></div>`).join('') || '<p class="acc-note">No skills installed. Agents add them with skill.install_local or skill.compile.</p>'}`;
  return '';
};
const renderAgent = (status, connection, task) => {
  const control = status.control;
  const sessions = status.sessions || [];
  const stateCopy = {
    running: ['Agents may act', sessions.length ? `${sessions.length} connected` : 'No agent is connected right now.'],
    paused: ['Paused', 'Agent actions are waiting. Sessions and tabs are kept.'],
    stopped: ['Stopped', 'Agent input is revoked until you allow it again.'],
  }[control];
  const controls = control === 'running'
    ? `<button class="btn" type="button" data-agent="agent.pause">Pause</button><button class="btn danger" type="button" data-agent="agent.stop">Stop agent</button>`
    : `<button class="btn good" type="button" data-agent="agent.resume">${control === 'stopped' ? 'Allow agent' : 'Resume'}</button>${control === 'paused' ? '<button class="btn danger" type="button" data-agent="agent.stop">Stop agent</button>' : ''}`;
  const handoffs = (status.handoffs || []).map(h => `
    <div class="need"><b>Your turn: ${esc(h.reason)}</b><p>Complete it in the page yourself. The agent is waiting and cannot continue on this tab until you resume it.</p>
      <div class="acts"><button class="btn" type="button" data-handoff="handoff.view" data-tab="${esc(h.tabId)}">Show the tab</button><button class="btn primary" type="button" data-handoff="handoff.resolve" data-tab="${esc(h.tabId)}">I'm done — resume</button></div></div>`).join('');
  const approvals = (status.approvals || []).map(a => `
    <div class="need"><b>Approve this action?</b><p>${esc(a.reason)}<br><code>${esc(a.target)}</code> on <code>${esc(a.origin)}</code> · requested by ${esc(a.session)}</p>
      <div class="acts"><button class="btn" type="button" data-approval="${esc(a.approvalId)}" data-approve="false">Deny</button><button class="btn primary" type="button" data-approval="${esc(a.approvalId)}" data-approve="true">Approve once</button></div></div>`).join('');
  const sessionRows = sessions.map(s => `
    <div class="row"><div><div class="row-title">${esc(s.client)}</div><div class="row-meta">${esc(s.transport)} · ${s.actions} action${s.actions === 1 ? '' : 's'} · ${s.tabs.length} tab${s.tabs.length === 1 ? '' : 's'}${s.last ? ` · last ${esc(s.last)}` : ''}</div></div><span class="row-side">${esc(s.session)}</span></div>`).join('');
  const activity = (status.activity || []).slice(0, 14).map(a => `
    <div class="act"><span>${esc(clock(a.at))}</span><span class="m"><em>${esc(a.method)}</em> ${esc(a.summary)}</span><span class="${a.ok ? '' : 'no'}">${a.ok ? 'ok' : 'refused'}</span></div>`).join('');
  toolBody.innerHTML = `
    <div class="state ${control}"><i></i><div><b>${esc(stateCopy[0])}</b><span>${esc(stateCopy[1])}</span></div><div class="acts">${controls}</div></div>
    <div data-section="handoff">${handoffs}${approvals}</div>
    ${renderTask(task)}${renderExtras()}
    ${sessions.length ? `<div class="label">Connected</div>${sessionRows}` : ''}
    ${activity ? `<div class="label">Recent actions</div>${activity}` : ''}
    <div class="label">Connect an agent</div>
    <div class="code">Claude Code: <b>claude mcp add agentgo -- agentgo mcp</b><br>Any shell: <b>agentgo call observe.page</b><br>Pipe: <b>${esc(connection.pipe)}</b></div>`;
  toolFoot.hidden = true;
};

/* ---------- account center ---------- */
let accounts = [];
let credentials = [];
let linking = null;    // account whose full-control warning is on screen
let connecting = null; // account whose API sign-in form is on screen
const renderAccounts = () => {
  const warning = (account, extra, confirm) => `
    <div class="need" role="alertdialog" aria-label="Link ${esc(account.name)}">
      <b>Give agents full control of ${esc(account.name)}?</b>
      <p>${esc(account.warning)}</p>
      ${extra}
      <div class="acts">${confirm}<button class="btn" type="button" data-acc-cancel>Cancel</button></div>
    </div>`;
  const rows = accounts.map(account => {
    if (linking === account.account) {
      return warning(account,
        '<p>After you confirm, the sign-in page opens in a new tab. You sign in yourself; an agent never sees your password.</p>',
        `<button class="btn danger" type="button" data-acc-confirm="${esc(account.account)}">I understand, link it</button>`);
    }
    if (connecting === account.account) {
      return warning(account, `
        <p>The API (${esc(account.connectors.join(', '))}) is reached with an OAuth application you register with ${esc(account.name)} as a desktop app. Its redirect address is <code>http://127.0.0.1</code> (any port). Tokens are encrypted for your Windows account and never shown to agents.</p>
        <form class="acc-add" id="accConnect" data-account="${esc(account.account)}">
          <input name="clientId" placeholder="Client ID" required aria-label="Client ID" autocomplete="off">
          <input name="clientSecret" type="password" placeholder="Client secret (only if the provider gives one)" aria-label="Client secret" autocomplete="off">
          <button class="btn danger" type="submit">I understand, connect the API</button>
        </form>`, '');
    }
    const api = account.api;
    const state = account.blocked ? 'agents cannot use this account' : account.linked ? `signed in here since ${esc(when(account.linkedAt))}` : 'agents may use it when you are signed in';
    const apiState = !api ? '' : api.connected ? ' · API connected' : api.error ? ` · API refused: ${esc(api.error)}` : '';
    return `
    <div class="row">
      <span><span class="row-title">${esc(account.name)}</span><span class="row-meta">${state}${apiState}</span></span>
      <span class="row-side"><span class="tag ${account.blocked ? 'off' : 'on'}">${account.blocked ? 'Blocked' : 'Allowed'}</span>
        <button class="btn sm" type="button" data-acc-block="${esc(account.account)}" data-blocked="${account.blocked}" title="${account.blocked ? 'Let agents use this account again' : 'Stop agents from using this account'}">${account.blocked ? 'Unblock' : 'Block'}</button>
        ${api && !api.connected ? `<button class="btn sm" type="button" data-acc-connect="${esc(account.account)}" title="Let agents call ${esc(account.connectors.join(', '))}">API</button>` : ''}
        ${account.linked
          ? `<button class="btn sm" type="button" data-acc-unlink="${esc(account.account)}">Unlink</button>`
          : `<button class="btn sm primary" type="button" data-acc-link="${esc(account.account)}">Sign in</button>`}
        ${account.custom && !account.linked ? `<button class="btn sm" type="button" data-acc-forget="${esc(account.account)}" title="Remove from this list">Remove</button>` : ''}</span>
    </div>`;
  }).join('');
  const saved = credentials.map(item => `
    <div class="row">
      <span><span class="row-title">${esc(item.username)}</span><span class="row-meta">${esc(item.origin)}</span></span>
      <span class="row-side"><button class="btn sm" type="button" data-cred-delete="${esc(item.origin)}">Delete</button></span>
    </div>`).join('');
  toolBody.innerHTML = `
    <p class="acc-note">Agents work in your real browser profile and can use the accounts you are signed in to. Block an account to keep agents out of it. Unlink signs it out of AgentGo.</p>
    ${rows}
    <div class="label">Add another site</div>
    <form class="acc-add" id="accAdd">
      <input name="name" placeholder="Name, for example Indeed" maxlength="40" required aria-label="Site name">
      <input name="signInUrl" placeholder="Sign-in address (https://...)" required aria-label="Sign-in address">
      <input name="cookie" placeholder="Session cookie name" maxlength="64" required aria-label="Session cookie name">
      <button class="btn" type="submit">Add site</button>
    </form>
    <div class="label">Sign-ins the browser may type for agents</div>
    <p class="acc-note">A saved sign-in is typed into its own site's login form when an agent asks. The agent never receives the password. It is encrypted for your Windows account.</p>
    ${saved}
    <form class="acc-add" id="credAdd">
      <input name="origin" placeholder="Site address (https://...)" required aria-label="Site address">
      <input name="username" placeholder="User name or e-mail" required aria-label="User name" autocomplete="off">
      <input name="password" type="password" placeholder="Password" required aria-label="Password" autocomplete="new-password">
      <button class="btn" type="submit">Save sign-in</button>
    </form>`;
  toolFoot.hidden = true;
};

/* ---------- find bar ---------- */
let findText = '';
const renderFind = () => {
  toolBody.innerHTML = `
    <form class="findbar" id="findForm">
      <input id="findText" placeholder="Find in page" aria-label="Find in page" autocomplete="off" spellcheck="false">
      <span class="count" id="findCount" aria-live="polite"></span>
      <button class="ib" type="button" data-find="prev" title="Previous (Shift+Enter)" aria-label="Previous match"><svg viewBox="0 0 12 12"><path d="m3 7.5 3-3 3 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg></button>
      <button class="ib" type="button" data-find="next" title="Next (Enter)" aria-label="Next match"><svg viewBox="0 0 12 12"><path d="m3 4.5 3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg></button>
      <button class="ib" type="button" data-find="close" title="Close (Esc)" aria-label="Close find"><svg viewBox="0 0 12 12"><path d="m2 2 8 8M10 2l-8 8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
    </form>`;
  toolFoot.hidden = true;
  const input = $('#findText');
  input.value = findText;
  input.focus();
  input.select();
  if (findText) callBridge('ui.find', { text: findText }).catch(() => {});
};
const findStep = forward => callBridge('ui.find', { text: findText, forward, next: true }).catch(() => {});
let findPrimed = '';
window.__agentgoFindResult = (count, ordinal) => {
  const label = $('#findCount');
  if (!label) return;
  // A new search counts the matches but selects none; one step forward
  // selects the first, as in Chrome.
  if (count > 0 && ordinal < 1 && findPrimed !== findText) {
    findPrimed = findText;
    findStep(true);
    return;
  }
  label.textContent = findText ? `${count ? ordinal : 0}/${count}` : '';
  label.classList.toggle('none', Boolean(findText) && count === 0);
};

/* ---------- menu ---------- */
const renderMenu = async () => {
  const tabs = await callBridge('tabs.list');
  if (activeTool !== 'menu') return;
  const tab = (tabs.tabs || []).find(item => item.tabId === tabs.activeTabId);
  const zoom = Math.round(tab?.zoom || 100);
  const strip = tab?.strip || {};
  const web = /^https?:/i.test(tab?.url || '');
  const item = (action, label, keys = '') => `<button class="menu-item" type="button" data-menu="${action}"><span>${esc(label)}</span><kbd>${esc(keys)}</kbd></button>`;
  toolBody.innerHTML = `
    ${item('newtab', 'New tab', 'Ctrl+T')}
    <div class="menu-sep"></div>
    ${item('history', 'History', 'Ctrl+H')}
    ${item('downloads', 'Downloads', 'Ctrl+J')}
    ${item('bookmarks', 'Bookmarks')}
    ${item('extensions', 'Extensions')}
    ${item('accounts', 'Accounts and passwords')}
    <div class="menu-sep"></div>
    <div class="menu-zoom"><span>Zoom</span><div class="seg"><button type="button" data-menu="zoom-out" aria-label="Zoom out">−</button><span id="menuZoom">${zoom}%</span><button type="button" data-menu="zoom-in" aria-label="Zoom in">+</button><button type="button" data-menu="zoom-reset" title="Reset to 100 %" aria-label="Reset zoom">↺</button></div></div>
    <div class="menu-sep"></div>
    ${item('find', 'Find in page', 'Ctrl+F')}
    ${item('print', 'Print', 'Ctrl+P')}
    ${web ? item('reader', 'Reader view') : ''}
    ${web ? item('translate', 'Translate page') : ''}
    <div class="menu-sep"></div>
    ${item(strip.pinned ? 'unpin' : 'pin', strip.pinned ? 'Unpin tab' : 'Pin tab')}
    ${item(strip.group ? 'ungroup' : 'group', strip.group ? `Remove from group “${strip.group.name}”` : 'Add tab to a new group')}
    <div class="menu-sep"></div>
    ${item('settings', 'Settings')}
    ${item('exit', 'Exit')}`;
  toolBody.dataset.zoom = String(zoom);
  toolBody.dataset.tab = tabs.activeTabId || '';
  toolFoot.hidden = true;
};
const ZOOM_STOPS = [25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 300, 500];
const menuAction = async action => {
  const zoom = Number(toolBody.dataset.zoom) || 100;
  const tabId = toolBody.dataset.tab;
  if (action.startsWith('zoom-')) {
    const percent = action === 'zoom-reset' ? 100
      : action === 'zoom-in' ? (ZOOM_STOPS.find(stop => stop > zoom) ?? 500)
      : ([...ZOOM_STOPS].reverse().find(stop => stop < zoom) ?? 25);
    await callBridge('page.zoom', { tabId, percent });
    await renderMenu();
    return;
  }
  if (['history', 'downloads', 'bookmarks', 'extensions', 'accounts', 'settings', 'find'].includes(action)) {
    await callBridge('ui.open_tool', { tool: action });
    return;
  }
  if (action === 'translate') {
    await callBridge('ui.run_action', { name: 'tool:translate' });
    return;
  }
  window.__agentgoRequestClose();
  if (action === 'reader') await callBridge('browser_ui.reader_mode', { tabId, view: true });
  else if (action === 'pin' || action === 'unpin') await callBridge(`tab.${action}`, { tabId });
  else if (action === 'ungroup') await callBridge('tab.ungroup', { tabId });
  else if (action === 'group') {
    const colors = ['#5B8CFF', '#E5534B', '#3FB950', '#D29922', '#A371F7', '#DB61A2'];
    const count = ((await callBridge('tabs.list')).tabs || []).filter(t => t.strip?.group).length;
    await callBridge('tab.group', { tabId, name: `Group ${count + 1}`, color: colors[count % colors.length] });
  } else if (action === 'newtab') await callBridge('tabs.create', { url: 'agentgo://shell/index.html?embedded=1' });
  else if (action === 'print') await callBridge('ui.print');
  else if (action === 'exit') await callBridge('window.close');
};

/* ---------- extensions ---------- */
// Installed extensions: which are on, and which get a button in the toolbar.
const extInitial = ext => esc((ext.name || '?').trim().charAt(0).toUpperCase() || '?');
const renderExtensions = list => {
  if (!list.length) {
    toolBody.innerHTML = `<div class="empty"><div><strong>No extensions yet</strong><p>Open an extension in the Chrome Web Store and press “Add to AgentGo”. It shows up here, and you choose whether it gets a button in the toolbar.</p></div></div>`;
  } else {
    toolBody.innerHTML = list.map((ext, i) => `
      <div class="ext${ext.enabled ? '' : ' off'}" style="--i:${i}" data-ext-row="${esc(ext.id)}">
        <span class="ext-ic">${ext.icon ? `<img alt="" src="${esc(ext.icon)}">` : extInitial(ext)}</span>
        <span class="ext-text"><b>${esc(ext.name)}</b><small>${ext.limited ? 'Page scripts only: its background part does not run in AgentGo, so some features may not work.' : esc(ext.description || `Version ${ext.version}`)}</small></span>
        <span class="ext-ctl">
          <label class="ext-pin"><span>Toolbar</span><button class="switch${ext.pinned ? ' on' : ''}" type="button" role="switch" aria-checked="${ext.pinned}" aria-label="Show ${esc(ext.name)} in the toolbar" data-ext-pin="${esc(ext.id)}"></button></label>
          <label class="ext-pin"><span>On</span><button class="switch${ext.enabled ? ' on' : ''}" type="button" role="switch" aria-checked="${ext.enabled}" aria-label="Turn ${esc(ext.name)} on or off" data-ext-on="${esc(ext.id)}"></button></label>
          ${ext.popup && ext.enabled ? `<button class="btn sm" type="button" data-ext-open="${esc(ext.id)}">Open</button>` : '}${ext.options && ext.enabled ? `<button class="btn sm" type="button" data-ext-options="${esc(ext.id)}">Options</button>` : '}
          <button class="btn sm" type="button" data-ext-remove="${esc(ext.id)}">Remove</button>
        </span>
      </div>`).join('');
  }
  toolFoot.hidden = false;
  toolFoot.innerHTML = '<span class="note">AgentGo runs the page scripts of extensions and opens their pop-ups and options pages. Background parts do not run yet, so features that need them may not work.</span><button class="btn" type="button" data-ext-store>Get extensions</button>';
};
const extensionList = async () => (await callBridge('extension.list')).extensions || [];
const extChanged = async () => { await refresh(); window.__agentgoExtensionsChanged?.(); };
/* ---------- bookmarks ---------- */
const renderBookmarks = items => {
  if (!items.length) empty('No bookmarks', 'Click the star in the address bar (Ctrl+D) to bookmark a page.');
  else toolBody.innerHTML = items.map(item => `
    <div class="row">
      <button type="button" data-open-url="${esc(item.url)}" style="min-width:0;text-align:left"><span class="row-title">${esc(item.title || item.url)}</span><span class="row-meta">${esc(item.url)}</span></button>
      <span class="row-side"><button class="btn sm" type="button" data-bookmark-remove="${esc(item.url)}">Remove</button></span>
    </div>`).join('');
  toolFoot.hidden = true;
};

const KIND_LABEL = { address: 'Go to', tab: 'Switch to tab', bookmark: 'Bookmark', history: 'History', search: 'Search' };
// The address as a person reads it: decoded, without the scheme, "www." or
// tracking parameters.
const prettyUrl = url => {
  let text = url;
  try { text = decodeURI(url); } catch (_) {}
  text = text.replace(/^https?:\/\/(www\.)?/, '').replace(/[?&](sei|ved|ei|oq|gs_lcp|sca_esv|sxsrf|source|submit)=[^&]*/g, '').replace(/\/$/, '');
  return text.replace(/\?&/, '?').replace(/\?$/, '');
};
// What the site chip means for this page, in plain words.
const renderSiteInfo = () => {
  const info = shellData().data || {};
  const host = info.host || 'this page';
  const cert = info.certificate;
  const internal = info.scheme === 'agentgo' || info.scheme === 'about' || info.scheme === 'chrome';
  const file = info.scheme === 'file';
  const kind = internal ? 'internal' : file ? 'file' : info.secure && (!cert || cert.secureConnection) ? 'secure' : 'insecure';
  const text = {
    secure: ['Connection is secure', `What you send to and receive from ${host} (passwords, card numbers, messages) is encrypted, so others on the network cannot read or change it. Secure means private on the way; it does not mean the site itself is trustworthy.`],
    insecure: ['Connection is not secure', `${host} does not encrypt this page. Others on the network could see or change what you send or receive. Do not enter passwords, card numbers or other private details here.`],
    internal: ['AgentGo page', 'This page is part of AgentGo itself. It is not loaded from the internet.'],
    file: ['File on this computer', 'This page is a file from your computer. It is not loaded from the internet.'],
  }[kind];
  const date = cert?.validUntil ? new Date(cert.validUntil * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' }) : '';
  const granted = Array.isArray(info.permissions) ? info.permissions : Object.keys(info.permissions || {}).filter(key => info.permissions[key]);
  toolBody.innerHTML = `
    <div class="site ${kind}">
      <div class="site-head"><span class="site-dot"></span><strong>${esc(text[0])}</strong></div>
      <p class="site-note">${esc(text[1])}</p>
      ${cert && kind === 'secure' ? `<dl class="site-cert">
        <dt>Issued to</dt><dd>${esc(cert.issuedTo || host)}</dd>
        <dt>Issued by</dt><dd>${esc(cert.issuedBy || 'Unknown')}</dd>
        ${date ? `<dt>Valid until</dt><dd>${esc(date)}</dd>` : ''}
      </dl>` : ''}
      ${internal || file ? '' : `<div class="site-perm"><span>Permissions</span><span>${granted.length ? esc(granted.join(', ')) : 'None granted'}</span></div>
      <button class="site-settings" type="button" data-site-settings>Site settings</button>`}
    </div>`;
  toolFoot.hidden = true;
};
const renderSuggest = () => {
  const { data } = shellData();
  const items = data?.items || [];
  toolBody.innerHTML = items.map((item, index) => `
    <button class="suggest${index === data.selected ? ' on' : ''}" type="button" data-suggest="${index}" style="--i:${index}">
      <span class="suggest-kind">${esc(KIND_LABEL[item.kind] || item.kind)}</span>
      <span class="suggest-text"><b>${esc(item.title || item.url)}</b>${item.kind === 'search' ? '' : `<small>${esc(prettyUrl(item.url))}</small>`}</span>
    </button>`).join('');
  toolFoot.hidden = true;
};

const trCardHtml = (result, loading) => {
  const names = new Intl.DisplayNames([navigator.language || 'en'], { type: 'language' });
  const name = code => { try { return names.of(code); } catch (_) { return code || ''; } };
  const to = result.to || 'en';
  const from = loading ? '' : name(result.from);
  const same = !loading && !result.error && String(result.text || '').trim().toLowerCase() === String(result.original || '').trim().toLowerCase();
  let main;
  if (loading) main = '<div class="tc-skel"><i></i><i></i><i></i></div>';
  else if (result.error) main = `<p class="tc-err">${esc(result.error)}</p>`;
  else main = `<p class="tr-out">${esc(result.text)}</p>${same ? `<p class="tr-same">This text is already in ${esc(name(to))}. Pick another language above.</p>` : `<details class="tc-orig"><summary>Original</summary><p>${esc(result.original)}</p></details>`}`;
  return `<div class="tc${loading ? ' loading' : ''}">
    <div class="tc-top"><span class="tc-title"><svg viewBox="0 0 24 24"><path d="M4 5h8M8 3v2M6 5c0 4 3 7 6 8M10 5c0 3-2 6-6 8M13 20l4-9 4 9M14.5 17h5"/></svg>Translation</span><button type="button" class="tc-x" data-tr-close aria-label="Close"><svg viewBox="0 0 24 24"><path d="m7 7 10 10M17 7 7 17"/></svg></button></div>
    <div class="tr-langs"><span>${esc(from || (loading ? 'Detecting' : 'Detected'))}</span><i></i><button type="button" class="tr-to" data-tr-pick="${esc(to)}">${esc(name(to))}<svg viewBox="0 0 24 24"><path d="m7 10 5 5 5-5"/></svg></button></div>
    <div class="tc-main">${main}</div>
    <div class="tr-acts"><button class="btn primary" type="button" data-ctx="copy_translation"${loading || result.error ? ' disabled' : ''}>Copy translation</button><small>Sent to Google Translate</small></div>
  </div>`;
};
const renderCtx = () => {
  const data = shellData().data || {};
  document.body.classList.toggle('report', data.mode === 'report');
  document.body.classList.remove('trcard');
  if (data.mode === 'report') {
    const { kind, result = {} } = data.report || {};
    document.body.classList.add('report');
    $('#toolKicker').textContent = 'Page tools';
    $('#toolTitle').textContent = kind === 'fonts' ? 'Font' : kind === 'translate' ? 'Translation' : 'Accessibility';
    if (result.error && kind !== 'translate') {
      toolBody.innerHTML = `<div class="need"><b>Could not check this page</b><p>${esc(result.error)}</p></div>`;
    } else if (kind === 'translate') {
      document.body.classList.add('trcard');
      toolBody.innerHTML = trCardHtml(result, data.report?.loading === true);
      toolBody.dataset.translation = result.text || '';
      toolBody.dataset.original = result.original || ''; } else if (kind === 'fonts') {
      const rows = [['Element', result.element], ['Font stack', (result.families || []).join(', ')],
        ['Drawn with', (result.drawn || []).map(f => `${f.familyName}${f.isCustomFont ? ' (web font)' : ''}`).join(', ') || 'Not reported'],
        ['Size', result.size], ['Weight', result.weight], ['Style', result.style], ['Line height', result.lineHeight],
        ['Letter spacing', result.letterSpacing], ['Colour', result.color], ['Web fonts loaded', (result.webFontsLoaded || []).join(', ') || 'None']];
      toolBody.innerHTML = `<dl class="ctx-dl">${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v ?? '')}</dd>`).join('')}</dl>
        ${result.sample ? `<p class="ctx-sample">${esc(result.sample)}</p>` : ''}`;
    } else {
      const list = result.violations || [];
      toolBody.innerHTML = list.length
        ? `<p class="ctx-sum">${result.total} issue${result.total === 1 ? '' : 's'} in ${list.length} rule${list.length === 1 ? '' : 's'}</p>` + list.map(v => `
          <div class="ctx-v ${esc(v.impact)}"><div><b>${esc(v.help)}</b><span>${esc(v.impact)} · ${v.count}</span></div>
          ${(v.examples || []).map(e => `<code>${esc(e)}</code>`).join('')}</div>`).join('')
        : '<div class="need"><b>No issues found</b><p>The built-in checks found nothing. Automated checks cover only part of accessibility, so test with a keyboard and a screen reader too.</p></div>';
    }
  } else {
    const menu = data.menu || {};
    const ICON = {
      back: 'M15 6l-6 6 6 6', forward: 'M9 6l6 6-6 6', reload: 'M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5',
      print: 'M7 9V4h10v5M7 17H5v-6h14v6h-2M7 14h10v6H7z', inspect: 'M5 4l6 16 2.5-6.5L20 11z',
      a11y: 'M12 3.5a1.4 1.4 0 1 0 .01 0M5 8.5l7 1.5 7-1.5M12 10v5l-3 5.5M12 15l3 5.5',
      fonts: 'M5 19L11 5l6 14M7.4 14h7.2M19 6v5M16.5 8.5h5',
      copy: 'M9 9h10v11H9zM5 15V4h10', copy_text: 'M9 9h10v11H9zM5 15V4h10', cut: 'M6 18a2.5 2.5 0 1 0 .01 0M6 6a2.5 2.5 0 1 0 .01 0M8 7.5L19 18M8 16.5L19 6',
      paste: 'M9 4h6v3H9zM7 6H5v14h14V6h-2', select_all: 'M4 4h16v16H4zM8 12h8M12 8v8',
      translate: 'M4 5h8M8 3v2M6 5c0 4 3 7 6 8M10 5c0 3-2 6-6 8M13 20l4-9 4 9M14.5 17h5', translate_page: 'M4 5h8M8 3v2M6 5c0 4 3 7 6 8M10 5c0 3-2 6-6 8M13 20l4-9 4 9M14.5 17h5',
      open_link: 'M14 4h6v6M20 4l-9 9M18 14v6H4V6h6', open_image: 'M14 4h6v6M20 4l-9 9M18 14v6H4V6h6',
      copy_link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1', copy_image: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
    };
    const HINT = { copy: 'Ctrl+C', copy_text: 'Ctrl+C', cut: 'Ctrl+X', paste: 'Ctrl+V', select_all: 'Ctrl+A', back: 'Alt+←', forward: 'Alt+→', reload: 'Ctrl+R', print: 'Ctrl+P', inspect: 'F12' };
    let row = 0;
    toolBody.innerHTML = (menu.items || []).map(item => item.sep ? '<div class="menu-sep"></div>'
      : `<button class="menu-item" type="button" style="--r:${row++}" data-ctx="${esc(item.action)}"${item.disabled ? ' disabled' : ''}><span class="mi"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="${ICON[item.action] || ''}"/></svg>${esc(item.label)}</span>${HINT[item.action] ? `<kbd>${HINT[item.action]}</kbd>` : ''}</button>`).join('');
  }
  toolFoot.hidden = true;
};

const loadTool = async tool => {
  if (tool === 'ctxmenu') {
    renderCtx();
  } else if (tool === 'palette') {
    if (!$('#palInput')) await renderPalette();
  } else if (tool === 'setup') {
    if (!document.querySelector('.wiz')) await renderSetup();
  } else if (tool === 'translate') {
    await renderTranslate();
  } else if (tool === 'permission') {
    await renderPermission();
  } else if (tool === 'report') {
    if (!document.querySelector('.rep, .rep-load, .rep-done')) await renderReport();
  } else if (tool === 'suggest') {
    renderSuggest();
  } else if (tool === 'siteinfo') {
    renderSiteInfo();
  } else if (tool === 'find') {
    renderFind();
  } else if (tool === 'menu') {
    await renderMenu();
  } else if (tool === 'extensions') {
    const list = await extensionList();
    if (tool !== activeTool) return;
    renderExtensions(list);
  } else if (tool === 'bookmarks') {
    const items = (await callBridge('bookmark.list')).items || [];
    if (tool !== activeTool) return;
    renderBookmarks(items);
  } else if (tool === 'history') {
    const items = (await callBridge('history.list', { limit: 100 })).items || [];
    if (tool !== activeTool) return;
    historyItems = items;
    renderHistory();
  } else if (tool === 'downloads') {
    const [list, settings] = await Promise.all([callBridge('downloads.list'), callBridge('settings.get')]);
    if (tool !== activeTool) return;
    renderDownloads(list.items || [], settings.downloadsEnabled === true);
  } else if (tool === 'settings') {
    const [settings, version, gomini] = await Promise.all([callBridge('settings.get'), callBridge('browser.version'), callBridge('gomini.status').catch(() => null)]);
    if (tool !== activeTool) return;
    renderSettings(settings, version, gomini);
    watchGomini(gomini);
  } else if (tool === 'savelogin') {
    const list = await callBridge('tabs.list');
    const tab = (list.tabs || []).find(item => item.tabId === list.activeTabId);
    if (tool !== activeTool) return;
    const offer = tab?.saveLogin;
    if (!offer) { window.__agentgoRequestClose(); return; }
    toolBody.innerHTML = `
      <div class="need save">
        <b>${offer.update ? 'Update the saved sign-in?' : 'Save this sign-in?'}</b>
        <p><code>${esc(offer.username)}</code> on <code>${esc(offer.origin)}</code></p>
        <p>It is encrypted for your Windows account. You fill it with one click on the key button, and agents can have it typed for them without ever seeing the password.</p>
        <div class="acts">
          <button class="btn primary" type="button" data-offer="accept" data-tab="${esc(tab.tabId)}">${offer.update ? 'Update' : 'Save'}</button>
          <button class="btn" type="button" data-offer="dismiss" data-tab="${esc(tab.tabId)}">Not now</button>
          <button class="btn" type="button" data-offer="never" data-tab="${esc(tab.tabId)}">Never for this site</button>
        </div>
      </div>`;
    toolFoot.hidden = true;
  } else if (tool === 'accounts') {
    [accounts, credentials] = await Promise.all([
      callBridge('accounts.list').then(result => result.accounts || []),
      callBridge('credentials.list').then(result => result.credentials || []),
    ]);
    if (tool !== activeTool) return;
    if (!accounts.some(account => account.account === linking)) linking = null;
    if (!accounts.some(account => account.account === connecting)) connecting = null;
    renderAccounts();
  } else if (tool === 'agent') {
    const [status, connection, task] = await Promise.all([callBridge('agent.status'), callBridge('agent.connection'), latestTask().catch(() => null)]);
    if (tool !== activeTool) return;
    renderAgent(status, connection, task);
  }
};

// Scrolls to the section a shell.* command named, once.
const focusSection = () => {
  const data = shellData();
  if (!data.section || data.focused) return;
  data.focused = true;
  const el = toolBody.querySelector(`[data-section="${CSS.escape(data.section)}"]`);
  if (!el) return;
  el.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  el.classList.add('flash');
};
const refresh = async () => {
  if (!activeTool || closing || activeTool === 'find') return;
  try { await loadTool(activeTool); focusSection(); } catch (error) { console.error(error.message); }
};
// Called by the native side when agent state changes while the panel is open.
window.__agentgoRefresh = refresh;

let settleTimer = 0;
window.__agentgoOpenTool = async tool => {
  if (!TITLES[tool]) return;
  const sequence = ++openSequence;
  activeTool = tool;
  closing = false;
  armed = null;
  document.body.className = tool;
  // The cascade plays once per opening; later redraws just appear.
  clearTimeout(settleTimer);
  settleTimer = setTimeout(() => { if (sequence === openSequence) document.body.classList.add('settled'); }, 900);
  document.body.removeAttribute('data-open');
  document.body.removeAttribute('data-closing');
  $('#toolKicker').textContent = TITLES[tool][0];
  $('#toolTitle').textContent = TITLES[tool][1];
  $('#toolSearch').hidden = tool !== 'history' && tool !== 'settings';
  toolFilter.value = '';
  lastQuery = '';
  searchBox.classList.remove('has');
  setCount('');
  toolBody.classList.remove('searching');
  const forSettings = tool === 'settings';
  toolFilter.setAttribute('aria-label', forSettings ? 'Search settings' : 'Filter history');
  toolFilter.placeholder = forSettings ? 'Search settings' : 'Filter history';
  if (forSettings) runHints(sequence); else clearTimeout(hintTimer);
  toolFoot.hidden = true;
  toolBody.replaceChildren();
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (sequence === openSequence && !closing) document.body.dataset.open = 'true';
  }));
  try {
    callBridge('settings.get').then(applyTheme).catch(() => {});
    await loadTool(tool);
    focusSection();
  } catch (error) {
    if (sequence === openSequence) empty('Unavailable', error.message || 'This panel could not be loaded.');
  }
};

window.__agentgoRequestClose = () => {
  if (!activeTool || closing) return;
  window.__agentgoShellData = null;
  closing = true;
  const closeSequence = ++openSequence;
  document.body.removeAttribute('data-open');
  document.body.dataset.closing = 'true';
  window.setTimeout(async () => {
    // Opened again meanwhile (a second right-click): this close is stale.
    if (closeSequence !== openSequence) return;
    try { await callBridge('ui.close_tool'); } catch (error) { console.error(error.message); }
    activeTool = null;
    closing = false;
    document.body.removeAttribute('data-closing');
  }, 150);
};

// Downloads progress while that panel is open.
setInterval(() => { if (activeTool === 'downloads' && !document.hidden) refresh(); }, 1000);
// The blocked counter ticks up while Settings is open (only the number is redrawn).
setInterval(() => { if (activeTool === 'settings' && !closing && !document.hidden) refreshAdBlockCount(); }, 900);

toolFilter.addEventListener('input', () => { if (activeTool === 'settings') applySearch(true); else renderHistory(); });

toolBody.addEventListener('click', async event => {
  const pill = event.target.closest('.switch');
  if (pill) {
    pill.classList.remove('pop');
    void pill.offsetWidth;
    pill.classList.add('pop');
    setTimeout(() => pill.classList.remove('pop'), 800);
  }
  const target = event.target.closest('button');
  if (!target) return;
  try {
    if (target.dataset.siteSettings !== undefined) {
      // Settings, scrolled to the site permissions.
      window.__agentgoShellData = { section: 'privacy', data: null };
      await callBridge('ui.open_tool', { tool: 'settings' });
    } else if (target.dataset.ctx) {
      const menu = shellData().data?.menu || {};
      const action = target.dataset.ctx;
      const pending = action === 'a11y' || action === 'fonts' || action === 'translate';
      if (action === 'translate_page') {
        await callBridge('ui.run_action', { name: 'tool:translate' });
        return;
      }
      if (pending) toolBody.innerHTML = action === 'translate' ? '<div class="tr"><div class="tr-load"><i></i><i></i><i></i></div><p class="tr-src">Translating…</p></div>' : '<div class="need"><b>Checking…</b></div>';
      await callBridge('ui.context_action', { action, tabId: menu.tabId, x: menu.x, y: menu.y, link: menu.link, source: menu.source, selection: menu.selection, to: await targetLang(), text: target.closest('.tr') ? toolBody.dataset.translation : undefined });
      if (action === 'copy_translation') { target.textContent = 'Copied'; return; }
      if (!pending) window.__agentgoRequestClose();
    } else if (target.dataset.suggest) {
      const { data } = shellData();
      await callBridge('omnibox.select_suggestion', { index: Number(target.dataset.suggest), text: data?.text });
      window.__agentgoRequestClose();
    } else if (target.dataset.find) {
      if (target.dataset.find === 'close') window.__agentgoRequestClose();
      else if (findText) await findStep(target.dataset.find === 'next');
    } else if (target.dataset.menu) {
      await menuAction(target.dataset.menu);
    } else if (target.dataset.bookmarkRemove) {
      await callBridge('bookmark.remove', { url: target.dataset.bookmarkRemove });
      await refresh();
    } else if ('clearData' in target.dataset) {
      if (armed !== 'data') {
        armed = 'data';
        target.classList.add('arm');
        target.textContent = 'Click again to clear';
        return;
      }
      armed = null;
      target.disabled = true;
      target.textContent = 'Clearing…';
      await callBridge('browsing_data.clear', { confirm: true, history: false });
      target.textContent = 'Cleared';
      setTimeout(refresh, 1500);
    } else if (target.dataset.gomini) {
      const action = target.dataset.gomini;
      if (action === 'remove' && armed !== 'gomini') {
        armed = 'gomini';
        target.classList.add('arm');
        target.textContent = 'Click again to remove';
        return;
      }
      armed = null;
      const card = target.closest('[data-gomini-card]');
      if (action === 'download') card?.classList.add('launch');
      const next = await callBridge(`gomini.${action === 'download' ? 'install' : action}`);
      const fresh = toolBody.querySelector('[data-gomini-card]');
      if (fresh) fresh.outerHTML = gominiCard(next);
      watchGomini(next);
    } else if ('openExtensions' in target.dataset) {
      await callBridge('ui.open_tool', { tool: 'extensions' });
    } else if ('openFolder' in target.dataset) {
      await callBridge('ui.open_download_folder');
    } else if (target.dataset.extPin || target.dataset.extOn) {
      const id = target.dataset.extPin || target.dataset.extOn;
      const ext = (await extensionList()).find(item => item.id === id);
      if (!ext) throw new Error('That extension is gone');
      const enabled = target.dataset.extOn ? !ext.enabled : ext.enabled;
      const pinned = target.dataset.extPin ? !ext.pinned : ext.pinned;
      await callBridge(enabled ? 'extension.enable' : 'extension.disable', { id, pinned });
      await extChanged();
    } else if (target.dataset.extOpen || target.dataset.extOptions) {
      await callBridge('extension.open_popup', { id: target.dataset.extOpen || target.dataset.extOptions, options: Boolean(target.dataset.extOptions) });
      window.__agentgoRequestClose();
    } else if (target.dataset.extRemove) {
      if (armed !== target.dataset.extRemove) {
        armed = target.dataset.extRemove;
        target.classList.add('arm');
        target.textContent = 'Sure?';
        return;
      }
      armed = null;
      await callBridge('extension.uninstall', { id: target.dataset.extRemove });
      await extChanged();
    } else if ('extStore' in target.dataset) {
      await callBridge('tabs.create', { url: 'https://chromewebstore.google.com/' });
      window.__agentgoRequestClose();
    } else if (target.dataset.setting === 'adBlock') {
      await toggleAdBlock(target);
    } else if (target.dataset.jump !== undefined) {
      await jumpToSetting(target.dataset.jump);
    } else if (target.dataset.chip !== undefined) {
      toolFilter.value = target.dataset.chip;
      toolFilter.dispatchEvent(new Event('input'));
      toolFilter.focus();
    } else if (target.dataset.setting) {
      const value = target.dataset.kind === 'boolean' ? target.getAttribute('aria-checked') !== 'true' : target.dataset.value;
      // the theme circle starts where the person clicked
      if (target.dataset.setting === 'theme' && lastPointer) await callBridge('ui.theme_origin', { x: lastPointer[0], y: lastPointer[1] });
      const key = target.dataset.setting;
      // Switches and segments change in place so their motion plays; only the
      // theme redraws the whole panel (its colours change).
      if (target.classList.contains('switch')) {
        target.setAttribute('aria-checked', String(value));
        target.classList.toggle('on', value);
        target.classList.remove('pop'); void target.offsetWidth; target.classList.add('pop');
      } else if (target.closest('.seg')) {
        target.closest('.seg').querySelectorAll('button').forEach(b => b.classList.toggle('on', b === target));
      }
      try {
        await callBridge('settings.set', { settings: { [key]: value } });
      } catch (error) {
        if (target.classList.contains('switch')) { target.setAttribute('aria-checked', String(!value)); target.classList.toggle('on', !value); }
        throw error;
      }
      if (key === 'theme') await refresh();
      else if (typeof settingsCache === 'object' && settingsCache) settingsCache[key] = value;
    } else if (target.dataset.openUrl) {
      await callBridge('tabs.navigate', { url: target.dataset.openUrl });
      window.__agentgoRequestClose();
    } else if (target.dataset.download) {
      await callBridge(`download.${target.dataset.download}`, { id: Number(target.dataset.id) });
      await refresh();
    } else if (target.dataset.agent) {
      await callBridge(target.dataset.agent);
      await refresh();
    } else if (target.dataset.approval) {
      await callBridge('agent.approval.respond', { approvalId: target.dataset.approval, approve: target.dataset.approve === 'true' });
      await refresh();
    } else if (target.dataset.accLink || target.dataset.accConnect || 'accCancel' in target.dataset) {
      linking = target.dataset.accLink || null;
      connecting = target.dataset.accConnect || null;
      renderAccounts();
    } else if (target.dataset.offer) {
      const accept = target.dataset.offer === 'accept';
      await callBridge(accept ? 'credentials.offer_accept' : 'credentials.offer_dismiss', { tabId: target.dataset.tab, never: target.dataset.offer === 'never' });
      window.__agentgoRequestClose();
    } else if (target.dataset.accBlock) {
      await callBridge(target.dataset.blocked === 'true' ? 'accounts.unblock' : 'accounts.block', { account: target.dataset.accBlock });
      await refresh();
    } else if (target.dataset.credDelete) {
      await callBridge('credentials.delete', { origin: target.dataset.credDelete });
      await refresh();
    } else if (target.dataset.accConfirm) {
      linking = null;
      await callBridge('accounts.link', { account: target.dataset.accConfirm, acknowledge: true });
      window.__agentgoRequestClose();
    } else if (target.dataset.accUnlink || target.dataset.accForget) {
      // Taking access away is one click; giving it is the guarded direction.
      await callBridge('accounts.unlink', { account: target.dataset.accUnlink || target.dataset.accForget, forget: Boolean(target.dataset.accForget) });
      await refresh();
    } else if (target.dataset.planRemove) {
      await callBridge('shell.agent_edit_plan', { taskId: target.dataset.task, remove: [Number(target.dataset.planRemove)] });
      await refresh();
    } else if (target.dataset.taskRetry) {
      await callBridge('shell.agent_retry', { taskId: target.dataset.taskRetry });
      setTimeout(refresh, 400);
    } else if (target.dataset.taskReplan) {
      await callBridge('shell.agent_replan', { taskId: target.dataset.taskReplan });
      await refresh();
    } else if (target.dataset.wfApprove) {
      await callBridge('workflow.approve', { name: target.dataset.wfApprove });
      const data = shellData().data;
      if (data?.workflows) data.workflows = data.workflows.map(w => w.name === target.dataset.wfApprove ? { ...w, approved: true } : w);
      await refresh();
    } else if (target.dataset.skillToggle) {
      const on = target.dataset.enabled !== 'true';
      await callBridge(on ? 'skill.enable' : 'skill.disable', { name: target.dataset.skillToggle });
      const data = shellData().data;
      if (data?.skills) data.skills = data.skills.map(k => k.name === target.dataset.skillToggle ? { ...k, enabled: on } : k);
      await refresh();
    } else if (target.dataset.handoff) {
      await callBridge(target.dataset.handoff, { tabId: target.dataset.tab });
      await refresh();
    }
  } catch (error) {
    // Show the real reason (for example: the challenge is still on the page).
    target.textContent = error.message.slice(0, 60);
    target.disabled = true;
    setTimeout(refresh, 1800);
  }
});

toolBody.addEventListener('change', async event => {
  const select = event.target.closest('select[data-select]');
  if (!select) return;
  await callBridge('settings.set', { settings: { [select.dataset.select]: select.value } }).catch(error => console.error(error.message));
});

toolBody.addEventListener('input', event => {
  if (event.target.id !== 'findText') return;
  findText = event.target.value;
  if (!findText) window.__agentgoFindResult(0, 0);
  callBridge('ui.find', { text: findText }).catch(() => {});
});

toolBody.addEventListener('submit', async event => {
  const form = event.target;
  if (form.id === 'findForm') {
    event.preventDefault();
    if (findText) findStep(!lastShift);
    return;
  }
  const method = { accAdd: 'accounts.add', credAdd: 'credentials.save', accConnect: 'accounts.connect' }[form.id];
  if (!method) return;
  event.preventDefault();
  const button = form.querySelector('button');
  const label = button.textContent;
  try {
    const fields = Object.fromEntries(new FormData(form));
    if (form.id === 'accConnect') {
      // Submitting this form is the acknowledgement of the warning above it.
      await callBridge(method, { ...fields, account: form.dataset.account, acknowledge: true });
      connecting = null;
      window.__agentgoRequestClose();
    } else {
      await callBridge(method, fields);
      await refresh();
    }
  } catch (error) {
    button.textContent = error.message.replace(/^.*?: /, '').slice(0, 70);
    setTimeout(() => { button.textContent = label; }, 2600);
  }
});

toolFoot.addEventListener('change', event => {
  const picked = event.target.closest('[data-history-range]');
  if (!picked) return;
  historyRange = picked.value;
  // A new range needs a fresh confirmation.
  armed = null;
  const button = toolFoot.querySelector('[data-clear]');
  if (button) { button.classList.remove('arm'); button.textContent = 'Clear history'; }
});
toolFoot.addEventListener('click', async event => {
  if (event.target.closest('[data-ext-store]')) {
    try { await callBridge('tabs.create', { url: 'https://chromewebstore.google.com/' }); window.__agentgoRequestClose(); } catch (error) { console.error(error.message); }
    return;
  }
  const button = event.target.closest('[data-clear]');
  if (!button) return;
  const target = button.dataset.clear;
  const range = target === 'history' ? (toolFoot.querySelector('[data-history-range]')?.value || historyRange) : null;
  if (armed !== target) {
    armed = target;
    button.classList.add('arm');
    button.textContent = range && range !== 'all'
      ? `Click again to clear the ${HISTORY_RANGES.find(([v]) => v === range)[1].toLowerCase()}`
      : 'Click again to confirm';
    return;
  }
  armed = null;
  try {
    if (range && range !== 'all') await callBridge('history.delete', { sinceMinutes: Number(range) });
    else await callBridge(`${target}.clear`, { confirm: true });
  } catch (error) { console.error(error.message); }
  await refresh();
});

$('#toolClose').addEventListener('click', window.__agentgoRequestClose);
let lastShift = false;
document.addEventListener('keydown', event => {
  lastShift = event.shiftKey;
  if (event.key === 'Escape') window.__agentgoRequestClose();
  else if (activeTool === 'find' && event.key === 'F3') { event.preventDefault(); if (findText) findStep(!event.shiftKey); }
  else if (activeTool === 'find' && event.ctrlKey && event.key.toLowerCase() === 'f') { event.preventDefault(); $('#findText')?.select(); }
});

