// Trusted browser chrome. Native state is authoritative: tabs, titles,
// favicons, loading and agent state are pushed from the CEF UI thread and
// only rendered here. Every control calls a real command through the bridge.
const $ = selector => document.querySelector(selector);
const address = $('#address');
const tabStrip = $('#tabs');

const LANDING_URL = 'agentgo://shell/index.html?embedded=1';
let tabs = [];
let activeTabId = null;
let closedTabs = [];
let bookmarks = new Set();
let agent = { control: 'running', sessions: [], approvals: [], handoffs: [] };

const callBridge = async (method, params = {}) => {
  const encoded = encodeURIComponent(JSON.stringify(params));
  const response = await fetch(`agentgo://shell/__bridge?method=${method}&params=${encoded}`);
  const body = await response.json();
  if (!response.ok || body.ok === false) {
    throw new Error(body.error?.message || `Bridge request failed: ${method}`);
  }
  return body.result;
};
const run = (method, params) => callBridge(method, params).catch(error => console.error(method, error.message));

// Same rule as the native resolver: an address opens directly, anything
// else is a search with the provider chosen in Settings (Google by default).
const SEARCH = { Google: "https://www.google.com/search?q=", Bing: "https://www.bing.com/search?q=", DuckDuckGo: "https://duckduckgo.com/?q=" };
let searchBase = SEARCH.Google;
const normalize = value => {
  const text = value.trim();
  if (!text) return LANDING_URL;
  if (/^(https?|file|about):/i.test(text)) return text;
  if (/^[^\s]+\.[^\s]+$/.test(text) || /^localhost(:\d+)?(\/|$)/.test(text)) return `https://${text}`;
  return searchBase + encodeURIComponent(text);
};

// A page that failed to load is shown by a bundled page; the tab is still
// "at" the address that was asked for.
const ERROR_PAGE = 'agentgo://shell/error.html?';
const failedUrl = url => url?.startsWith(ERROR_PAGE) ? new URLSearchParams(url.slice(ERROR_PAGE.length)).get('url') || '' : null;
const isInternal = url => !url || (url.startsWith('agentgo://') && failedUrl(url) === null) || url === 'about:blank';
const hostOf = url => { try { return new URL(url).host; } catch { return url; } };
const titleOf = tab => isInternal(tab.url) ? 'New Tab' : (tab.title || hostOf(tab.url));
const activeTab = () => tabs.find(tab => tab.tabId === activeTabId);

