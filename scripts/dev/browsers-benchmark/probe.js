// What a page can observe about the browser. Returns one JSON object.
(async () => {
  const out = {};
  const nav = navigator;
  out.ua = nav.userAgent;
  out.webdriver = nav.webdriver;
  out.languages = nav.languages;
  out.platform = nav.platform;
  out.hardwareConcurrency = nav.hardwareConcurrency;
  out.deviceMemory = nav.deviceMemory;
  out.plugins = Array.from(nav.plugins || []).map(p => p.name);
  out.mimeTypes = (nav.mimeTypes || []).length;
  out.pdfViewerEnabled = nav.pdfViewerEnabled;
  out.uaData = nav.userAgentData ? { brands: nav.userAgentData.brands, mobile: nav.userAgentData.mobile, platform: nav.userAgentData.platform } : null;
  try { out.uaHigh = nav.userAgentData ? await nav.userAgentData.getHighEntropyValues(['fullVersionList', 'platformVersion', 'architecture', 'model', 'bitness', 'wow64']) : null; } catch (e) { out.uaHigh = String(e); }
  out.chromeKeys = window.chrome ? Object.keys(window.chrome).sort() : null;
  out.chromeRuntime = !!(window.chrome && window.chrome.runtime);
  out.notification = typeof Notification !== 'undefined' ? Notification.permission : 'none';
  try { out.permNotifications = (await navigator.permissions.query({ name: 'notifications' })).state; } catch (e) { out.permNotifications = String(e); }
  out.screen = [screen.width, screen.height, screen.availWidth, screen.availHeight, screen.colorDepth, devicePixelRatio];
  out.window = [innerWidth, innerHeight, outerWidth, outerHeight];
  out.timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  out.customElementsInDom = Array.from(document.querySelectorAll('*')).map(e => e.tagName.toLowerCase()).filter(t => t.includes('-'));
  out.globalsUnusual = Object.getOwnPropertyNames(window).filter(k => /agentgo|__ag|cdc_|\$cdc|webdriver|playwright|puppeteer|selenium|domAutomation/i.test(k));
  // DevTools protocol (Runtime.enable) detection: console.debug serialises
  // an Error's stack only when a protocol client listens to the console.
  out.cdpRuntimeListening = await new Promise(resolve => {
    let hit = false;
    const e = new Error();
    Object.defineProperty(e, 'stack', { configurable: false, enumerable: false, get() { hit = true; return ''; } });
    console.debug(e);
    setTimeout(() => resolve(hit), 50);
  });
  out.toStringNative = [HTMLCanvasElement.prototype.toDataURL, CanvasRenderingContext2D.prototype.getImageData, Function.prototype.toString].map(f => Function.prototype.toString.call(f));
  out.webglVendor = (() => { try { const g = document.createElement('canvas').getContext('webgl'); const x = g.getExtension('WEBGL_debug_renderer_info'); return [g.getParameter(x.UNMASKED_VENDOR_WEBGL), g.getParameter(x.UNMASKED_RENDERER_WEBGL)]; } catch (e) { return String(e); } })();
  return JSON.stringify(out);
})()
