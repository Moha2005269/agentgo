// A small part of the chrome.* extension API for an extension's content
// scripts in AgentGo (each extension runs in its own isolated world). Filled
// in by the browser: the extension id, its manifest, its messages and its
// saved storage. Storage changes are saved through __agExt.
((id, manifest, messages, saved) => {
  const chrome = globalThis.chrome = globalThis.chrome || {};
  const listeners = () => { const set = new Set(); return { addListener: f => set.add(f), removeListener: f => set.delete(f), hasListener: f => set.has(f), hasListeners: () => set.size > 0, _fire: (...a) => set.forEach(f => { try { f(...a); } catch (e) {} }) }; };
  const finish = (args, value) => {
    const callback = args.find(a => typeof a === 'function');
    if (callback) setTimeout(() => callback(value), 0);
    return Promise.resolve(value);
  };
  chrome.runtime = Object.assign(chrome.runtime || {}, {
    id,
    getManifest: () => JSON.parse(JSON.stringify(manifest)),
    getURL: path => `chrome-extension://${id}/${String(path || '').replace(/^\//, '')}`,
    // There is no background page in AgentGo: messages to it get no answer.
    sendMessage: (...args) => finish(args, undefined),
    connect: () => ({ name: '', postMessage() {}, disconnect() {}, onMessage: listeners(), onDisconnect: listeners() }),
    onMessage: listeners(),
    onConnect: listeners(),
    onInstalled: listeners(),
    lastError: undefined,
  });
  chrome.i18n = Object.assign(chrome.i18n || {}, {
    getUILanguage: () => navigator.language,
    getMessage: (name, substitutions) => {
      const entry = messages[String(name || '').toLowerCase()];
      if (!entry) return '';
      const subs = Array.isArray(substitutions) ? substitutions : substitutions == null ? [] : [substitutions];
      let text = String(entry.message || '');
      const placeholders = entry.placeholders || {};
      text = text.replace(/\$([A-Za-z0-9_@]+)\$/g, (_, key) => {
        const ph = placeholders[key.toLowerCase()] || placeholders[key];
        return ph ? String(ph.content || '').replace(/\$(\d)/g, (__, n) => subs[n - 1] ?? '') : '';
      });
      return text.replace(/\$(\d)/g, (_, n) => subs[n - 1] ?? '');
    },
  });
  const data = Object.assign({}, saved);
  const changed = listeners();
  const pick = keys => {
    if (keys == null) return Object.assign({}, data);
    if (typeof keys === 'string') return keys in data ? { [keys]: data[keys] } : {};
    if (Array.isArray(keys)) return Object.fromEntries(keys.filter(k => k in data).map(k => [k, data[k]]));
    return Object.fromEntries(Object.entries(keys).map(([k, d]) => [k, k in data ? data[k] : d]));
  };
  const save = (values, removed) => { try { globalThis.__agExt && globalThis.__agExt(JSON.stringify({ id, set: values || {}, remove: removed || [] })); } catch (e) {} };
  const area = name => ({
    get: (keys, ...rest) => finish(typeof keys === 'function' ? [keys] : rest, pick(typeof keys === 'function' ? null : keys)),
    set: (values, ...rest) => {
      const changes = {};
      for (const [k, v] of Object.entries(values || {})) { changes[k] = { oldValue: data[k], newValue: v }; data[k] = v; }
      save(values);
      changed._fire(changes, name);
      return finish(rest, undefined);
    },
    remove: (keys, ...rest) => { const list = [].concat(keys); list.forEach(k => delete data[k]); save({}, list); return finish(rest, undefined); },
    clear: (...rest) => { const list = Object.keys(data); list.forEach(k => delete data[k]); save({}, list); return finish(rest, undefined); },
    getBytesInUse: (...rest) => finish(rest, JSON.stringify(data).length),
    onChanged: changed,
  });
  chrome.storage = Object.assign(chrome.storage || {}, { local: area('local'), sync: area('sync'), session: area('session'), managed: area('managed'), onChanged: changed });
})