// Tabs moved into another AgentGo window are not in this strip.
const stripTabs = () => tabs.filter(tab => !tab.strip?.window);
const renderTabs = () => {
  const shown = stripTabs();
  const nodes = [];
  let lastGroup = null;
  for (const tab of shown) {
    const group = tab.strip?.group;
    if (group && group.name !== lastGroup) {
      nodes.push(Object.assign(document.createElement('span'), { className: 'tab-group', textContent: group.name, style: `--group:${group.color}` }));
    }
    lastGroup = group?.name || null;
    nodes.push(tabNode(tab, shown.length));
  }
  tabStrip.replaceChildren(...nodes);
  // Only tabs that just appeared play their entrance.
  for (const node of tabStrip.querySelectorAll('.tab')) {
    if (seenTabs && !seenTabs.has(node.dataset.tabId)) node.classList.add('fresh');
  }
  seenTabs = new Set(shown.map(tab => tab.tabId));
  placeFilament();
};
// The filament glides to the active tab and stretches to its width.
let seenTabs = null;
const placeFilament = () => requestAnimationFrame(() => {
  const filament = $('#filament');
  const active = tabStrip.querySelector('.tab.active');
  if (!filament || !active) { if (filament) filament.style.opacity = '0'; return; }
  const strip = filament.parentElement.getBoundingClientRect();
  const box = active.getBoundingClientRect();
  const inset = Math.min(18, box.width / 4);
  filament.style.transform = `translateX(${box.left - strip.left + inset}px) scaleX(${Math.max(0, box.width - inset * 2) / 100})`;
  filament.style.opacity = '1';
});
addEventListener('resize', placeFilament);
const tabNode = (tab, count) => {
  {
    const el = document.createElement('div');
    const strip = tab.strip || {};
    el.className = `tab${tab.tabId === activeTabId ? ' active' : ''}${tab.loading ? ' loading' : ''}${strip.pinned ? ' pinned' : ''}${strip.discarded ? ' discarded' : ''}${strip.frozen ? ' frozen' : ''}`;
    if (strip.group) { el.dataset.group = strip.group.name; el.style.setProperty('--group', strip.group.color); }
    if (strip.discarded) tab = { ...tab, url: strip.discarded.url, title: strip.discarded.title };
    el.role = 'tab';
    el.tabIndex = 0;
    el.dataset.tabId = tab.tabId;
    el.ariaSelected = String(tab.tabId === activeTabId);
    el.title = isInternal(tab.url) ? 'New Tab' : `${titleOf(tab)}\n${tab.url}`;

    const icon = document.createElement('span');
    icon.className = 'tab-ic';
    if (isInternal(tab.url)) {
      icon.innerHTML = '<svg class="mark"><use href="#i-mark"/></svg>';
    } else if (tab.favicon && /^https?:/.test(tab.favicon)) {
      const img = document.createElement('img');
      img.alt = '';
      img.src = tab.favicon;
      img.addEventListener('error', () => icon.replaceChildren(Object.assign(document.createElement('span'), { className: 'glyph' })));
      icon.append(img);
    } else {
      icon.innerHTML = '<span class="glyph"></span>';
    }

    const title = document.createElement('span');
    title.className = 'tab-title';
    title.textContent = titleOf(tab);
    el.append(icon, title);

    if (tab.handoff) {
      el.append(Object.assign(document.createElement('span'), { className: 'tab-agent need', textContent: 'Needs you' }));
    } else if (tab.agent) {
      el.append(Object.assign(document.createElement('span'), { className: 'tab-agent', textContent: 'Agent' }));
    }

    const close = document.createElement('button');
    close.className = 'tab-x';
    close.type = 'button';
    close.ariaLabel = 'Close tab';
    close.disabled = count <= 1;
    close.innerHTML = '<svg><use href="#i-x"/></svg>';
    close.addEventListener('click', event => { event.stopPropagation(); run('tabs.close', { tabId: tab.tabId }); });
    el.append(close);

    el.addEventListener('click', () => { if (tab.tabId !== activeTabId) run('tabs.activate', { tabId: tab.tabId }); });
    el.addEventListener('auxclick', event => { if (event.button === 1 && count > 1) run('tabs.close', { tabId: tab.tabId }); });
    el.addEventListener('keydown', event => {
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); run('tabs.activate', { tabId: tab.tabId }); }
    });
    return el;
  }
};

let wasLoading = false;
const renderToolbar = () => {
  const tab = activeTab();
  if (!tab) return;
  const internal = isInternal(tab.url);
  if (document.activeElement !== address) address.value = internal ? '' : failedUrl(tab.url) ?? tab.url;
  $('#back').disabled = !tab.canGoBack;
  $('#forward').disabled = !tab.canGoForward;

  const reload = $('#i-reload');
  reload.title = tab.loading ? 'Stop loading' : 'Reload (Ctrl+R)';
  $('#reloadIcon').setAttribute('href', tab.loading ? '#i-x' : '#i-reload');

  const security = $('#security');
  const scheme = internal ? 'internal' : tab.url.startsWith('https://') ? 'secure' : 'insecure';
  security.className = `sec ${scheme}`;
  $('#securityIcon').setAttribute('href', scheme === 'internal' ? '#i-mark' : scheme === 'secure' ? '#i-lock' : '#i-info');
  $('#securityText').textContent = scheme === 'internal' ? 'AgentGo' : scheme === 'secure' ? 'Secure' : tab.url.startsWith('file:') ? 'File' : 'Not secure';
  security.title = scheme === 'internal' ? 'Built-in AgentGo page' : scheme === 'secure' ? `Encrypted connection to ${hostOf(tab.url)}` : 'This connection is not encrypted';
  // An agent read text on this page that tries to steer it.
  if (tab.injection) {
    security.className = 'sec insecure';
    $('#securityIcon').setAttribute('href', '#i-info');
    $('#securityText').textContent = 'Agent warning';
    security.title = 'This page has text written to steer an AI agent. AgentGo flagged it and removed it from what the agent read.';
  }

  // On a Chrome Web Store extension page: install it into AgentGo.
  const storePage = /^https:\/\/(chromewebstore\.google\.com\/detail|chrome\.google\.com\/webstore\/detail)\//.test(tab.url || '');
  const addExtension = $('#addExtension');
  if (addExtension.dataset.url !== tab.url) { addExtension.dataset.url = tab.url; addExtension.disabled = false; addExtension.firstElementChild.textContent = 'Add to AgentGo'; }
  addExtension.hidden = !storePage;

  // One button for saved sign-ins: offer to save one, or fill the saved one.
  const key = $('#loginKey');
  key.hidden = !(tab.saveLogin || tab.hasLogin);
  key.classList.toggle('on', Boolean(tab.saveLogin));
  key.title = tab.saveLogin ? `Save the sign-in for ${hostOf(tab.saveLogin.origin)}` : 'Fill your saved sign-in';

  const star = $('#star');
  star.disabled = !/^https?:/.test(tab.url);
  star.classList.toggle('on', bookmarks.has(tab.url));
  star.title = bookmarks.has(tab.url) ? 'Remove bookmark' : 'Bookmark this page';

  const chip = $('#zoomChip');
  const zoom = Math.round(tab.zoom || 100);
  chip.hidden = zoom === 100;
  chip.textContent = `${zoom}%`;

  const progress = $('#progress');
  if (tab.loading && !wasLoading) progress.className = 'progress on';
  if (!tab.loading && wasLoading) progress.className = 'progress done';
  wasLoading = Boolean(tab.loading);
};

