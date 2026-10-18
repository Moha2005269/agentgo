// The chrome.* API for an extension's own pages in AgentGo (its toolbar
// pop-up and options page), served from agentgo-ext://<id>/. Builds on the
// content-script shim (extshim.js, same storage), and adds what pages use:
// the address of their own files, the active tab, and opening a tab.
// Filled in by the browser: id, manifest, messages, saved storage, the tab.
((install, id, manifest, messages, saved, tab) => {
  const base = `agentgo-ext://${id}/`;
  // Storage is saved where the extension's content scripts read it.
  globalThis.__agExt = payload => {
    fetch(`${base}__agentgo/storage?p=${encodeURIComponent(payload)}`).catch(() => {});
  };
  install(id, manifest, messages, saved);
  const chrome = globalThis.chrome;
  const finish = (args, value) => {
    const callback = args.find(a => typeof a === 'function');
    if (callback) setTimeout(() => callback(value), 0);
    return Promise.resolve(value);
  };
  const events = () => ({ addListener() {}, removeListener() {}, hasListener: () => false });
  const url = path => base + String(path || '').replace(/^\//, '');
  chrome.runtime.getURL = url;
  chrome.runtime.openOptionsPage = (...rest) => {
    const page = (manifest.options_ui && manifest.options_ui.page) || manifest.options_page;
    if (page) open(url(page), '_blank');
    return finish(rest, undefined);
  };
  chrome.extension = Object.assign(chrome.extension || {}, { getURL: url, isAllowedIncognitoAccess: (...r) => finish(r, false) });
  const active = Object.assign({ id: 1, index: 0, windowId: 1, active: true, highlighted: true, pinned: false, incognito: false, status: 'complete' }, tab || {});
  chrome.tabs = Object.assign(chrome.tabs || {}, {
    query: (info, ...rest) => finish(rest, [active]),
    get: (tabId, ...rest) => finish(rest, active),
    getCurrent: (...rest) => finish(rest, undefined),
    create: (props, ...rest) => {
      const target = String((props && props.url) || '');
      if (/^https?:\/\//i.test(target)) fetch(`${base}__agentgo/open?u=${encodeURIComponent(target)}`).catch(() => {});
      else if (target) open(url(target), '_blank');
      return finish(rest, active);
    },
    sendMessage: (...args) => finish(args, undefined),
    onUpdated: events(), onActivated: events(), onRemoved: events(),
  });
  chrome.windows = Object.assign(chrome.windows || {}, { getCurrent: (...r) => finish(r, { id: 1, focused: true }), WINDOW_ID_CURRENT: -2 });
  const badge = { setBadgeText: (...r) => finish(r, undefined), setBadgeBackgroundColor: (...r) => finish(r, undefined), setTitle: (...r) => finish(r, undefined), setIcon: (...r) => finish(r, undefined), getBadgeText: (...r) => finish(r, ''), onClicked: events() };
  chrome.action = Object.assign(chrome.action || {}, badge);
  chrome.browserAction = Object.assign(chrome.browserAction || {}, badge);
  chrome.permissions = Object.assign(chrome.permissions || {}, { contains: (...r) => finish(r, true), request: (...r) => finish(r, false), getAll: (...r) => finish(r, { permissions: manifest.permissions || [], origins: manifest.host_permissions || [] }) });
  // An extension whose pop-up talks to its background worker (which AgentGo
  // does not run yet) may wait forever; say so instead of leaving a spinner.
  if (manifest.background) {
    addEventListener('DOMContentLoaded', () => {
      const note = document.createElement('div');
      note.setAttribute('role', 'note');
      note.style.cssText = 'position:fixed;left:0;right:0;bottom:0;z-index:2147483647;display:flex;gap:8px;align-items:center;padding:7px 10px;font:12px/1.35 system-ui,sans-serif;color:#f4f0e8;background:#1c1714;border-top:1px solid #ff6a2b';
      note.innerHTML = '<span style="flex:1">Parts of this extension run in its background worker, which AgentGo does not run yet, so some of it may not respond.</span><button type="button" aria-label="Dismiss" style="all:unset;cursor:pointer;padding:0 4px;font-size:15px">×</button>';
      note.querySelector('button').onclick = () => note.remove();
      document.body.appendChild(note);
    });
  }
})
