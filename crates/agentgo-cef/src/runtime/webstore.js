// The Chrome Web Store, inside AgentGo: its own "Add to Chrome" button
// installs the extension into AgentGo. Runs in the page before the store's
// scripts, only on chromewebstore.google.com. The request goes to AgentGo's
// world on the page (window.postMessage), which asks the browser; the
// browser installs only the extension of the page that is open, and only
// after a click by the person.
(() => {
  if (location.hostname !== 'chromewebstore.google.com') return;
  const pending = new Map();
  addEventListener('message', event => {
    const data = event.source === window && event.data;
    if (!data || data.__agentgo !== 'ext-installed') return;
    window.__agentgoInstalled = Array.from(new Set([...(window.__agentgoInstalled || []), ...(data.ok ? [data.id] : [])]));
    const done = pending.get(data.id);
    pending.delete(data.id);
    if (done) done(data.ok ? '' : 'unknown_error');
  });
  const reply = (args, value) => {
    const callback = args.find(a => typeof a === 'function');
    if (callback) setTimeout(() => callback(value), 0);
    return Promise.resolve(value);
  };
  const status = id => (window.__agentgoInstalled || []).includes(id) ? 'enabled' : 'installable';
  const shim = {
    getExtensionStatus: (id, ...rest) => reply(rest, status(id)),
    beginInstallWithManifest3: (details, ...rest) => {
      const id = details && details.id;
      const result = new Promise(resolve => {
        pending.set(id, resolve);
        window.postMessage({ __agentgo: 'ext-install', id }, location.origin);
        setTimeout(() => { if (pending.has(id)) { pending.delete(id); resolve('unknown_error'); } }, 120000);
      });
      const callback = rest.find(a => typeof a === 'function');
      if (callback) result.then(callback);
      return result;
    },
    completeInstall: (id, ...rest) => reply(rest, undefined),
    enableAppLauncher: (...rest) => reply(rest, undefined),
    isInIncognitoMode: (...rest) => reply(rest, false),
    getReferrerChain: (...rest) => reply(rest, ''),
    getBrowserLogin: (...rest) => reply(rest, { login: '' }),
    getStoreLogin: (...rest) => reply(rest, ''),
    setStoreLogin: (login, ...rest) => reply(rest, undefined),
    isPendingCustodianApproval: (id, ...rest) => reply(rest, false),
    getMV2DeprecationStatus: (...rest) => reply(rest, 'inactive'),
  };
  const install = () => {
    try {
      const chrome = window.chrome || (window.chrome = {});
      const base = chrome.webstorePrivate || {};
      const merged = new Proxy(base, { get: (target, key) => (key in shim ? shim[key] : Reflect.get(target, key)) });
      Object.defineProperty(chrome, 'webstorePrivate', { configurable: true, get: () => merged });
      if (chrome.management && chrome.management.getAll) {
        const mgmt = new Proxy(chrome.management, {
          get: (target, key) => key === 'getAll' ? ((...rest) => reply(rest, []))
            : key === 'get' ? ((id, ...rest) => reply(rest, status(id) === 'enabled' ? { id, enabled: true } : undefined))
            : Reflect.get(target, key),
        });
        Object.defineProperty(chrome, 'management', { configurable: true, get: () => mgmt });
      }
    } catch (error) { /* the store keeps its own behaviour */ }
  };
  install();
})();