// ---- favorites bar: the starred sites, one row under the address bar ----
let favorites = [];
let favSignature = '';
let flyingUrl = null;
let favsOn = true;
let showExtensions = true;
let motionOff = false;
const stillMotion = () => motionOff || matchMedia('(prefers-reduced-motion: reduce)').matches;
const initialOf = text => (text || '?').trim().charAt(0).toUpperCase() || '?';
const faviconOf = url => { try { return `${new URL(url).origin}/favicon.ico`; } catch { return ''; } };
const iconTile = (url, title) => {
  const tile = document.createElement('span');
  tile.className = 'fav-ic';
  tile.textContent = initialOf(title || hostOf(url));
  const source = faviconOf(url);
  if (source) {
    const img = new Image();
    img.alt = '';
    img.addEventListener('load', () => { if (img.naturalWidth > 1) tile.replaceChildren(img); });
    img.src = source;
  }
  return tile;
};
const favNode = item => {
  const node = document.createElement('div');
  node.className = `fav${flyingUrl === item.url ? ' landing' : ''}`;
  node.dataset.url = item.url;
  node.draggable = true;
  node.title = `${item.title || item.url}\n${item.url}`;
  node.role = 'link';
  node.tabIndex = 0;
  const label = document.createElement('span');
  label.className = 'fav-t';
  label.textContent = item.title || hostOf(item.url);
  const close = document.createElement('button');
  close.className = 'fav-x';
  close.type = 'button';
  close.ariaLabel = 'Remove from favorites';
  close.innerHTML = '<svg><use href="#i-x"/></svg>';
  close.addEventListener('click', event => { event.stopPropagation(); removeFavorite(item.url); });
  node.append(iconTile(item.url, item.title), label, close);
  node.addEventListener('click', () => run('tabs.navigate', { tabId: activeTabId, url: item.url }));
  node.addEventListener('auxclick', event => { if (event.button === 1) { event.preventDefault(); run('tabs.create', { url: item.url }); } });
  node.addEventListener('keydown', event => { if (event.key === 'Enter') run('tabs.navigate', { tabId: activeTabId, url: item.url }); });
  node.addEventListener('dragstart', event => { event.dataTransfer.setData('text/plain', item.url); event.dataTransfer.effectAllowed = 'move'; node.classList.add('dragging'); });
  node.addEventListener('dragend', () => { node.classList.remove('dragging'); for (const other of $('#favs').children) other.classList.remove('drop'); });
  node.addEventListener('dragover', event => { event.preventDefault(); node.classList.add('drop'); });
  node.addEventListener('dragleave', () => node.classList.remove('drop'));
  node.addEventListener('drop', async event => {
    event.preventDefault();
    node.classList.remove('drop');
    const url = event.dataTransfer.getData('text/plain');
    const index = favorites.findIndex(other => other.url === item.url);
    if (!url || url === item.url || index < 0) return;
    await run('bookmark.move', { url, index });
    refreshBookmarks();
  });
  return node;
};
const renderFavorites = () => {
  const box = $('#favs');
  if (!favsOn) return;
  const signature = JSON.stringify(favorites.map(item => [item.url, item.title])) + flyingUrl;
  if (signature !== favSignature) {
    favSignature = signature;
    if (!favorites.length) {
      box.innerHTML = '<span class="favs-hint"><svg><use href="#i-star"/></svg>Star a site to keep it here</span>';
    } else {
      const more = document.createElement('button');
      more.className = 'fav-more';
      more.type = 'button';
      more.hidden = true;
      more.addEventListener('click', () => run('ui.open_tool', { tool: 'bookmarks' }));
      box.replaceChildren(...favorites.map(favNode), more);
    }
  }
  fitFavorites();
};
// What does not fit moves behind the "»" button, which opens the full list.
const fitFavorites = () => {
  const box = $('#favs');
  const chips = [...box.querySelectorAll('.fav')];
  const more = box.querySelector('.fav-more');
  if (!more) return;
  for (const chip of chips) chip.hidden = false;
  more.hidden = true;
  const room = box.clientWidth - 24;
  const used = () => chips.filter(chip => !chip.hidden).reduce((sum, chip) => sum + chip.offsetWidth + 2, 0);
  if (used() <= room) return;
  more.hidden = false;
  let hidden = 0;
  for (let i = chips.length - 1; i > 0 && used() + more.offsetWidth > room; i -= 1) { chips[i].hidden = true; hidden += 1; }
  more.textContent = `» ${hidden}`;
};
addEventListener('resize', fitFavorites);
const removeFavorite = async url => {
  const chip = [...$('#favs').querySelectorAll('.fav')].find(node => node.dataset.url === url);
  if (chip && !stillMotion()) { chip.classList.add('removing'); await new Promise(done => setTimeout(done, 300)); } /* ui-timer */
  await run('bookmark.remove', { url });
  refreshBookmarks();
};
// A copy of the site's tile flies from the star to its place in the bar.
const flyToFavorite = url => {
  const chip = [...$('#favs').querySelectorAll('.fav')].find(node => node.dataset.url === url);
  const land = () => { flyingUrl = null; if (chip) { chip.classList.remove('landing'); chip.classList.add('fresh'); } };
  if (!chip || chip.hidden || !favsOn || stillMotion()) { land(); return; }
  const star = $('#star').getBoundingClientRect();
  const to = chip.getBoundingClientRect();
  const fly = document.createElement('div');
  fly.className = 'fly';
  fly.textContent = initialOf(chip.querySelector('.fav-t')?.textContent);
  document.body.append(fly);
  const [sx, sy] = [star.left + star.width / 2, star.top + star.height / 2];
  const [tx, ty] = [to.left + 15, to.top + to.height / 2];
  const flight = fly.animate([
    { transform: `translate(${sx}px, ${sy}px) scale(.5)`, opacity: 1 },
    { transform: `translate(${(sx + tx) / 2}px, ${Math.min(sy, ty) - 14}px) scale(1.2)`, offset: .45 },
    { transform: `translate(${tx}px, ${ty}px) scale(.85)`, opacity: .95 },
  ], { duration: 640, easing: 'cubic-bezier(.3,.7,.2,1)' });
  flight.onfinish = () => { fly.remove(); land(); };
};
const starCelebrate = () => {
  if (stillMotion()) return;
  const star = $('#star');
  const ring = Object.assign(document.createElement('i'), { className: 'ring' });
  star.append(ring);
  setTimeout(() => ring.remove(), 650); /* ui-timer */
  for (let i = 0; i < 8; i += 1) {
    const spark = Object.assign(document.createElement('i'), { className: 'spark' });
    spark.style.setProperty('--a', `${i * 45}deg`);
    star.append(spark);
    setTimeout(() => spark.remove(), 700); /* ui-timer */
  }
};
const starDrop = () => {
  if (stillMotion()) return;
  const star = $('#star');
  star.classList.remove('off-anim');
  void star.offsetWidth;
  star.classList.add('off-anim');
};
let favTimer = 0;
const refreshBookmarksSoon = () => { clearTimeout(favTimer); favTimer = setTimeout(refreshBookmarks, 500); /* ui-timer */ };

