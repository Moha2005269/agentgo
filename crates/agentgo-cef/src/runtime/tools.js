// Page tools: the small, single-purpose commands of the tool register that
// need nothing but the document. They run in AgentGo's isolated world, like
// the page agent they extend, and are reached as `__ag.tool(name, params)`.
// A tool returns a plain object; `{error, detail}` becomes a typed failure.
(() => {
  'use strict';
  const { lookup, visible, clean, refOf, describe, signals, risk } = globalThis.__ag.h;
  const all = (selector, root = document) => [...root.querySelectorAll(selector)];
  const shown = (selector, root) => all(selector, root).filter(visible);
  const found = (els, max = 20) => ({ count: els.length, matches: els.slice(0, max).map(el => ({ ref: refOf(el), line: describe(el) })) });
  const fail = (error, detail) => ({ error, detail });
  const need = (params, key) => {
    if (params[key] === undefined || params[key] === null || params[key] === '') throw fail('params', `params.${key} is required`);
    return params[key];
  };
  // The element a tool is about: a reference from the outline, or a selector.
  const target = params => {
    if (params.ref !== undefined) {
      const hit = lookup(params.ref);
      if (hit.error) throw hit;
      return hit.el;
    }
    const el = document.querySelector(need(params, 'selector'));
    if (!el) throw fail('stale', `nothing matches ${params.selector}`);
    return el;
  };
  const text = el => clean(el.innerText || el.textContent || '', 200);
  // A click from here gets no approval: anything that pays or submits goes
  // through act.click, which asks the person.
  const safeClick = el => {
    if (risk(el)) throw fail('policy', `${describe(el)} pays or submits a form; use act.click, which asks the person`);
    el.click();
  };
  const has = (el, needle, exact) => {
    const value = text(el).toLowerCase();
    const want = String(needle).toLowerCase();
    return exact ? value === want : value.includes(want);
  };
  const labelOf = el => clean((el.labels && [...el.labels].map(l => l.innerText).join(' ')) || el.getAttribute('aria-label') || '', 120);
  const fields = () => shown('input:not([type=hidden]),textarea,select,[contenteditable=""],[contenteditable=true]');
  const rect = el => { const b = el.getBoundingClientRect(); return { x: Math.round(b.left), y: Math.round(b.top), width: Math.round(b.width), height: Math.round(b.height) }; };
  const hash = value => { let h = 0xcbf29ce484222325n; for (const ch of value) h = BigInt.asUintN(64, (h ^ BigInt(ch.codePointAt(0))) * 0x100000001b3n); return h.toString(16); };
  const store = kind => kind === 'session' ? sessionStorage : localStorage;
  const media = params => {
    const list = all('video,audio');
    const el = params.ref !== undefined || params.selector ? target(params) : list[params.index || 0];
    if (!el || !/^(VIDEO|AUDIO)$/.test(el.tagName)) throw fail('stale', 'no media element on this page');
    return el;
  };
  const mediaState = el => ({ kind: el.tagName.toLowerCase(), src: el.currentSrc || el.src || '', paused: el.paused, muted: el.muted, volume: el.volume, time: el.currentTime, duration: Number.isFinite(el.duration) ? el.duration : null, speed: el.playbackRate, ended: el.ended });
  const luminance = color => {
    const [r, g, b] = (color.match(/[\d.]+/g) || [0, 0, 0]).slice(0, 3).map(Number).map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  // Values are written the way an input event would leave them; payment
  // fields are refused (they go through act.fill and its approval).
  const payment = el => /^cc-/.test(el.autocomplete || '') || /card|cvv|cvc|expir/i.test(`${el.name} ${el.id}`);
  // A password field's value is never handed to an agent: credentials.fill
  // types saved sign-ins without the agent seeing them.
  const secret = el => el instanceof HTMLInputElement && el.type === 'password';
  const readable = (el, name) => {
    if (secret(el) && /^(value|defaultvalue)$/i.test(String(name))) throw fail('policy', 'the value of a password field is not readable');
    return el;
  };
  const SETTABLE = new Set(['value', 'checked', 'selected', 'selectedIndex', 'indeterminate', 'title', 'alt', 'placeholder', 'textContent', 'innerText', 'hidden', 'open', 'scrollTop', 'scrollLeft', 'currentTime', 'volume', 'muted', 'playbackRate']);
  const setValue = (el, types, value) => {
    if (el.tagName !== 'INPUT' || !types.includes(el.type)) throw fail('params', `${describe(el)} is not a ${types.join(' or ')} field`);
    if (payment(el)) throw fail('payment', 'payment fields are filled with act.fill, which asks the person first');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(el, String(value));
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    if (value !== '' && el.value === '') throw fail('params', `the browser rejected ${JSON.stringify(value)} for a ${el.type} field`);
    return { value: el.value };
  };
  const sleep = ms => new Promise(done => setTimeout(done, ms));
  // Polls `check` until it returns something, for at most 8 s (page tools
  // have a 10 s budget).
  const poll = async (p, what, check) => {
    const end = Date.now() + Math.min(Number(p.timeout) || 5000, 8000);
    for (;;) {
      const value = check();
      if (value) return value;
      if (Date.now() > end) throw fail('timeout', `${what} did not happen in time`);
      await sleep(100);
    }
  };
  const devices = async kind => ({ devices: (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === kind).map(d => ({ id: d.deviceId, label: d.label || '(name hidden until the site may use it)' })) });
  const permissions = async names => { const out = {}; for (const name of names) { try { out[name] = (await navigator.permissions.query({ name })).state; } catch (_) { out[name] = 'unknown'; } } return out; };
  const items = p => { const list = need(p, 'items'); if (!Array.isArray(list)) throw fail('params', 'params.items must be an array'); return list; };
  const field = (item, key) => key === undefined ? item : item && item[key];
  const words = value => String(value ?? '').toLowerCase().split(/\W+/).filter(Boolean);
  const background = el => { for (let at = el; at; at = at.parentElement) { const c = getComputedStyle(at).backgroundColor; if (c && !/rgba?\(0, 0, 0, 0\)|transparent/.test(c)) return c; } return 'rgb(255, 255, 255)'; };

  const tools = {
    // ---- locate: find elements and get references to act on ---------------
    'locate.role': p => found(shown('*').filter(el => describe(el).split(' ')[0].replace(/\(.*/, '') === need(p, 'role') && (!p.name || has(el, p.name) || labelOf(el).toLowerCase().includes(String(p.name).toLowerCase())))),
    'locate.name': p => found(shown('a,button,input,select,textarea,[role],summary').filter(el => describe(el).toLowerCase().includes(`"${String(need(p, 'name')).toLowerCase()}`))),
    'locate.label': p => found(fields().filter(el => labelOf(el).toLowerCase().includes(String(need(p, 'label')).toLowerCase()))),
    'locate.placeholder': p => found(fields().filter(el => (el.placeholder || '').toLowerCase().includes(String(need(p, 'placeholder')).toLowerCase()))),
    'locate.text': p => found(shown('body *').filter(el => !el.children.length && has(el, need(p, 'text'), p.exact))),
    'locate.text_pattern': p => { const pattern = new RegExp(need(p, 'pattern'), p.flags || 'i'); return found(shown('body *').filter(el => !el.children.length && pattern.test(text(el)))); },
    'locate.link': p => found(shown('a[href]').filter(el => !p.text || has(el, p.text) || el.href.includes(p.text))),
    'locate.button': p => found(shown('button,input[type=button],input[type=submit],[role=button]').filter(el => !p.text || has(el, p.text) || (el.value || '').toLowerCase().includes(String(p.text).toLowerCase()))),
    'locate.field': p => found(fields().filter(el => !p.name || [labelOf(el), el.name, el.id, el.placeholder].join(' ').toLowerCase().includes(String(p.name).toLowerCase()))),
    'locate.checkbox': p => found(shown('input[type=checkbox],[role=checkbox]').filter(el => !p.name || labelOf(el).toLowerCase().includes(String(p.name).toLowerCase()))),
    'locate.radio': p => found(shown('input[type=radio],[role=radio]').filter(el => !p.name || labelOf(el).toLowerCase().includes(String(p.name).toLowerCase()))),
    'locate.select': p => found(shown('select,[role=combobox],[role=listbox]').filter(el => !p.name || labelOf(el).toLowerCase().includes(String(p.name).toLowerCase()))),
    'locate.file_input': () => found(all('input[type=file]')),
    'locate.test_id': p => found(all(`[data-testid="${CSS.escape(need(p, 'id'))}"],[data-test-id="${CSS.escape(p.id)}"],[data-test="${CSS.escape(p.id)}"]`)),
    'locate.css': p => found(all(need(p, 'selector'))),
    'locate.xpath': p => { const out = []; const it = document.evaluate(need(p, 'xpath'), document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null); for (let i = 0; i < it.snapshotLength; i += 1) if (it.snapshotItem(i).nodeType === 1) out.push(it.snapshotItem(i)); return found(out); },
    'locate.all': p => found(all(need(p, 'selector')), 100),
    'locate.nth': p => { const el = all(need(p, 'selector'))[p.index || 0]; return el ? found([el]) : fail('stale', 'no element at that index'); },
    'locate.unique': p => { const els = all(need(p, 'selector')); return els.length === 1 ? found(els) : fail(els.length ? 'ambiguous' : 'stale', `${els.length} elements match ${p.selector}`); },
    'locate.closest': p => { const el = target(p).closest(need(p, 'ancestor')); return el ? found([el]) : fail('stale', 'no such ancestor'); },
    'locate.descendant': p => found(all(need(p, 'descendant'), target(p))),
    'locate.filter': p => found(all(need(p, 'selector')).filter(el => (!p.text || has(el, p.text)) && (!p.visible || visible(el)))),
    'locate.coordinate': p => { const el = document.elementFromPoint(need(p, 'x'), need(p, 'y')); return el ? found([el]) : fail('stale', 'nothing at that point'); },
    'locate.region': p => found(shown('a,button,input,select,textarea,[role]').filter(el => { const b = el.getBoundingClientRect(); return b.left >= need(p, 'x') && b.top >= need(p, 'y') && b.right <= p.x + need(p, 'width') && b.bottom <= p.y + need(p, 'height'); })),
    'locate.frame': () => ({ count: all('iframe,frame').length, frames: all('iframe,frame').map((f, index) => ({ index, name: f.name || '', src: f.src || '', ...rect(f) })) }),
    'locate.shadow': p => { const out = []; const walk = root => { for (const el of all('*', root)) if (el.shadowRoot) { out.push(...all(need(p, 'selector'), el.shadowRoot)); walk(el.shadowRoot); } }; walk(document); return found(out); },
    'locate.dom_ref': p => found([target(p)]),
    'locate.explain': p => { const el = target(p); return { line: describe(el), tag: el.tagName.toLowerCase(), id: el.id || null, classes: [...el.classList].slice(0, 8), visible: visible(el), rect: rect(el), label: labelOf(el) }; },

    // ---- runtime: read and write single things -----------------------------
    'runtime.query': p => { const el = document.querySelector(need(p, 'selector')); return el ? { found: true, ...found([el]).matches[0], text: text(el) } : { found: false }; },
    'runtime.query_all': p => found(all(need(p, 'selector')), p.max || 50),
    'runtime.get_attribute': p => ({ value: readable(target(p), need(p, 'name')).getAttribute(p.name) }),
    'runtime.get_property': p => { const value = readable(target(p), need(p, 'name'))[p.name]; return { value: typeof value === 'object' && value !== null ? String(value) : value ?? null }; },
    'runtime.read_property': p => { const value = readable(target(p), need(p, 'name'))[p.name]; return { value: typeof value === 'object' && value !== null ? String(value) : value ?? null }; },
    'runtime.set_property': p => {
      const el = target(p);
      const name = need(p, 'name');
      // Only plain state: markup, addresses, handlers and an input's type
      // could run script or unmask a password (that is runtime.evaluate's job,
      // behind its own gate).
      if (!SETTABLE.has(name)) throw fail('policy', `${name} cannot be set with runtime.set_property; use act.* or runtime.evaluate`);
      if (name === 'value' && el.tagName === 'INPUT' && payment(el)) throw fail('payment', 'payment fields are filled with act.fill, which asks the person first');
      el[name] = p.value;
      return { value: secret(el) ? null : el[name] ?? null };
    },
    'runtime.get_text': p => ({ text: clean(target(p).innerText || target(p).textContent || '', p.max || 4000) }),
    'runtime.get_html': p => ({ html: target(p).outerHTML.slice(0, p.max || 20000) }),
    'runtime.get_value': p => { const el = target(p); return { value: el.type === 'password' ? null : el.value ?? el.innerText ?? null }; },
    'runtime.get_rect': p => rect(target(p)),
    // Visible status, alert and live-region text (a compact outline leaves it out).
    'observe.status': () => ({ status: [...new Set(all('[role=status],[role=alert],[aria-live]:not([aria-live=off]),output').filter(visible).map(el => clean(el.innerText || el.textContent || '', 200)).filter(Boolean))].slice(0, 5) }),

    // ---- storage of the page's own origin ----------------------------------
    'storage.local_get': p => p.key === undefined ? { keys: Object.keys(localStorage) } : { value: localStorage.getItem(p.key) },
    'storage.local_set': p => { localStorage.setItem(need(p, 'key'), String(need(p, 'value'))); return { stored: true }; },
    'storage.local_delete': p => { localStorage.removeItem(need(p, 'key')); return { deleted: true }; },
    'storage.local_clear': () => { const count = localStorage.length; localStorage.clear(); return { cleared: count }; },
    'storage.session_get': p => p.key === undefined ? { keys: Object.keys(sessionStorage) } : { value: sessionStorage.getItem(p.key) },
    'storage.session_set': p => { sessionStorage.setItem(need(p, 'key'), String(need(p, 'value'))); return { stored: true }; },
    'storage.session_clear': () => { const count = sessionStorage.length; sessionStorage.clear(); return { cleared: count }; },
    'storage.indexeddb_list': async () => ({ databases: indexedDB.databases ? (await indexedDB.databases()).map(db => ({ name: db.name, version: db.version })) : [] }),
    'storage.cache_list': async () => ({ caches: globalThis.caches ? await caches.keys() : [] }),
    'storage.cache_clear': async () => { const keys = globalThis.caches ? await caches.keys() : []; await Promise.all(keys.map(key => caches.delete(key))); return { cleared: keys.length }; },
    'storage.service_workers': async () => ({ workers: navigator.serviceWorker ? (await navigator.serviceWorker.getRegistrations()).map(r => ({ scope: r.scope, active: Boolean(r.active) })) : [] }),
    'storage.service_worker_unregister': async () => { const regs = navigator.serviceWorker ? await navigator.serviceWorker.getRegistrations() : []; await Promise.all(regs.map(r => r.unregister())); return { unregistered: regs.length }; },
    'navigate.service_worker_status': async () => ({ controlled: Boolean(navigator.serviceWorker && navigator.serviceWorker.controller), registrations: navigator.serviceWorker ? (await navigator.serviceWorker.getRegistrations()).length : 0 }),

    // ---- focus and selection ------------------------------------------------
    'focus.read': () => { const el = document.activeElement; return el && el !== document.body ? { focused: true, ...found([el]).matches[0] } : { focused: false }; },
    'keyboard.focused_element': () => tools['focus.read'](),
    'observe.focus': () => tools['focus.read'](),
    'focus.element': p => { const el = target(p); el.focus(); return { focused: document.activeElement === el, line: describe(el) }; },
    'act.focus': p => tools['focus.element'](p),
    'focus.blur': () => { const el = document.activeElement; if (el && el.blur) el.blur(); return { focused: false }; },
    'selection.read': () => globalThis.__ag.selection(),
    'observe.selection': () => globalThis.__ag.selection(),
    'extract.selection': () => globalThis.__ag.selection(),
    'selection.set': p => { const el = target(p); if (el.select && /^(INPUT|TEXTAREA)$/.test(el.tagName)) { el.focus(); el.setSelectionRange(p.start || 0, p.end ?? el.value.length); } else { const range = document.createRange(); range.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(range); } return globalThis.__ag.selection(); },
    'selection.clear': () => { getSelection().removeAllRanges(); return { text: '' }; },
    'edit.clear': p => { const el = target(p); el.focus(); if ('value' in el) { Object.getOwnPropertyDescriptor(el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, 'value').set.call(el, ''); el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); } else el.textContent = ''; return { value: el.value ?? el.textContent }; },

    // ---- observe: specific views of the page -------------------------------
    'viewport.size': () => ({ width: innerWidth, height: innerHeight, scale: devicePixelRatio, scrollX: Math.round(scrollX), scrollY: Math.round(scrollY), pageWidth: document.documentElement.scrollWidth, pageHeight: document.documentElement.scrollHeight }),
    'observe.viewport': () => tools['viewport.size'](),
    'observe.lists': p => ({ lists: shown('ul,ol').slice(0, p.max || 10).map(list => ({ ordered: list.tagName === 'OL', items: [...list.children].filter(li => li.tagName === 'LI').slice(0, 50).map(li => text(li)) })) }),
    'extract.list': p => tools['observe.lists'](p),
    'observe.images': p => ({ images: shown('img').slice(0, p.max || 50).map(img => ({ ref: refOf(img), src: img.currentSrc || img.src, alt: img.alt || '', width: img.naturalWidth, height: img.naturalHeight })) }),
    'extract.images': p => tools['observe.images'](p),
    'observe.media': () => ({ media: all('video,audio').map((el, index) => ({ index, ...mediaState(el) })) }),
    'extract.media': () => tools['observe.media'](),
    'media.list': () => tools['observe.media'](),
    'observe.frames': () => tools['locate.frame'](),
    'frame.list': () => tools['locate.frame'](),
    'observe.shadow_roots': () => { const hosts = all('*').filter(el => el.shadowRoot); return { count: hosts.length, hosts: hosts.slice(0, 30).map(el => ({ tag: el.tagName.toLowerCase(), id: el.id || null, children: el.shadowRoot.childElementCount })) }; },
    'observe.overlays': () => found(shown('dialog[open],[role=dialog],[role=alertdialog],[aria-modal=true]').concat(shown('body *').filter(el => { const s = getComputedStyle(el); const b = el.getBoundingClientRect(); return s.position === 'fixed' && b.width >= innerWidth * 0.5 && b.height >= innerHeight * 0.5; }))),
    'observe.fixed_elements': () => found(shown('body *').filter(el => /fixed|sticky/.test(getComputedStyle(el).position))),
    'observe.scroll_containers': () => found(shown('body *').filter(el => /(auto|scroll)/.test(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight + 4)),
    'observe.loading_indicators': () => found(shown('[aria-busy=true],[role=progressbar],progress,.spinner,.loading,.skeleton')),
    'observe.canvas': () => ({ canvases: all('canvas').map(c => ({ ref: refOf(c), width: c.width, height: c.height, ...rect(c) })) }),
    'observe.computed_style': p => { const style = getComputedStyle(target(p)); const names = p.properties || ['display', 'visibility', 'color', 'background-color', 'font-size', 'position', 'z-index', 'opacity']; return { style: Object.fromEntries(names.slice(0, 40).map(name => [name, style.getPropertyValue(name)])) }; },
    'observe.favicon': () => ({ icons: all('link[rel~=icon],link[rel="apple-touch-icon"]').map(l => l.href) }),
    'observe.hit_test': p => { const el = document.elementFromPoint(need(p, 'x'), need(p, 'y')); return el ? { hit: true, ...found([el]).matches[0], tag: el.tagName.toLowerCase() } : { hit: false }; },
    'pointer.hit_test': p => tools['observe.hit_test'](p),
    'observe.layout': p => ({ elements: shown('a,button,input,select,textarea,[role],h1,h2,h3,img').slice(0, p.max || 60).map(el => ({ line: describe(el), ...rect(el) })) }),
    'observe.animation': () => ({ running: document.getAnimations().filter(a => a.playState === 'running').length }),
    'state.content_hash': () => ({ hash: hash(document.body ? document.body.innerText : ''), length: document.body ? document.body.innerText.length : 0 }),
    'state.fingerprint': () => ({ fingerprint: hash(location.href + '|' + document.title + '|' + all('a,button,input,select,textarea').length + '|' + (document.body ? document.body.innerText.length : 0)) }),
    'state.page_identity': () => ({ url: location.href, origin: location.origin, title: document.title, lang: document.documentElement.lang || null, canonical: (document.querySelector('link[rel=canonical]') || {}).href || null }),
    'evidence.page_identity': () => tools['state.page_identity'](),

    // ---- media --------------------------------------------------------------
    'media.status': p => mediaState(media(p)),
    'media.play': async p => {
      const el = media(p);
      // play() does not settle while there is nothing to play; do not wait for it.
      const started = await Promise.race([el.play().then(() => true, () => false), new Promise(done => setTimeout(() => done(false), 1200))]);
      return { ...mediaState(el), started };
    },
    'act.play': p => tools['media.play'](p),
    'media.pause': p => { const el = media(p); el.pause(); return mediaState(el); },
    'act.pause': p => tools['media.pause'](p),
    'media.stop': p => { const el = media(p); el.pause(); el.currentTime = 0; return mediaState(el); },
    'media.seek': p => { const el = media(p); el.currentTime = need(p, 'time'); return mediaState(el); },
    'act.seek': p => tools['media.seek'](p),
    'media.volume': p => { const el = media(p); el.volume = Math.min(1, Math.max(0, need(p, 'volume'))); return mediaState(el); },
    'media.mute': p => { const el = media(p); el.muted = p.muted !== false; return mediaState(el); },
    'media.speed': p => { const el = media(p); el.playbackRate = Math.min(16, Math.max(0.1, need(p, 'speed'))); return mediaState(el); },
    'media.tracks': p => { const el = media(p); return { text: [...el.textTracks].map(t => ({ kind: t.kind, label: t.label, language: t.language, mode: t.mode })) }; },

    // ---- forms ----------------------------------------------------------------
    'form.required_fields': () => found(fields().filter(el => el.required || el.getAttribute('aria-required') === 'true')),
    'form.invalid_fields': () => { const bad = fields().filter(el => el.checkValidity && !el.checkValidity()); return { ...found(bad), messages: bad.slice(0, 20).map(el => ({ line: describe(el), message: el.validationMessage })) }; },
    'form.validate': p => { const form = p.ref !== undefined || p.selector ? target(p) : document.querySelector('form'); if (!form || !form.checkValidity) return fail('stale', 'no form on this page'); return { valid: form.checkValidity(), invalid: [...form.elements].filter(el => el.checkValidity && !el.checkValidity()).map(el => ({ line: describe(el), message: el.validationMessage })) }; },
    'form.reset': p => { const form = p.ref !== undefined || p.selector ? target(p) : document.querySelector('form'); if (!form || !form.reset) return fail('stale', 'no form on this page'); form.reset(); return { reset: true }; },
    'act.expand': p => { const el = target(p); if (el.tagName === 'DETAILS') el.open = true; else if (el.getAttribute('aria-expanded') === 'false') safeClick(el); return { expanded: el.open === true || el.getAttribute('aria-expanded') === 'true' }; },
    'act.collapse': p => { const el = target(p); if (el.tagName === 'DETAILS') el.open = false; else if (el.getAttribute('aria-expanded') === 'true') safeClick(el); return { expanded: el.open === true || el.getAttribute('aria-expanded') === 'true' }; },

    // ---- extraction -------------------------------------------------------------
    'extract.json_ld': () => ({ items: all('script[type="application/ld+json"]').map(s => { try { return JSON.parse(s.textContent); } catch { return null; } }).filter(Boolean) }),
    'extract.schema': () => ({ items: all('[itemscope]').slice(0, 30).map(scope => ({ type: scope.getAttribute('itemtype'), properties: Object.fromEntries(all('[itemprop]', scope).slice(0, 40).map(prop => [prop.getAttribute('itemprop'), prop.content || prop.href || text(prop)])) })) }),
    'extract.article': p => { const root = document.querySelector('article,main,[role=main]') || document.body; return { title: (document.querySelector('h1') || {}).innerText || document.title, text: clean(root.innerText, p.max || 8000), words: root.innerText.split(/\s+/).filter(Boolean).length }; },
    'extract.readability': p => tools['extract.article'](p),
    'extract.citations': p => ({ citations: shown('a[href^="http"]').filter(a => text(a)).slice(0, p.max || 50).map(a => ({ text: text(a), url: a.href })) }),
    'extract.comments': () => ({ comments: shown('[class*=comment i],[id*=comment i]').filter(el => !el.querySelector('[class*=comment i]')).slice(0, 50).map(text).filter(Boolean) }),

    // ---- accessibility ------------------------------------------------------------
    'a11y.landmarks': () => ({ landmarks: shown('main,nav,header,footer,aside,form[aria-label],section[aria-label],[role=main],[role=navigation],[role=banner],[role=contentinfo],[role=complementary],[role=search],[role=region]').map(el => ({ role: el.getAttribute('role') || { MAIN: 'main', NAV: 'navigation', HEADER: 'banner', FOOTER: 'contentinfo', ASIDE: 'complementary', FORM: 'form', SECTION: 'region' }[el.tagName], label: el.getAttribute('aria-label') || '' })) }),
    'a11y.labels': () => { const unlabeled = fields().filter(el => !labelOf(el) && !el.placeholder && !el.title); return { fields: fields().length, unlabeled: found(unlabeled).matches }; },
    'a11y.live_regions': () => ({ regions: all('[aria-live],[role=status],[role=alert],[role=log]').map(el => ({ politeness: el.getAttribute('aria-live') || el.getAttribute('role'), text: text(el) })) }),
    'a11y.focus_order': p => found(shown('a[href],button,input:not([type=hidden]),select,textarea,[tabindex]').filter(el => el.tabIndex >= 0 && !el.disabled).sort((a, b) => (a.tabIndex || 1e9) - (b.tabIndex || 1e9)), p.max || 40),
    'a11y.role': p => ({ role: describe(target(p)).split(' ')[0] }),
    'a11y.name': p => ({ name: (describe(target(p)).match(/"(.*?)"/) || [null, ''])[1] }),
    'a11y.description': p => { const el = target(p); const ids = (el.getAttribute('aria-describedby') || '').split(/\s+/).filter(Boolean); return { description: clean(ids.map(id => (document.getElementById(id) || {}).innerText || '').join(' ') || el.title || '', 300) }; },
    'a11y.value': p => tools['runtime.get_value'](p),
    'a11y.state': p => { const el = target(p); return { disabled: Boolean(el.disabled) || el.getAttribute('aria-disabled') === 'true', checked: el.checked ?? (el.getAttribute('aria-checked') === 'true' ? true : null), expanded: el.getAttribute('aria-expanded'), selected: el.selected ?? el.getAttribute('aria-selected'), required: Boolean(el.required), invalid: el.checkValidity ? !el.checkValidity() : false, focused: document.activeElement === el }; },
    'a11y.bounds': p => rect(target(p)),
    'a11y.contrast_check': p => { const el = target(p); const fg = luminance(getComputedStyle(el).color); const bg = luminance(background(el)); const ratio = (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05); return { ratio: Math.round(ratio * 100) / 100, passesAA: ratio >= 4.5, passesAAA: ratio >= 7 }; },

    // ---- form controls without a keyboard path ------------------------------
    'form.slider': p => { const el = target(p); const n = Number(need(p, 'value')); const min = Number(el.min || 0); const max = Number(el.max || 100); return setValue(el, ['range'], Math.min(max, Math.max(min, n))); },
    'form.color': p => setValue(target(p), ['color'], String(need(p, 'value')).toLowerCase()),
    'form.datetime': p => setValue(target(p), ['date', 'time', 'datetime-local', 'month', 'week'], need(p, 'value')),
    'edit.contenteditable': p => {
      const el = target(p);
      if (!el.isContentEditable) return fail('params', `${describe(el)} is not editable`);
      el.focus();
      document.getSelection().selectAllChildren(el);
      document.execCommand('insertText', false, String(need(p, 'text')));
      return { text: clean(el.innerText, 2000) };
    },

    // ---- structure ---------------------------------------------------------
    'observe.dom_subtree': p => { const html = target(p).outerHTML; const max = Math.min(Number(p.max) || 20000, 200000); return { html: html.slice(0, max), chars: html.length, truncated: html.length > max }; },
    'observe.iframe_content': p => ({ frames: all('iframe,frame').slice(0, 20).map((frame, index) => {
      let doc = null;
      try { doc = frame.contentDocument; } catch (_) {}
      return { index, name: frame.name || '', src: frame.src || (frame.srcdoc ? 'srcdoc' : ''), sameOrigin: Boolean(doc), text: doc && doc.body ? clean(doc.body.innerText, Number(p.max) || 2000) : null };
    }) }),
    'frame.children': () => ({ frames: all('iframe,frame').map((frame, index) => { let same = false; try { same = Boolean(frame.contentDocument); } catch (_) {} return { index, name: frame.name || '', src: frame.src || (frame.srcdoc ? 'srcdoc' : ''), sameOrigin: same, ref: refOf(frame) }; }) }),
    'frame.parent': () => ({ top: window === window.top }),
    'tab.opener': () => ({ hasOpener: Boolean(window.opener) }),
    'state.ref_validate': p => { const hit = lookup(need(p, 'ref')); return hit.error ? { valid: false, reason: hit.detail || hit.error } : { valid: visible(hit.el), line: describe(hit.el) }; },
    'state.ref_resolve': p => { const hit = lookup(need(p, 'ref')); if (hit.error) throw hit; return { line: describe(hit.el), box: rect(hit.el), visible: visible(hit.el) }; },

    // ---- timing and storage, as the page sees them ---------------------------
    'network.timing': p => ({ entries: performance.getEntriesByType('resource').slice(-(Number(p.limit) || 50)).map(e => ({ url: e.name, type: e.initiatorType, ms: Math.round(e.duration), bytes: e.transferSize })) }),
    'navigate.redirect_chain': () => { const nav = performance.getEntriesByType('navigation')[0]; return { url: location.href, redirects: nav ? nav.redirectCount : 0 }; },
    'network.redirects': () => { const nav = performance.getEntriesByType('navigation')[0]; return { url: location.href, redirects: nav ? nav.redirectCount : 0 }; },
    'debug.performance_timeline': () => { const nav = performance.getEntriesByType('navigation')[0]; const paint = Object.fromEntries(performance.getEntriesByType('paint').map(e => [e.name, Math.round(e.startTime)])); return { domContentLoaded: nav ? Math.round(nav.domContentLoadedEventEnd) : null, load: nav ? Math.round(nav.loadEventEnd) : null, paint, resources: performance.getEntriesByType('resource').length }; },
    'debug.storage': () => ({ localStorage: localStorage.length, sessionStorage: sessionStorage.length, cookies: document.cookie ? document.cookie.split(';').length : 0 }),
    'debug.permissions': async () => { const names = ['geolocation', 'notifications', 'camera', 'microphone', 'clipboard-read', 'clipboard-write']; const out = {}; for (const name of names) { try { out[name] = (await navigator.permissions.query({ name })).state; } catch (_) { out[name] = 'unknown'; } } return out; },
    'storage.permissions': async () => { const names = ['geolocation', 'notifications', 'camera', 'microphone']; const out = {}; for (const name of names) { try { out[name] = (await navigator.permissions.query({ name })).state; } catch (_) { out[name] = 'unknown'; } } return out; },

    // ---- waiting for one thing ----------------------------------------------
    'wait.role': p => poll(p, `a ${need(p, 'role')}`, () => { const hit = tools['locate.role'](p); return hit.count ? hit : null; }),
    'wait.enabled': p => poll(p, 'the element to be enabled', () => { const el = target(p); return !el.disabled && el.getAttribute('aria-disabled') !== 'true' ? { enabled: true, line: describe(el) } : null; }),
    'wait.media': p => poll(p, 'the media to be ready', () => { const el = media(p); return el.readyState >= 3 ? mediaState(el) : null; }),

    // ---- the agent's pointer, devices, permissions ---------------------------
    'pointer.position': () => { const at = globalThis.__ag.pointerAt && globalThis.__ag.pointerAt(); return at ? { x: Math.round(at.x), y: Math.round(at.y) } : { x: null, y: null, note: 'the agent has not used its pointer on this page' }; },
    'observe.cursor': () => tools['pointer.position'](),
    'browser_ui.view_source': p => tools['observe.dom_subtree']({ ...p, selector: 'html', ref: undefined }),
    'browser_ui.reader_mode': p => {
      const article = tools['extract.article'](p);
      if (p.view !== true) return article;
      // Reader view: the article alone, large and calm, over the page. A
      // second call (or Esc, or the button) puts the page back.
      const old = document.getElementById('agentgo-reader');
      if (old) { old.remove(); return { ...article, view: 'closed' }; }
      const host = document.createElement('div');
      host.id = 'agentgo-reader';
      host.style.cssText = 'position:fixed;inset:0;z-index:2147483646;overflow:auto;';
      const root = host.attachShadow({ mode: 'closed' });
      const style = document.createElement('style');
      style.textContent = ':host{all:initial}.page{min-height:100%;box-sizing:border-box;background:#f6f3ec;color:#222;font:20px/1.7 Georgia,"Times New Roman",serif;padding:56px 24px 96px}@media(prefers-color-scheme:dark){.page{background:#16171a;color:#d8d8d4}}article{max-width:42em;margin:0 auto}h1{font:700 2em/1.25 system-ui,sans-serif;margin:0 0 .6em}p{margin:0 0 1.1em;white-space:pre-wrap}button{position:fixed;top:14px;right:18px;font:600 13px system-ui,sans-serif;padding:7px 14px;border-radius:8px;border:1px solid #8884;background:#fff8;color:inherit;cursor:pointer}';
      const page = document.createElement('div');
      page.className = 'page';
      const body = document.createElement('article');
      const title = document.createElement('h1');
      title.textContent = article.title || document.title;
      body.append(title);
      for (const block of String(article.text || '').split(/\n{1,}/)) {
        if (!block.trim()) continue;
        const paragraph = document.createElement('p');
        paragraph.textContent = block.trim();
        body.append(paragraph);
      }
      const close = document.createElement('button');
      close.textContent = 'Close reader (Esc)';
      const end = () => { host.remove(); window.removeEventListener('keydown', onKey, true); };
      const onKey = event => { if (event.key === 'Escape') end(); };
      close.addEventListener('click', end);
      window.addEventListener('keydown', onKey, true);
      page.append(body, close);
      root.append(style, page);
      document.documentElement.append(host);
      return { ...article, view: 'open' };
    },
    'device.camera_list': () => devices('videoinput'),
    'device.microphone_list': () => devices('audioinput'),
    'device.output_list': () => devices('audiooutput'),
    'site_info.permissions': () => permissions(['geolocation', 'notifications', 'camera', 'microphone']),
    'a11y.focus': p => { const el = target(p); el.focus(); return { focused: document.activeElement === el, line: describe(el) }; },

    // ---- site data and requests from the page --------------------------------
    'storage.indexeddb_clear': async () => { const names = (await indexedDB.databases()).map(db => db.name).filter(Boolean); await Promise.all(names.map(name => new Promise(done => { const r = indexedDB.deleteDatabase(name); r.onsuccess = r.onerror = r.onblocked = () => done(); }))); return { deleted: names }; },
    'network.fetch': async p => {
      const url = new URL(need(p, 'url'), location.href);
      if (!/^https?:$/.test(url.protocol)) return fail('params', 'only http and https addresses');
      // GET only: anything that changes a site goes through the gated actions.
      const response = await fetch(url, { method: 'GET', credentials: 'same-origin' });
      const body = await response.text();
      const max = Math.min(Number(p.max) || 20000, 200000);
      return { status: response.status, url: response.url, contentType: response.headers.get('content-type'), text: body.slice(0, max), truncated: body.length > max };
    },

    // Helper for form.submit (not a register name): the form's submit button.
    'form.submit_target': p => {
      const at = p.ref !== undefined || p.selector ? target(p) : document.querySelector('form');
      const form = at && (at.tagName === 'FORM' ? at : at.form || at.closest('form'));
      if (!form) return fail('stale', 'no form here');
      const button = [...form.querySelectorAll('button,input[type=submit],input[type=image]'), ...(form.id ? all(`[form="${CSS.escape(form.id)}"]`) : [])]
        .find(el => visible(el) && !el.disabled && (el.tagName !== 'BUTTON' || !['button', 'reset'].includes(el.type)));
      if (!button) return fail('stale', 'this form has no submit button; press Enter in one of its fields with act.press');
      return { ref: refOf(button), line: describe(button) };
    },

    // ---- collections: shape lists the agent already has ---------------------
    'collect.next_page': () => { const link = shown('a[rel~=next],link[rel~=next]')[0] || shown('a,button').find(el => /^(next|next page|›|»|older)$/i.test(text(el))); return link ? { found: true, ref: refOf(link), line: describe(link), url: link.href || null } : { found: false }; },
    'collect.deduplicate': p => { const seen = new Set(); const out = items(p).filter(item => { const k = JSON.stringify(field(item, p.key)); if (seen.has(k)) return false; seen.add(k); return true; }); return { items: out, removed: items(p).length - out.length }; },
    'collect.sort': p => { const dir = p.order === 'desc' ? -1 : 1; return { items: [...items(p)].sort((a, b) => { const x = field(a, p.key); const y = field(b, p.key); return (x > y ? 1 : x < y ? -1 : 0) * dir; }) }; },
    'collect.filter': p => { const want = String(need(p, 'contains')).toLowerCase(); return { items: items(p).filter(item => JSON.stringify(field(item, p.key) ?? '').toLowerCase().includes(want)) }; },
    'collect.limit': p => ({ items: items(p).slice(Number(p.offset) || 0, (Number(p.offset) || 0) + (Number(need(p, 'count')) || 0)) }),
    'collect.rank': p => { const terms = new Set(words(need(p, 'query'))); const scored = items(p).map(item => ({ item, score: words(JSON.stringify(field(item, p.key))).filter(w => terms.has(w)).length })); return { items: scored.sort((a, b) => b.score - a.score).map(s => ({ ...((typeof s.item === 'object' && s.item) || { value: s.item }), score: s.score })) }; },
    'extract.diff': p => { const before = String(need(p, 'before')).split('\n'); const after = String(need(p, 'after')).split('\n'); const was = new Set(before); const now = new Set(after); return { added: after.filter(l => !was.has(l)), removed: before.filter(l => !now.has(l)) }; },

    // ---- signals for handoff ----------------------------------------------------------
    'handoff.detect_login': () => ({ detected: signals().login === true }),
    'handoff.detect_captcha': () => ({ detected: signals().captcha === true }),
    'handoff.detect_2fa': () => ({ detected: shown('input[autocomplete="one-time-code"],input[name*="otp" i],input[name*="2fa" i],input[name*="verification" i],input[inputmode="numeric"][maxlength="6"]').length > 0 }),
    'handoff.detect_high_impact': () => ({ detected: signals().payment === true }),
  };

  // ---- grounding: which control does a description mean ------------------
  const wordsOf = value => String(value).toLowerCase().split(/[^a-z0-9\u00c0-\uffff]+/).filter(w => w.length > 1);
  // Scores every visible control against a description: share of the
  // description's words found in the control's role, name, label or text.
  const rank = p => {
    const want = wordsOf(need(p, 'query'));
    if (!want.length) throw fail('params', 'params.query has no words');
    const scored = shown('a,button,input:not([type=hidden]),select,textarea,summary,[role],[contenteditable=true]').map(el => {
      const hay = new Set(wordsOf([describe(el), labelOf(el), el.placeholder || '', el.title || '', el.name || '', el.id || ''].join(' ')));
      return { el, score: want.filter(w => hay.has(w)).length / want.length };
    }).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
    const top = scored.slice(0, p.max || 5).map(item => ({ ref: refOf(item.el), line: describe(item.el), score: Math.round(item.score * 100) / 100 }));
    const margin = top.length > 1 ? Math.round((top[0].score - top[1].score) * 100) / 100 : top.length ? top[0].score : 0;
    return { candidates: top, best: top[0] || null, confidence: top.length ? top[0].score : 0, margin, ambiguous: top.length > 1 && margin === 0 };
  };
  const actionsOf = el => {
    const role = describe(el).split(' ')[0].replace(/\(.*/, '');
    const list = /textbox|searchbox|spinbutton/.test(role) || el.isContentEditable ? ['fill', 'type', 'focus'] : /combobox|listbox/.test(role) ? ['select_option', 'focus'] : /checkbox|radio|switch/.test(role) ? ['check', 'uncheck', 'toggle'] : ['click', 'hover'];
    return { role, actions: el.disabled ? [] : list };
  };
  Object.assign(tools, {
    'ground.semantic': rank,
    'ground.hybrid': rank,
    'ground.compare_candidates': rank,
    'ground.confidence': p => { const r = rank(p); return { confidence: r.confidence, margin: r.margin, ambiguous: r.ambiguous, best: r.best }; },
    'handoff.detect_ambiguity': p => { const r = rank(p); return { detected: r.ambiguous || r.candidates.length === 0, candidates: r.candidates }; },
    'state.relevance_filter': p => rank({ ...p, max: p.max || 15 }),
    'context.relevance': p => rank({ ...p, max: p.max || 15 }),
    'state.region_focus': p => { const root = target(p); return found(shown('a,button,input:not([type=hidden]),select,textarea,summary,[role]', root), 60); },
    'state.ref_create': p => found([target(p)]),
    'context.reference': p => found([target(p)]),
    'locate.ax_ref': p => found([target(p)]),
    'state.ref_heal': p => { const hit = lookup(need(p, 'ref')); return hit.error ? fail(hit.error, hit.detail) : { ref: hit.healed || p.ref, healed: Boolean(hit.healed), line: describe(hit.el) }; },
    'a11y.actions': p => actionsOf(target(p)),
    'a11y.keyboard_only': () => { const reachable = shown('a[href],button,input:not([type=hidden]),select,textarea,[tabindex]').filter(el => el.tabIndex >= 0 && !el.disabled); const clickOnly = shown('[onclick],[role=button],[role=link]').filter(el => el.tabIndex < 0 && !/^(A|BUTTON|INPUT|SELECT|TEXTAREA|SUMMARY)$/.test(el.tagName)); return { reachable: reachable.length, unreachable: found(clickOnly).matches }; },
    'selection.extend': p => { const s = getSelection(); if (!s.rangeCount) throw fail('stale', 'nothing is selected; selection.set first'); s.modify('extend', p.direction === 'backward' ? 'backward' : 'forward', ['character', 'word', 'line', 'sentence', 'paragraph'].includes(p.by) ? p.by : 'word'); return { text: String(s) }; },
    'clipboard.read_html': () => { const s = getSelection(); if (!s.rangeCount) return { html: '' }; const box = document.createElement('div'); box.appendChild(s.getRangeAt(0).cloneContents()); return { html: box.innerHTML.slice(0, 50000) }; },
    'media.select_track': p => { const el = media(p); const tracks = [...el.textTracks]; tracks.forEach((track, index) => { track.mode = index === p.track ? 'showing' : 'disabled'; }); return { tracks: tracks.map(track => ({ label: track.label, language: track.language, mode: track.mode })) }; },
    'extract.transcript': p => { const cues = all('video,audio').flatMap(el => [...el.textTracks]).flatMap(track => [...(track.cues || [])]).map(cue => ({ start: cue.startTime, text: cue.text })); const blocks = shown('[class*=transcript i],[id*=transcript i]').map(el => clean(el.innerText, p.max || 8000)); return { cues, text: blocks.join('\n') }; },
    'media.transcript': p => tools['extract.transcript'](p),
    'frame.snapshot': () => ({ frames: all('iframe,frame').map((frame, index) => { let content = null; try { content = clean(frame.contentDocument.body.innerText, 2000); } catch { content = null; } return { index, name: frame.name || '', src: frame.src || '', readable: content !== null, text: content }; }) }),
    'keyboard.layout': () => ({ language: navigator.language, languages: [...navigator.languages] }),
    'evidence.citation': p => tools['extract.citations'](p),
  });

  // ---- exports of the page's own storage, and cut --------------------------
  const readStore = (db, name) => new Promise(done => {
    try {
      const request = db.transaction(name, 'readonly').objectStore(name).getAll(null, 200);
      request.onsuccess = () => done(request.result);
      request.onerror = () => done([]);
    } catch { done([]); }
  });
  const openDb = name => new Promise(done => { const request = indexedDB.open(name); request.onsuccess = () => done(request.result); request.onerror = () => done(null); });
  Object.assign(tools, {
    'storage.indexeddb_export': async p => {
      const names = indexedDB.databases ? (await indexedDB.databases()).map(db => db.name) : [];
      const out = {};
      for (const name of names.filter(n => !p.name || n === p.name).slice(0, 10)) {
        const db = await openDb(name);
        if (!db) continue;
        out[name] = {};
        for (const store of [...db.objectStoreNames].slice(0, 20)) out[name][store] = await readStore(db, store);
        db.close();
      }
      const json = JSON.stringify(out);
      return { databases: out, bytes: json.length, truncated: false };
    },
    'storage.cache_export': async () => {
      const result = {};
      for (const key of globalThis.caches ? await caches.keys() : []) result[key] = (await (await caches.open(key)).keys()).slice(0, 500).map(request => request.url);
      return { caches: result };
    },
    'storage.site_data_export': async () => ({
      origin: location.origin,
      localStorage: Object.fromEntries(Object.keys(localStorage).slice(0, 500).map(key => [key, localStorage.getItem(key)])),
      sessionStorage: Object.fromEntries(Object.keys(sessionStorage).slice(0, 500).map(key => [key, sessionStorage.getItem(key)])),
      cookieNames: document.cookie ? document.cookie.split('; ').map(pair => pair.split('=')[0]) : [],
      indexedDB: indexedDB.databases ? (await indexedDB.databases()).map(db => db.name) : [],
    }),
    // Removes the selected text from the focused field and returns it, so it
    // can be put on the agent's clipboard (clipboard.write) and pasted.
    'edit.cut': p => {
      const el = p.ref !== undefined || p.selector ? target(p) : document.activeElement;
      if (!el || !('value' in el) || typeof el.selectionStart !== 'number') throw fail('blocked', 'select text in a field first (selection.set)');
      if (secret(el)) throw fail('policy', 'text cannot be cut from a password field');
      const cut = el.value.slice(el.selectionStart, el.selectionEnd);
      el.focus();
      el.setRangeText('', el.selectionStart, el.selectionEnd, 'start');
      el.dispatchEvent(new Event('input', { bubbles: true }));
      return { text: cut, value: el.type === 'password' ? null : el.value };
    },
  });

  // The page's media devices, with the ids a page's own picker takes. Labels
  // appear once the site was granted camera or microphone (permission.grant).
  const mediaDevicesOf = async kind => ({ devices: navigator.mediaDevices ? (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === kind).map(d => ({ id: d.deviceId, label: d.label, group: d.groupId })) : [] });
  Object.assign(tools, {
    'device.camera_select': () => mediaDevicesOf('videoinput'),
    'device.microphone_select': () => mediaDevicesOf('audioinput'),
    'device.output_select': async p => {
      const out = await mediaDevicesOf('audiooutput');
      if (p.id !== undefined) {
        // Routes the page's audio and video elements to that output.
        const done = await Promise.all(all('audio,video').filter(el => el.setSinkId).map(el => el.setSinkId(p.id).then(() => true, () => false)));
        return { ...out, routed: done.filter(Boolean).length };
      }
      return out;
    },
  });

  globalThis.__ag.tool = async (name, params) => {
    const run = tools[name];
    if (!run) return fail('unknown', `no page tool named ${name}`);
    try {
      return await run(params || {});
    } catch (thrown) {
      if (thrown && thrown.error) return thrown;
      return fail('failed', String((thrown && thrown.message) || thrown));
    }
  };
  globalThis.__ag.toolNames = () => Object.keys(tools);
})();
