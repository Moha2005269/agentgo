// Runs first in the pages (pop-up, options) of an extension the engine runs.
// Their tabs live in AgentGo's own window, which Chromium's tab list does not
// know: chrome.tabs.query would answer with no tab and chrome.tabs.create and
// openOptionsPage would fail. They are answered here instead, for the tab the
// person opened the pop-up over; everything else is Chromium's own API.
((active, extensionId) => {
  const tabs = globalThis.chrome && chrome.tabs;
  if (!tabs || !tabs.query) return;
  const ask = (message) => { try { globalThis.__agExtTabs(JSON.stringify(message)); } catch (_) { /* binding gone */ } };
  const tab = () => ({
    id: -1, index: 0, windowId: -1, openerTabId: undefined, active: true, highlighted: true, selected: true,
    pinned: false, audible: false, discarded: false, autoDiscardable: true, incognito: false, groupId: -1,
    status: 'complete', url: active.url || '', title: active.title || '', favIconUrl: active.favIconUrl || '',
    width: 0, height: 0,
  });
  const answer = (value, callback) => {
    if (typeof callback === 'function') { setTimeout(() => callback(value), 0); return undefined; }
    return Promise.resolve(value);
  };
  const query = tabs.query.bind(tabs);
  const wantsActive = (info) => !info || info.active || info.currentWindow || info.lastFocusedWindow || info.highlighted;
  tabs.query = function (info, callback) {
    const native = new Promise((resolve) => { try { query(info || {}, (found) => resolve(found || [])); } catch (_) { resolve([]); } });
    const result = native.then((found) => (found.length || !wantsActive(info) || !active.url) ? found : [tab()]);
    if (typeof callback === 'function') { result.then(callback); return undefined; }
    return result;
  };
  if (tabs.getSelected) tabs.getSelected = (windowId, callback) => answer(tab(), typeof windowId === 'function' ? windowId : callback);
  const allowed = (url) => /^https?:\/\//i.test(url) || url.startsWith(`chrome-extension://${extensionId}/`);
  const create = tabs.create && tabs.create.bind(tabs);
  tabs.create = function (properties, callback) {
    let url = (properties && properties.url) || '';
    if (url && !/^[a-z][a-z0-9+.-]*:/i.test(url)) url = new URL(url, location.href).href;
    if (allowed(url)) {
      ask({ op: 'create', url, active: !properties || properties.active !== false });
      return answer({ ...tab(), url, title: '', active: true }, callback);
    }
    return create ? create(properties, callback) : answer(undefined, callback);
  };
  if (chrome.runtime && chrome.runtime.openOptionsPage) {
    chrome.runtime.openOptionsPage = (callback) => {
      ask({ op: 'options' });
      return answer(undefined, callback);
    };
  }
})