// ---- extensions: the puzzle button and the pinned ones ----
let extensions = [];
let extSignature = '';
const renderExtensions = () => {
  $('#openExtensions').hidden = !showExtensions;
  const bar = $('#extBar');
  const pinned = showExtensions ? extensions.filter(ext => ext.pinned) : [];
  const signature = JSON.stringify(pinned.map(ext => [ext.id, ext.enabled, ext.name, (ext.icon || '').length]));
  if (signature === extSignature) return;
  extSignature = signature;
  bar.replaceChildren(...pinned.map(ext => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `ib ext${ext.enabled ? '' : ' off'}`;
    button.title = `${ext.name}${ext.enabled ? '' : ' (off)'}`;
    button.ariaLabel = ext.name;
    if (ext.icon) {
      const img = new Image();
      img.alt = '';
      img.src = ext.icon;
      button.append(img);
    } else {
      button.append(Object.assign(document.createElement('span'), { className: 'ext-letter', textContent: initialOf(ext.name) }));
    }
    button.addEventListener('click', () => run('ui.open_tool', { tool: 'extensions' }));
    return button;
  }));
};
const refreshExtensions = async () => {
  try { extensions = (await callBridge('extension.list')).extensions || []; renderExtensions(); } catch (error) { console.error(error.message); }
};
setInterval(() => { if (openTool === 'extensions' && !document.hidden) refreshExtensions(); }, 1200);

