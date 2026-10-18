// The Chrome Web Store, inside AgentGo: its own "Add to Chrome" button
// installs the extension into AgentGo. Runs in the page before the store's
// scripts, only on chromewebstore.google.com. The request goes to AgentGo's
// world on the page (window.postMessage), which asks the browser; the
// browser installs only the extension of the page that is open, and only
// after a click by the person.
(() => {
  if (location.hostname !== 'chromewebstore.google.com') return;
  // The store offers "Add to Chrome" only to a browser that names Google
  // Chrome among its brands (this engine says Chromium), and otherwise asks
  // the person to switch to Chrome. On the store's own pages, and only there,
  // AgentGo names it too; the install still goes through AgentGo.
  try {
    const data = navigator.userAgentData;
    if (data && !data.brands.some(b => b.brand === 'Google Chrome')) {
      const major = (data.brands.find(b => b.brand === 'Chromium') || {}).version || '152';
      const brands = [...data.brands, { brand: 'Google Chrome', version: major }];
      const high = data.getHighEntropyValues.bind(data);
      const named = Object.create(Object.getPrototypeOf(data), {
        brands: { get: () => brands.map(b => ({ ...b })) },
        mobile: { get: () => data.mobile },
        platform: { get: () => data.platform },
        toJSON: { value: () => ({ brands, mobile: data.mobile, platform: data.platform }) },
        getHighEntropyValues: {
          value: hints => high(hints).then(values => {
            if (Array.isArray(values.fullVersionList)) {
              const full = (values.fullVersionList.find(b => b.brand === 'Chromium') || {}).version || `${major}.0.0.0`;
              values.fullVersionList = [...values.fullVersionList, { brand: 'Google Chrome', version: full }];
            }
            if (Array.isArray(values.brands)) values.brands = brands.map(b => ({ ...b }));
            return values;
          }),
        },
      });
      Object.defineProperty(Navigator.prototype, 'userAgentData', { configurable: true, get: () => named });
    }
  } catch (error) { /* the store keeps its own behaviour */ }
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
    // The store reads Chrome's full version before it offers the button.
    getFullChromeVersion: (...rest) => reply(rest, { version_number: chromeVersion() }),
  };
  const chromeVersion = () => (navigator.userAgent.match(/Chrome\/([\d.]+)/) || [])[1] || '';
  // What the store asks chrome.management: which extensions are installed
  // (AgentGo's, as far as the store page knows them).
  const management = {
    getAll: (...rest) => reply(rest, (window.__agentgoInstalled || []).map(id => ({ id, enabled: true, type: 'extension' }))),
    get: (id, ...rest) => reply(rest, status(id) === 'enabled' ? { id, enabled: true, type: 'extension' } : undefined),
    getSelf: (...rest) => reply(rest, undefined),
    setEnabled: (id, enabled, ...rest) => reply(rest, undefined),
    uninstall: (id, ...rest) => reply(rest, undefined),
    onInstalled: { addListener() {}, removeListener() {}, hasListener: () => false },
    onUninstalled: { addListener() {}, removeListener() {}, hasListener: () => false },
    onEnabled: { addListener() {}, removeListener() {}, hasListener: () => false },
    onDisabled: { addListener() {}, removeListener() {}, hasListener: () => false },
  };
  const install = () => {
    try {
      const chrome = window.chrome || (window.chrome = {});
      const base = chrome.webstorePrivate || {};
      const merged = new Proxy(base, { get: (target, key) => (key in shim ? shim[key] : Reflect.get(target, key)) });
      Object.defineProperty(chrome, 'webstorePrivate', { configurable: true, get: () => merged });
      // The store only offers "Add to Chrome" when chrome.management exists;
      // this engine has none on web pages, so the store page gets AgentGo's.
      const mgmt = chrome.management
        ? new Proxy(chrome.management, { get: (target, key) => (key in management ? management[key] : Reflect.get(target, key)) })
        : management;
      Object.defineProperty(chrome, 'management', { configurable: true, get: () => mgmt });
    } catch (error) { /* the store keeps its own behaviour */ }
  };
  install();
})();