const refreshBookmarks = async () => {
  try {
    const result = await callBridge('bookmark.list');
    bookmarks = new Set(result.items.map(item => item.url));
    favorites = result.items;
    renderToolbar();
    renderFavorites();
  } catch (error) { console.error(error.message); }
};

// Pushed by the native UI thread after creation, activation, navigation,
// title/favicon changes and load completion.
window.__agentgoNativeTabs = snapshot => {
  if (!snapshot || !Array.isArray(snapshot.tabs)) return;
  for (const gone of tabs) {
    if (!isInternal(gone.url) && !snapshot.tabs.some(tab => tab.tabId === gone.tabId)) {
      closedTabs.push(gone.url);
      if (closedTabs.length > 10) closedTabs.shift();
    }
  }
  tabs = snapshot.tabs;
  activeTabId = snapshot.activeTabId || stripTabs()[0]?.tabId || null;
  renderTabs();
  renderToolbar();
  refreshBookmarksSoon();
};

// Pushed whenever sessions, approvals, handoffs or the control state change.
// Client names as agents report them, said the way people know them.
const agentName = raw => {
  const name = String(raw || '').trim();
  const known = [[/claude/i, 'Claude Code'], [/codex/i, 'Codex'], [/droid|factory/i, 'Droid'], [/cursor/i, 'Cursor'], [/gemini/i, 'Gemini'], [/^(stdio-agent|agent|mcp|agentgo-mcp)$/i, 'Agent']];
  for (const [test, label] of known) if (test.test(name)) return label;
  return name ? name.replace(/[-_]+/g, ' ').replace(/\b\w/g, c => c.toUpperCase()).slice(0, 24) : 'Agent';
};

window.__agentgoAgent = status => {
  if (!status) return;
  agent = status;
  // Agents that connected (agent.connect, or a long-lived connection such as
  // the MCP server), not each one-shot command: the pill stays steady while
  // an agent works and goes back to "No agent" when it disconnects.
  const agents = status.agents || [];
  const sessions = agents.length;
  const needs = (status.approvals?.length || 0) + (status.handoffs?.length || 0);
  const pill = $('#agentPill');
  pill.className = `pill${needs ? ' need' : status.control === 'stopped' ? ' stopped' : status.control === 'paused' ? ' paused' : sessions ? ' live' : ''}`;
  $('#agentText').textContent = status.handoffs?.length ? 'Agent needs you'
    : status.approvals?.length ? `${status.approvals.length} approval${status.approvals.length === 1 ? '' : 's'} waiting`
    : status.control === 'stopped' ? 'Agent stopped'
    : status.control === 'paused' ? 'Agent paused'
    : sessions === 1 ? `${agentName(agents[0].agent)} connected`
    : sessions ? `${sessions} agents connected`
    : 'No agent';

  const showControls = sessions > 0 || status.control !== 'running';
  const pause = $('#agentPause');
  const stop = $('#agentStop');
  pause.hidden = !showControls;
  stop.hidden = !showControls || status.control === 'stopped';
  const running = status.control === 'running';
  pause.classList.toggle('go', !running);
  $('#pauseIcon').setAttribute('href', running ? '#i-pause' : '#i-play');
  $('#pauseText').textContent = running ? 'Pause' : status.control === 'stopped' ? 'Allow agent' : 'Resume';
  pause.title = running ? 'Pause agent actions (sessions are kept)' : 'Let agents act again';
};

// The site chip: what "Secure" / "Not secure" means for this page.
let openTool = null;
const toggleSiteInfo = () => {
  if (openTool === 'siteinfo') { run('ui.open_tool', { tool: 'siteinfo' }); return; }
  run('ui.siteinfo', { x: Math.round($('#security').getBoundingClientRect().left) });
};
$('#security').addEventListener('click', toggleSiteInfo);
$('#security').addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); toggleSiteInfo(); } });
window.__agentgoToolState = tool => {
  openTool = tool;
  $('#security').classList.toggle('on', tool === 'siteinfo');
  $('#openHistory').classList.toggle('on', tool === 'history');
  $('#openDownloads').classList.toggle('on', tool === 'downloads');
  $('#openSettings').classList.toggle('on', tool === 'settings');
  $('#openAccounts').classList.toggle('on', tool === 'accounts');
  $('#openMenu').classList.toggle('on', tool === 'menu');
  $('#openExtensions').classList.toggle('on', tool === 'extensions');
  if (!tool) { refreshBookmarks(); refreshExtensions(); }
};

window.__agentgoTheme = settings => {
  customShortcuts = settings.shortcuts && typeof settings.shortcuts === 'object' ? settings.shortcuts : {};
  searchBase = SEARCH[settings.searchEngine] || SEARCH.Google;
  favsOn = settings.favoritesBar !== false;
  document.body.classList.toggle('favs-on', favsOn);
  showExtensions = settings.showExtensions !== false;
  motionOff = settings.motion === false;
  $('#home').hidden = settings.showHome === false;
  renderExtensions();
  if (favsOn) renderFavorites();
  address.placeholder = `Search ${SEARCH[settings.searchEngine] ? settings.searchEngine : "Google"} or enter an address`;
  // the animated switch arrives separately (__agentgoThemeSync); this only
  // makes sure the theme is right
  window.agentgoSetTheme(settings.theme);
};

$('#navigate').addEventListener('submit', event => {
  event.preventDefault();
  hideSuggestions();
  run('tabs.navigate', { tabId: activeTabId, url: normalize(address.value) });
  address.blur();
});
address.addEventListener('focus', () => address.select());

// Suggestions under the address bar (drawn by the tools surface).
let suggestions = [];
let suggestIndex = -1;
let suggestTimer = 0;
const showSuggestions = () => {
  const box = $('#navigate').getBoundingClientRect();
  run('ui.suggest', { x: Math.round(box.left), width: Math.round(box.width), items: suggestions, selected: suggestIndex, text: address.value });
};
const hideSuggestions = () => {
  if (!suggestions.length) return;
  suggestions = [];
  suggestIndex = -1;
  run('ui.suggest', { items: [] });
};
const refreshSuggestions = () => {
  clearTimeout(suggestTimer);
  suggestTimer = setTimeout(async () => {
    const text = address.value;
    if (!text.trim()) { hideSuggestions(); return; }
    try {
      const result = await callBridge('omnibox.suggestions', { text });
      if (address.value !== text || document.activeElement !== address) return;
      suggestions = result.suggestions || [];
      suggestIndex = -1;
      showSuggestions();
    } catch (error) { console.error(error.message); }
  }, 90);
};
address.addEventListener('input', refreshSuggestions);
// A click in the popover blurs the address bar first; give it time to land.
address.addEventListener('blur', () => setTimeout(hideSuggestions, 220));
window.__agentgoOmnibox = ({ text = '', show = false, blur = false } = {}) => {
  address.value = text;
  if (blur) { address.blur(); hideSuggestions(); renderToolbar(); return; }
  if (show) refreshSuggestions(); else hideSuggestions();
};
// The agent's status line and the person's prompt to the agent.
// The agent's status line, and the input lock while an agent acts (the lock
// message wins while it is on; the agent's own note comes back after).
const overlayState = { note: null, lock: null };
window.__agentgoOverlay = ({ on, text, lock }) => {
  overlayState[lock ? 'lock' : 'note'] = on ? (text || '') : null;
  const shown = overlayState.lock ?? overlayState.note;
  $('#agentOverlay').hidden = shown === null;
  $('#agentOverlay').classList.toggle('lock', overlayState.lock !== null);
  $('#agentOverlayText').textContent = shown || '';
};
window.__agentgoPrompt = ({ on, placeholder }) => {
  const form = $('#personPrompt');
  form.hidden = !on;
  $('#personPromptText').placeholder = placeholder || 'Tell the agent something';
};
$('#personPrompt').addEventListener('submit', event => {
  event.preventDefault();
  const input = $('#personPromptText');
  const text = input.value.trim();
  if (!text) return;
  run('ui.prompt', { text });
  input.value = '';
  input.placeholder = 'Sent. The agent will read it.';
});
address.addEventListener('keydown', event => {
  if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && suggestions.length) {
    event.preventDefault();
    // -1 is the typed text itself; the arrows cycle through it.
    const next = suggestIndex + (event.key === 'ArrowDown' ? 1 : -1);
    suggestIndex = next >= suggestions.length ? -1 : next < -1 ? suggestions.length - 1 : next;
    showSuggestions();
    return;
  }
  if (event.key === 'Enter' && suggestIndex >= 0 && suggestions[suggestIndex]) {
    event.preventDefault();
    const item = suggestions[suggestIndex];
    hideSuggestions();
    if (item.kind === 'tab') run('tabs.activate', { tabId: item.tabId });
    else run('tabs.navigate', { tabId: activeTabId, url: item.url });
    address.blur();
    return;
  }
  if (event.key === 'Escape') { hideSuggestions(); address.blur(); renderToolbar(); }
});

$('#back').addEventListener('click', () => run('tab.back', { tabId: activeTabId }));
$('#forward').addEventListener('click', () => run('tab.forward', { tabId: activeTabId }));
$('#i-reload').addEventListener('click', () => run(activeTab()?.loading ? 'tab.stop' : 'tab.reload', { tabId: activeTabId }));
$('#home').addEventListener('click', () => run('tabs.navigate', { tabId: activeTabId, url: LANDING_URL }));
$('#newTab').addEventListener('click', () => run('tabs.create', { url: LANDING_URL }));
$('#star').addEventListener('click', async () => {
  const tab = activeTab();
  if (!tab) return;
  const url = tab.url;
  if (bookmarks.has(url)) {
    starDrop();
    await removeFavorite(url);
    return;
  }
  flyingUrl = url;
  starCelebrate();
  await run('bookmark.add', { url, title: titleOf(tab) });
  await refreshBookmarks();
  flyToFavorite(url);
});

$('#windowMinimize').addEventListener('click', () => run('window.minimize'));
$('#windowMaximize').addEventListener('click', () => run('window.maximize'));
$('#windowClose').addEventListener('click', () => run('window.close'));
$('.strip').addEventListener('dblclick', event => { if (event.target.closest('.strip') && !event.target.closest('.tab,button')) run('window.maximize'); });

$('#addExtension').addEventListener('click', async () => {
  const button = $('#addExtension');
  button.disabled = true;
  button.firstElementChild.textContent = 'Adding…';
  try {
    const added = await callBridge('extension.install', { store: activeTab().url });
    button.firstElementChild.textContent = added?.name ? `Added: ${added.name}` : 'Added';
    await refreshExtensions();
    const puzzle = $('#openExtensions');
    puzzle.classList.remove('pulse');
    void puzzle.offsetWidth;
    puzzle.classList.add('pulse');
  } catch (error) {
    button.firstElementChild.textContent = 'Could not add it';
    button.title = String(error?.message || error);
    button.disabled = false;
  }
});
$('#loginKey').addEventListener('click', () => {
  const tab = activeTab();
  if (tab?.saveLogin) run('ui.open_tool', { tool: 'savelogin' });
  else if (tab) run('credentials.fill', { tabId: tab.tabId });
});
$('#zoomChip').addEventListener('click', () => run('page.zoom', { tabId: activeTabId, percent: 100 }));
for (const [id, tool] of [['#openExtensions', 'extensions'], ['#openHistory', 'history'], ['#openDownloads', 'downloads'], ['#openSettings', 'settings'], ['#openAccounts', 'accounts'], ['#openMenu', 'menu'], ['#agentPill', 'agent']]) {
  $(id).addEventListener('click', () => run('ui.open_tool', { tool }));
}
$('#agentPause').addEventListener('click', () => run(agent.control === 'running' ? 'agent.pause' : 'agent.resume'));
$('#agentStop').addEventListener('click', () => run('agent.stop'));

// The one shortcut table. Keys pressed while the chrome has the focus are
// mapped here; keys pressed while a page has it arrive from the native
// keyboard handler (app.rs `browser_shortcut`) under the same names.
const switchTab = index => {
  const tab = tabs[index];
  if (tab && tab.tabId !== activeTabId) run('tabs.activate', { tabId: tab.tabId });
};
const cycleTab = step => {
  if (tabs.length > 1) switchTab((tabs.findIndex(tab => tab.tabId === activeTabId) + step + tabs.length) % tabs.length);
};
const shortcuts = {
  focus_address: () => address.focus(),
  new_tab: () => run('tabs.create', { url: LANDING_URL }),
  close_tab: () => tabs.length > 1 && run('tabs.close', { tabId: activeTabId }),
  reopen_tab: () => { const url = closedTabs.pop(); if (url) run('tabs.create', { url }); },
  next_tab: () => cycleTab(1),
  prev_tab: () => cycleTab(-1),
  reload: () => run('tab.reload', { tabId: activeTabId }),
  hard_reload: () => run('tab.reload', { tabId: activeTabId, ignoreCache: true }),
  back: () => activeTab()?.canGoBack && run('tab.back', { tabId: activeTabId }),
  forward: () => activeTab()?.canGoForward && run('tab.forward', { tabId: activeTabId }),
  history: () => run('ui.open_tool', { tool: 'history' }),
  downloads: () => run('ui.open_tool', { tool: 'downloads' }),
  bookmark: () => $('#star').click(),
  favorites_bar: () => run('settings.set', { settings: { favoritesBar: !favsOn } }),
  find: () => run('ui.open_tool', { tool: 'find' }),
  print: () => run('ui.print'),
  agent: () => run('ui.open_tool', { tool: 'agent' }),
  palette: () => run('ui.open_tool', { tool: 'palette' }),
};
for (let n = 1; n <= 9; n += 1) shortcuts[`tab_${n}`] = () => switchTab(n === 9 ? tabs.length - 1 : n - 1);
window.__agentgoShortcut = name => shortcuts[name]?.();

// The person's own bindings: {"Ctrl+K": "focus_address"}.
let customShortcuts = {};
const comboOf = event => {
  const key = /^[a-z0-9]$/i.test(event.key) ? event.key.toUpperCase() : /^F([1-9]|1[0-2])$/.test(event.key) ? event.key : null;
  if (!key) return null;
  return [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift', key].filter(Boolean).join('+');
};
const shortcutFor = event => {
  const custom = customShortcuts[comboOf(event)];
  if (custom) return custom;
  const key = event.key.toLowerCase();
  const { ctrlKey: ctrl, shiftKey: shift, altKey: alt } = event;
  if (ctrl && !alt) {
    if (key === 't') return shift ? 'reopen_tab' : 'new_tab';
    if (key === 'w' || key === 'f4') return 'close_tab';
    if (key === 'tab') return shift ? 'prev_tab' : 'next_tab';
    if (key === 'pagedown') return 'next_tab';
    if (key === 'pageup') return 'prev_tab';
    if (/^[1-9]$/.test(key) && !shift) return `tab_${key}`;
    if (key === 'l') return 'focus_address';
    if (key === 'r') return shift ? 'hard_reload' : 'reload';
    if (key === 'f5') return 'hard_reload';
    if (key === 'h') return 'history';
    if (key === 'j') return 'downloads';
    if (key === 'k' && !shift) return 'palette';
    if (key === 'd') return 'bookmark';
    if (key === 'b' && shift) return 'favorites_bar';
    if (key === 'f' && !shift) return 'find';
    if (key === 'p' && !shift) return 'print';
  } else if (alt && !ctrl) {
    if (key === 'd') return 'focus_address';
    if (key === 'arrowleft') return 'back';
    if (key === 'arrowright') return 'forward';
  } else if (!ctrl && !alt && !shift) {
    if (key === 'f5') return 'reload';
    if (key === 'f6') return 'focus_address';
  }
  return null;
};
document.addEventListener('keydown', event => {
  const name = shortcutFor(event);
  if (name) { event.preventDefault(); shortcuts[name](); }
});
document.addEventListener('contextmenu', event => { if (event.target !== address) event.preventDefault(); });

// Initial state: the native side also pushes after load, this covers reloads.
callBridge('tabs.list').then(window.__agentgoNativeTabs).catch(() => {});
callBridge('agent.status').then(window.__agentgoAgent).catch(() => {});
callBridge('settings.get').then(window.__agentgoTheme).catch(() => {});
refreshBookmarks();
refreshExtensions();


/* ---------- updates ---------- */
// A new release: with automatic updates it is already downloaded and goes in
// when AgentGo closes (a quiet "Restart to update"); without them the button
// is the way to get it.
let updateInfo = {};
window.__agentgoUpdate = info => {
  updateInfo = info || {};
  const button = $('#updateButton');
  if (!updateInfo.available) { button.hidden = true; return; }
  const staged = updateInfo.staged === true;
  button.hidden = false;
  button.classList.toggle('quiet', staged);
  $('#updateText').textContent = staged ? 'Restart to update' : `Update to ${updateInfo.available}`;
  button.title = updateInfo.error
    ? `AgentGo ${updateInfo.available} is available. The automatic download failed (${updateInfo.error}); click to try again.`
    : staged ? `AgentGo ${updateInfo.available} is ready. It installs when you close AgentGo, or now with a restart.`
      : `AgentGo ${updateInfo.available} is available. Click to download, check and install it (AgentGo restarts).`;
};
$('#updateButton').addEventListener('click', async () => {
  const button = $('#updateButton');
  button.disabled = true;
  $('#updateText').textContent = updateInfo.staged ? 'Restarting…' : 'Downloading…';
  try {
    await callBridge('browser.update_stage', { apply: 'now' });
  } catch (error) {
    $('#updateText').textContent = 'Update failed';
    button.title = String(error?.message || error);
    button.disabled = false;
  }
});
callBridge('browser.update_status').then(window.__agentgoUpdate, () => {});
