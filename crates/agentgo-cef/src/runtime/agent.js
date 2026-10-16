// AgentGo page agent. Runs in an isolated JavaScript world: page scripts can
// neither read nor replace anything defined here. It turns the live DOM into a
// compact, reference-addressed view and resolves references back to trusted
// input targets. It never performs side effects on its own except scrolling
// and focusing when the native layer asks for a target.
(() => {
  if (globalThis.__ag) return;
  const ids = new WeakMap();
  const els = new Map();
  const prints = new Map();
  let next = 0;

  const TAG_ROLE = { A: 'link', BUTTON: 'button', SELECT: 'combobox', TEXTAREA: 'textbox', SUMMARY: 'button', OPTION: 'option', IMG: 'img' };
  const INPUT_ROLE = { button: 'button', submit: 'button', reset: 'button', image: 'button', checkbox: 'checkbox', radio: 'radio', range: 'slider', search: 'searchbox', file: 'file' };
  const ROLES = new Set(['button', 'link', 'checkbox', 'radio', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'switch', 'combobox', 'textbox', 'searchbox', 'slider', 'spinbutton', 'treeitem', 'listbox']);
  const MONEY = /\b(buy|purchase|pay|place (your )?order|check ?out|subscribe|donate|bid|confirm (order|payment|purchase)|complete (order|purchase)|send money|transfer funds|upgrade (now|plan))\b/i;
  const CARD = 'input[autocomplete^="cc-"],input[name*="card" i],input[name*="cvv" i],input[name*="cvc" i],input[id*="cardnumber" i]';

  const clean = (text, max = 80) => {
    const value = String(text ?? '').replace(/\s+/g, ' ').trim();
    return value.length > max ? value.slice(0, max - 1) + '…' : value;
  };
  const quote = text => JSON.stringify(text);

  const visible = el => {
    if (!el.isConnected) return false;
    if (el.checkVisibility && !el.checkVisibility({ visibilityProperty: true })) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };

  const roleOf = el => {
    const explicit = el.getAttribute('role');
    if (explicit) return explicit.split(' ')[0];
    if (el.tagName === 'INPUT') return INPUT_ROLE[el.type] || 'textbox';
    if (el.tagName === 'A') return el.hasAttribute('href') ? 'link' : 'generic';
    if (el.isContentEditable) return 'textbox';
    if (!TAG_ROLE[el.tagName] && el.getAttribute('draggable') === 'true') return 'draggable';
    return TAG_ROLE[el.tagName] || 'clickable';
  };

  // Text of a label without the text of the controls nested inside it.
  const ownText = node => {
    if (node.nodeType === 3) return node.textContent;
    if (node.nodeType !== 1 || /^(SELECT|TEXTAREA|INPUT|BUTTON|SCRIPT|STYLE)$/.test(node.tagName)) return '';
    return [...node.childNodes].map(ownText).join(' ');
  };

  const labelText = el => {
    const root = el.getRootNode();
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const text = by.split(/\s+/).map(id => root.getElementById?.(id)?.textContent || '').join(' ');
      if (text.trim()) return text;
    }
    if (el.labels?.length) return [...el.labels].map(ownText).join(' ');
    return '';
  };

  const nameOf = el => clean(
    el.getAttribute('aria-label') || labelText(el) ||
    (el.tagName === 'INPUT' && /^(button|submit|reset)$/.test(el.type) ? el.value : '') ||
    el.getAttribute('placeholder') || el.getAttribute('alt') ||
    (/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) ? el.getAttribute('title') || el.name : el.innerText || el.textContent) ||
    el.getAttribute('title') || el.querySelector?.('img[alt]')?.alt || el.querySelector?.('svg title')?.textContent ||
    // A control with no name of its own (a bare checkbox in a list row) is
    // named by the row it sits in, as a person reads it.
    (/^(checkbox|radio)$/.test(el.type || '') || !el.textContent?.trim()
      ? (el.closest('li,tr,[role=row],[role=listitem],label')?.innerText || '').slice(0, 80)
      : '') || ''
  );

  const interactive = el => {
    const tag = el.tagName;
    if (tag === 'INPUT') return el.type !== 'hidden';
    if (tag === 'A' && el.hasAttribute('href')) return true;
    if (tag === 'BUTTON' || tag === 'SELECT' || tag === 'TEXTAREA' || tag === 'SUMMARY') return true;
    if (ROLES.has(el.getAttribute('role'))) return true;
    if (el.isContentEditable && !el.parentElement?.isContentEditable) return true;
    if (el.hasAttribute('onclick') || (el.hasAttribute('tabindex') && el.tabIndex >= 0)) return true;
    if (el.getAttribute('draggable') === 'true') return true;
    // Script-driven controls (a div with a click listener) usually show the
    // pointer cursor. Only the outermost such element counts, not its text.
    if (/^(A|DIV|SPAN|LI|IMG|SVG|I|TD|TR|P|SECTION|ARTICLE|LABEL)$/i.test(tag) && el.firstChild !== null || tag === 'IMG' || tag === 'A') {
      const parent = el.parentElement;
      // `cursor` is inherited, so a pointer parent does not make a child a
      // second target... unless the parent is a menu or list of such items,
      // where every row is its own target and the parent is not.
      const group = node => {
        if (!node || getComputedStyle(node).cursor !== 'pointer') return false;
        let rows = 0;
        for (const child of node.children) {
          if (child.firstChild === null || (child.innerText || '').length > 80) continue;
          if (getComputedStyle(child).cursor === 'pointer' && ++rows >= 2) return true;
        }
        return false;
      };
      if (getComputedStyle(el).cursor === 'pointer'
        && (!(parent && getComputedStyle(parent).cursor === 'pointer') || group(parent))
        && !group(el)
        && !el.closest('a[href],button,summary,label,[role=button],[role=link]')
        // A wrapper around one real control is that control, not a second target.
        && el.querySelectorAll('a[href],button,input:not([type=hidden]),select,textarea').length !== 1) return true;
    }
    return false;
  };

  const refOf = el => {
    let id = ids.get(el);
    // An expired reference (state.ref_expire) is never handed out again.
    if (!id || !els.has(id)) {
      id = ++next;
      ids.set(el, id);
      els.set(id, new WeakRef(el));
    }
    return id;
  };

  // Depth-first walk through light DOM, open shadow roots and same-origin
  // frames. `visit` returns false to skip a subtree.
  const walk = (root, visit, offset = [0, 0]) => {
    for (const el of root.children || []) {
      if (/^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|HEAD)$/.test(el.tagName)) continue;
      if (visit(el, offset) === false) continue;
      if (el.shadowRoot) walk(el.shadowRoot, visit, offset);
      if (el.tagName === 'IFRAME') {
        let doc = null;
        try { doc = el.contentDocument; } catch (_) {}
        if (doc?.body) {
          const rect = el.getBoundingClientRect();
          walk(doc.body, visit, [offset[0] + rect.left + el.clientLeft, offset[1] + rect.top + el.clientTop]);
        }
        continue;
      }
      walk(el, visit, offset);
    }
  };

  const describe = el => {
    const role = roleOf(el);
    let line = `${role === 'textbox' && el.type && el.type !== 'text' && el.type !== 'textarea' ? `textbox(${el.type})` : role} ${quote(nameOf(el))}`;
    if (/^(INPUT|TEXTAREA)$/.test(el.tagName) && !/^(checkbox|radio|button|submit|reset|file|password|image)$/.test(el.type)) {
      if (el.value) line += ` value=${quote(clean(el.value, 60))}`;
    } else if (el.tagName === 'SELECT') {
      line += ` value=${quote(clean(el.selectedOptions[0]?.textContent, 40))}`;
    } else if (el.isContentEditable) {
      const text = clean(el.innerText, 60);
      if (text) line = `textbox ${quote(clean(el.getAttribute('aria-label') || el.getAttribute('data-placeholder') || 'editor'))} value=${quote(text)}`;
    }
    if (el.checked || el.getAttribute('aria-checked') === 'true') line += ' checked';
    if (el.getAttribute('aria-selected') === 'true') line += ' selected';
    if (el.getAttribute('aria-expanded')) line += el.getAttribute('aria-expanded') === 'true' ? ' expanded' : ' collapsed';
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') line += ' disabled';
    if (el.required) line += ' required';
    return line;
  };

  const collect = (opts = {}) => {
    const items = [];
    const height = innerHeight, width = innerWidth;
    walk(document.documentElement, (el, offset) => {
      if (el.getAttribute('aria-hidden') === 'true' && !el.contains(document.activeElement)) return false;
      const heading = /^H[1-4]$/.test(el.tagName);
      const isInteractive = interactive(el);
      if (!heading && !isInteractive && !opts.text) return true;
      if (!visible(el)) return isInteractive || heading ? false : true;
      if (opts.viewport) {
        const rect = el.getBoundingClientRect();
        if (rect.bottom + offset[1] < 0 || rect.top + offset[1] > height || rect.right + offset[0] < 0 || rect.left + offset[0] > width) return true;
      }
      if (heading) {
        const text = clean(el.innerText, 100);
        if (text) items.push({ line: `${'#'.repeat(+el.tagName[1])} ${text}` });
        return true;
      }
      if (isInteractive) {
        const line = describe(el);
        const id = refOf(el);
        prints.set(id, line.replace(/ value=.*$/, ''));
        items.push({ id, line, el });
        // Controls inside a link/button are part of that one target.
        return !/^(A|BUTTON|SUMMARY|SELECT)$/.test(el.tagName) && !/^(button|link|tab|menuitem|option)$/.test(el.getAttribute('role') || '');
      }
      if (opts.text && /^(P|LI|TD|TH|DT|DD|BLOCKQUOTE|PRE|FIGCAPTION|LABEL|SPAN|DIV)$/.test(el.tagName)) {
        const own = clean([...el.childNodes].filter(node => node.nodeType === 3).map(node => node.textContent).join(' '), 200);
        if (own.length > 2) items.push({ line: `· ${own}` });
      }
      return true;
    });
    return items;
  };

  // Cloudflare's full-page check ("Just a moment..."). Recognised by its
  // structure rather than its wording, so translated titles still match, and
  // never by the analytics script (/challenge-platform/scripts/jsd/) that
  // many ordinary Cloudflare sites load. Detection only: AgentGo waits for
  // the page's own check to finish or hands the tab to the human.
  const interstitial = () => {
    if (document.querySelector('#challenge-form,#challenge-running,#challenge-stage,#cf-challenge-running,#cf-please-wait,.cf-browser-verification,[id^="cf-chl-widget"]')) return true;
    if (document.querySelector('script[src*="/cdn-cgi/challenge-platform/"][src*="/orchestrate/"],script[src*="/cdn-cgi/challenge-platform/"][src*="/chl_page/"]')) return true;
    if (document.body.children.length < 12) {
      for (const script of document.scripts) {
        if (!script.src && script.text.length < 30000 && script.text.includes('_cf_chl_opt')) return true;
      }
    }
    return false;
  };

  // Text an agent could be steered by, with whether a person could see it.
  // Scored in Rust (injection.rs); this only collects.
  const injectionChunks = () => {
    const out = [];
    let budget = 60000;
    const push = (t, how) => {
      t = String(t || '').replace(/\s+/g, ' ').trim();
      if (t.length < 8 || budget <= 0) return;
      t = t.slice(0, 1500);
      budget -= t.length;
      out.push({ t, how });
    };
    const hiddenHow = el => {
      for (let n = el, depth = 0; n && n.nodeType === 1 && depth < 6; n = n.parentElement, depth++) {
        const s = getComputedStyle(n);
        if (s.display === 'none') return 'display-none';
        if (s.visibility === 'hidden' || s.visibility === 'collapse') return 'visibility-hidden';
        if (parseFloat(s.opacity) < 0.05) return 'transparent';
        if (n.getAttribute('aria-hidden') === 'true' && !n.matches('svg,[role=presentation]')) return 'aria-hidden';
        if (parseFloat(s.fontSize) < 2) return 'tiny-font';
        const m = /rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)/.exec(s.color);
        if (m && m[4] !== undefined && parseFloat(m[4]) < 0.05) return 'transparent-text';
        if (s.clip === 'rect(0px, 0px, 0px, 0px)' || s.clipPath === 'inset(100%)') return 'clipped';
        if (s.textIndent && parseFloat(s.textIndent) < -500) return 'offscreen';
      }
      const r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) return 'zero-size';
      if (r.right < -50 || r.bottom < -50 || r.left > innerWidth + 3000 || r.top > document.documentElement.scrollHeight + 3000) return 'offscreen';
      return '';
    };
    const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_COMMENT);
    for (let node, count = 0; (node = walker.nextNode()) && count < 3000; count++) {
      const parent = node.parentElement;
      if (!parent || /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE)$/.test(parent.tagName)) continue;
      if (node.nodeType === 8) { push(node.nodeValue, 'comment'); continue; }
      push(node.nodeValue, hiddenHow(parent) || '');
    }
    for (const el of document.querySelectorAll('[alt],[title],[aria-label],[data-prompt],[data-instructions]')) {
      for (const name of ['alt', 'title', 'aria-label', 'data-prompt', 'data-instructions']) {
        const v = el.getAttribute(name);
        if (v && v.length > 30) push(v, 'attribute');
      }
    }
    return { url: location.href, chunks: out };
  };

  // Translates a selection. The text goes to Google's translation endpoint from
  // the isolated world, which the page's own security policy does not bind.
  const translateText = async ({ text, to }) => {
    const query = '&sl=auto&dt=t&dj=1&tl=' + encodeURIComponent(to || 'en') + '&q=' + encodeURIComponent(String(text || '').slice(0, 4000));
    const hosts = ['https://translate.googleapis.com/translate_a/single?client=gtx', 'https://clients5.google.com/translate_a/t?client=dict-chrome-ex'];
    let last = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      const url = hosts[attempt % hosts.length] + query;
      try {
        const response = await fetch(url, { credentials: 'omit', referrerPolicy: 'no-referrer' });
        if (!response.ok) throw new Error('translation service answered ' + response.status);
        const data = await response.json();
        if (data.sentences) return { text: data.sentences.map(s => s.trans || '').join(''), from: data.src || '' };
        const rows = Array.isArray(data) ? data : [];
        const first = rows[0];
        return { text: (Array.isArray(first) ? first[0] : first) || '', from: (Array.isArray(first) ? first[1] : '') || '' };
      } catch (error) {
        last = error;
        await new Promise(resolve => setTimeout(resolve, 350 * (attempt + 1)));
      }
    }
    throw new Error('Could not reach the translation service. Check your connection and try again. (' + (last && last.message || 'network error') + ')');
  };

  const translatePage = async ({ to, force, restore } = {}) => {
    const saved = globalThis.__agTranslated;
    if (saved) {
      for (const [node, original] of saved) node.nodeValue = original;
      globalThis.__agTranslated = null;
      document.documentElement.removeAttribute('data-agentgo-translated');
      if (!force) return { restored: true };
    }
    if (restore) return { restored: true };
    const pageLang = (document.documentElement.lang || '').toLowerCase().split('-')[0];
    if (pageLang && pageLang === String(to || 'en').toLowerCase().split('-')[0]) return { skipped: 'same-language', pageLang };
    const skip = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'CODE', 'PRE', 'TEXTAREA', 'SVG']);
    const nodes = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n && nodes.length < 900; n = walker.nextNode()) {
      const p = n.parentElement;
      if (!p || skip.has(p.tagName.toUpperCase()) || p.isContentEditable || !/\p{L}/u.test(n.nodeValue)) continue;
      const r = p.getBoundingClientRect();
      if (r.width && r.height) nodes.push(n);
    }
    const batches = [];
    for (let i = 0, cur = [], size = 0; i <= nodes.length; i++) {
      const n = nodes[i];
      if (!n || cur.length >= 40 || size + n.nodeValue.length > 3000) {
        if (cur.length) batches.push(cur);
        cur = []; size = 0;
      }
      if (n) { cur.push(n); size += n.nodeValue.length; }
    }
    const originals = new Map();
    let next = 0;
    const worker = async () => {
      while (next < batches.length) {
        const batch = batches[next++];
        const body = batch.map(n => 'q=' + encodeURIComponent(n.nodeValue.trim())).join('&');
        try {
          const response = await fetch('https://translate.googleapis.com/translate_a/t?client=gtx&sl=auto&format=text&tl=' + encodeURIComponent(to || 'en'), {
            method: 'POST', credentials: 'omit', referrerPolicy: 'no-referrer',
            headers: { 'content-type': 'application/x-www-form-urlencoded' }, body,
          });
          if (!response.ok) continue;
          const out = await response.json();
          batch.forEach((n, i) => {
            const t = Array.isArray(out[i]) ? out[i][0] : out[i];
            if (typeof t === 'string' && t) {
              originals.set(n, n.nodeValue);
              n.nodeValue = n.nodeValue.match(/^\s*/)[0] + t + n.nodeValue.match(/\s*$/)[0];
            }
          });
        } catch (_) { /* keep original text for this batch */ }
      }
    };
    await Promise.all([worker(), worker(), worker()]);
    globalThis.__agTranslated = originals;
    document.documentElement.setAttribute('data-agentgo-translated', to || 'en');
    return { translated: originals.size, restored: false };
  };

  const signals = () => {
    const out = {};
    const body = document.body;
    if (!body) return out;
    if (interstitial()) {
      out.captcha = true;
      out.challenge = 'interstitial';
    }
    if (document.querySelector('iframe[src*="recaptcha"],iframe[src*="hcaptcha"],iframe[src*="challenges.cloudflare.com"],.g-recaptcha,.h-captcha,#cf-challenge-running,[data-sitekey]') ||
        /verify (that )?you('| a)re (a )?human|i'?m not a robot|unusual traffic from your|complete the security check|are you a robot/i.test(document.title + ' ' + (body.innerText || '').slice(0, 1500))) out.captcha = true;
    if ([...document.querySelectorAll('input[type=password]')].some(visible)) out.login = true;
    if ([...document.querySelectorAll(CARD)].some(visible)) out.payment = true;
    return out;
  };

  const scrollState = () => {
    const root = document.scrollingElement || document.documentElement;
    return { y: Math.round(root.scrollTop), h: root.scrollHeight, vh: innerHeight };
  };

  const snapshot = (opts = {}) => {
    const max = opts.max || 150;
    const start = opts.offset || 0;
    let items = collect(opts);
    if (opts.filter) {
      const needle = String(opts.filter).toLowerCase();
      items = items.filter(item => item.line.toLowerCase().includes(needle));
    }
    const page = items.slice(start, start + max);
    return {
      url: location.href,
      title: clean(document.title, 120),
      lines: page.map(item => (item.id ? `[${item.id}] ${item.line}` : item.line)),
      total: items.length,
      next: start + max < items.length ? start + max : null,
      max: next,
      signals: signals(),
      scroll: scrollState(),
    };
  };

  const deepHit = (x, y) => {
    let doc = document, ox = 0, oy = 0, hit = null;
    for (let depth = 0; depth < 12; depth += 1) {
      hit = doc.elementFromPoint(x - ox, y - oy);
      while (hit?.shadowRoot) {
        const inner = hit.shadowRoot.elementFromPoint(x - ox, y - oy);
        if (!inner || inner === hit) break;
        hit = inner;
      }
      if (hit?.tagName !== 'IFRAME') break;
      let inner = null;
      try { inner = hit.contentDocument; } catch (_) {}
      if (!inner) break;
      const rect = hit.getBoundingClientRect();
      ox += rect.left + hit.clientLeft;
      oy += rect.top + hit.clientTop;
      doc = inner;
    }
    return hit;
  };

  const frameOffset = el => {
    let x = 0, y = 0, win = el.ownerDocument.defaultView;
    while (win && win !== window && win.frameElement) {
      const rect = win.frameElement.getBoundingClientRect();
      x += rect.left + win.frameElement.clientLeft;
      y += rect.top + win.frameElement.clientTop;
      win = win.parent;
    }
    return [x, y];
  };

  const lookup = id => {
    const el = els.get(id)?.deref();
    if (el?.isConnected) return { el };
    const print = prints.get(id);
    if (!print) return { error: 'stale', detail: `reference ${id} does not exist in this document` };
    const matches = collect().filter(item => item.id && item.line.replace(/ value=.*$/, '') === print);
    if (matches.length === 1) return { el: matches[0].el, healed: matches[0].id };
    return { error: matches.length ? 'ambiguous' : 'stale', detail: `${matches.length} elements match ${print}` };
  };

  // Classifies what submitting an element's form would do: 'money' when it
  // carries payment card fields, 'submit' for a consequential form, null for
  // search-like forms (a GET with one or two plain fields) and no form.
  const formRisk = el => {
    const form = el?.form || el?.closest?.('form');
    if (!form) return null;
    if (form.querySelector(CARD)) return 'money';
    // Signing in is not a final submission: agents may use the person's accounts
    // unless one is blocked, and the Account Center is where that is decided.
    if (form.querySelector('input[type=password]') && form.querySelectorAll('input:not([type=hidden]),textarea,select').length <= 4) return null;
    const fields = [...form.elements].filter(f => f.tagName !== 'FIELDSET' && !/^(hidden|submit|button|reset|image)$/.test(f.type));
    const plain = !fields.some(f => /^(password|file)$/.test(f.type) || f.tagName === 'TEXTAREA');
    const search = form.getAttribute('role') === 'search' || ((form.getAttribute('method') || 'get').toLowerCase() === 'get' && fields.length <= 2 && plain);
    return search ? null : 'submit';
  };
  const isSubmitter = el => !!el.form && /^(submit|image)$/.test(el.type || '');

  const moneyRisk = el => {
    const label = `${nameOf(el)} ${el.value || ''}`;
    if (MONEY.test(label)) return clean(label, 60);
    if (isSubmitter(el) && formRisk(el) === 'money') return 'submits a form with payment card fields';
    return null;
  };

  // Risk of pressing Enter right now: implicit submission of the focused
  // field's form.
  const enterRisk = () => {
    let el = document.activeElement;
    while (el?.shadowRoot?.activeElement) el = el.shadowRoot.activeElement;
    while (el?.tagName === 'IFRAME') {
      let inner = null;
      try { inner = el.contentDocument?.activeElement; } catch (_) {}
      if (!inner) break;
      el = inner;
    }
    if (!el || el.tagName !== 'INPUT' || !el.form) return { risk: null };
    return { risk: formRisk(el), line: describe(el) };
  };

  // Resolves a reference to a viewport point that a trusted click will hit.
  const target = (id, opts = {}) => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    if (!visible(el)) return { error: 'hidden', detail: `${describe(el)} is not visible` };
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const rect = el.getBoundingClientRect();
    const [fx, fy] = frameOffset(el);
    const x = Math.round(fx + Math.min(Math.max(rect.left + rect.width / 2, 1), innerWidth - 1));
    const y = Math.round(fy + Math.min(Math.max(rect.top + rect.height / 2, 1), innerHeight - 1));
    const hit = deepHit(x, y);
    const ok = hit && (hit === el || el.contains(hit) || hit.contains(el) || [...(el.labels || [])].some(label => label.contains(hit)));
    const out = {
      x, y, line: describe(el), tag: el.tagName, type: el.type || null,
      submits: isSubmitter(el) && formRisk(el) === 'submit',
      disabled: !!(el.disabled || el.getAttribute('aria-disabled') === 'true'),
      editable: /^(INPUT|TEXTAREA)$/.test(el.tagName) || el.isContentEditable,
      money: moneyRisk(el), healed: found.healed || null,
      href: el.tagName === 'A' ? el.href : null,
    };
    if (!ok && !opts.force) out.covered = hit ? describe(hit) : 'nothing at that point';
    return out;
  };

  // The field that has the keyboard focus, if it takes text.
  const focusedEditable = () => {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    const editable = Boolean(el) && el !== document.body && (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable);
    return { editable, line: editable ? describe(el) : '' };
  };

  const focusForInput = id => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    if (el.tagName === 'SELECT') return { kind: 'select' };
    if (!(/^(INPUT|TEXTAREA)$/.test(el.tagName) || el.isContentEditable)) return { error: 'not-editable', detail: `${describe(el)} does not accept text` };
    if (el.disabled || el.readOnly) return { error: 'disabled', detail: `${describe(el)} is read-only or disabled` };
    el.focus();
    if (el.select) el.select();
    else {
      const range = document.createRange();
      range.selectNodeContents(el);
      const selection = el.ownerDocument.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
    return { kind: 'text', empty: !(el.value ?? el.innerText), money: [...(el.closest('form')?.querySelectorAll(CARD) || [])].includes(el) ? 'payment card field' : null };
  };

  const choose = (id, wanted) => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    if (el.tagName !== 'SELECT') return { error: 'not-select', detail: `${describe(el)} is not a native select` };
    const want = String(wanted).trim().toLowerCase();
    const option = [...el.options].find(o => o.value.toLowerCase() === want || o.textContent.trim().toLowerCase() === want) ||
      [...el.options].find(o => o.textContent.trim().toLowerCase().includes(want));
    if (!option) return { error: 'no-option', detail: `options: ${[...el.options].slice(0, 30).map(o => clean(o.textContent, 30)).join(' | ')}` };
    el.value = option.value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { value: clean(option.textContent, 60) };
  };

  const valueOf = id => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    return { value: el.type === 'password' ? '•'.repeat(String(el.value).length) : el.value ?? el.innerText ?? '', line: describe(el) };
  };

  // Readable page text as light markdown, paged by character offset.
  const text = (opts = {}) => {
    const root = (opts.selector && document.querySelector(opts.selector)) || document.querySelector('main,[role=main],article') || document.body;
    const out = [];
    const block = /^(P|DIV|SECTION|ARTICLE|LI|TR|H[1-6]|BLOCKQUOTE|PRE|UL|OL|TABLE|DL|DT|DD|BR|HR|FIGCAPTION|HEADER|MAIN)$/;
    const emit = (node, pre = false) => {
      if (node.nodeType === 3) {
        // Preformatted text keeps its line breaks.
        const value = pre ? node.textContent : node.textContent.replace(/\s+/g, ' ');
        if (value.trim()) out.push(value);
        return;
      }
      if (node.nodeType !== 1) return;
      const tag = node.tagName;
      // A line break has no box of its own, so it is handled before the visibility test.
      if (tag === 'BR') { out.push('\n'); return; }
      if (/^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|SVG|NAV|FOOTER|ASIDE|IFRAME|BUTTON|SELECT|FORM)$/.test(tag) && !opts.all) return;
      if (node.getAttribute('aria-hidden') === 'true' || !visible(node)) return;
      const row = tag === 'LI' || tag === 'TR';
      if (/^H[1-6]$/.test(tag)) out.push(`\n\n${'#'.repeat(+tag[1])} `);
      else if (tag === 'LI') out.push('\n- ');
      else if (tag === 'TR') out.push('\n');
      else if (tag === 'TD' || tag === 'TH') out.push(' | ');
      else if (block.test(tag)) out.push('\n');
      pre = pre || tag === 'PRE' || tag === 'TEXTAREA';
      if (tag === 'A' && opts.links && node.href && !node.href.startsWith('javascript:')) {
        out.push(`[${clean(node.innerText, 120)}](${node.href})`);
        return;
      }
      for (const child of node.childNodes) emit(child, pre);
      if (node.shadowRoot) for (const child of node.shadowRoot.childNodes) emit(child, pre);
      if (block.test(tag) && !row) out.push('\n');
    };
    emit(root);
    const full = out.join('').replace(/[ \t]+/g, ' ').replace(/ ?\n ?/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
    const start = opts.offset || 0;
    const max = opts.max || 6000;
    return { url: location.href, title: clean(document.title, 120), text: full.slice(start, start + max), total: full.length, next: start + max < full.length ? start + max : null };
  };

  const links = (opts = {}) => {
    const seen = new Set();
    const all = [];
    const needle = opts.filter ? String(opts.filter).toLowerCase() : null;
    for (const a of document.querySelectorAll('a[href]')) {
      if (!visible(a) || a.href.startsWith('javascript:') || seen.has(a.href)) continue;
      const label = clean(a.innerText || a.getAttribute('aria-label') || a.title, 100);
      if (!label || (needle && !(label.toLowerCase().includes(needle) || a.href.toLowerCase().includes(needle)))) continue;
      seen.add(a.href);
      all.push({ ref: refOf(a), text: label, url: a.href });
    }
    const start = opts.offset || 0, max = opts.max || 60;
    return { links: all.slice(start, start + max), total: all.length, next: start + max < all.length ? start + max : null, max: next };
  };

  const tables = (opts = {}) => {
    const found = [...document.querySelectorAll('table')].filter(visible);
    const index = opts.index ?? null;
    const pick = index === null ? found : found.slice(index, index + 1);
    const maxRows = opts.maxRows || 50;
    return {
      count: found.length,
      tables: pick.slice(0, opts.max || 5).map((table, i) => ({
        index: index ?? i,
        caption: clean(table.caption?.innerText || table.getAttribute('aria-label') || '', 80),
        rowCount: table.rows.length,
        rows: [...table.rows].slice(opts.rowOffset || 0, (opts.rowOffset || 0) + maxRows).map(row => [...row.cells].map(cell => clean(cell.innerText, 120))),
      })),
    };
  };

  const forms = () => {
    const groups = new Map();
    for (const el of document.querySelectorAll('input,select,textarea,[contenteditable=""],[contenteditable="true"]')) {
      if (el.type === 'hidden' || !visible(el)) continue;
      const form = el.closest('form');
      if (!groups.has(form)) groups.set(form, []);
      const field = { ref: refOf(el), label: nameOf(el), type: el.tagName === 'SELECT' ? 'select' : el.type || 'text' };
      if (el.required) field.required = true;
      if (el.tagName === 'SELECT') field.options = [...el.options].slice(0, 40).map(o => clean(o.textContent, 40));
      if (el.type === 'checkbox' || el.type === 'radio') field.checked = el.checked;
      else if (el.type !== 'password' && el.type !== 'file') field.value = clean(el.value ?? el.innerText, 80);
      if (el.validationMessage) field.invalid = clean(el.validationMessage, 100);
      groups.get(form).push(field);
    }
    return {
      max: next,
      forms: [...groups].map(([form, fields]) => ({
        name: form ? clean(form.getAttribute('aria-label') || form.name || form.id || form.action, 80) : '(no form element)',
        submit: form ? [...form.querySelectorAll('button:not([type=button]),input[type=submit]')].filter(visible).map(el => ({ ref: refOf(el), label: nameOf(el) })) : [],
        fields,
      })),
    };
  };

  // Resolves once the document has been quiet for `quiet` ms.
  const settle = (quiet = 250, timeout = 4000) => new Promise(resolve => {
    const started = performance.now();
    let last = started;
    const observer = new MutationObserver(() => { last = performance.now(); });
    observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
    const check = () => {
      const now = performance.now();
      const quietFor = now - last;
      if ((quietFor >= quiet && document.readyState === 'complete') || now - started >= timeout) {
        observer.disconnect();
        resolve({ stable: quietFor >= quiet, ms: Math.round(now - started) });
      } else setTimeout(check, 50);
    };
    setTimeout(check, Math.min(quiet, 60));
  });

  const scroll = (opts = {}) => {
    const root = document.scrollingElement || document.documentElement;
    if (opts.to === 'top') root.scrollTo({ top: 0, behavior: 'instant' });
    else if (opts.to === 'bottom') root.scrollTo({ top: root.scrollHeight, behavior: 'instant' });
    else root.scrollBy({ top: opts.dy ?? Math.round(innerHeight * 0.85) * (opts.pages ?? 1), left: opts.dx || 0, behavior: 'instant' });
    return scrollState();
  };

  const waitFor = (opts = {}) => new Promise(resolve => {
    const started = performance.now();
    const timeout = opts.timeout || 8000;
    const needle = opts.text ? String(opts.text).toLowerCase() : null;
    const check = () => {
      let ok = true;
      if (needle) ok = (document.body?.innerText || '').toLowerCase().includes(needle);
      if (ok && opts.selector) ok = [...document.querySelectorAll(opts.selector)].some(visible);
      if (ok && opts.gone) ok = ![...document.querySelectorAll(opts.gone)].some(visible);
      if (ok && opts.urlContains) ok = location.href.includes(opts.urlContains);
      const ms = Math.round(performance.now() - started);
      if (ok) resolve({ met: true, ms });
      else if (ms >= timeout) resolve({ met: false, ms });
      else setTimeout(check, 80);
    };
    check();
  });

  const meta = () => ({
    url: location.href,
    title: document.title,
    lang: document.documentElement.lang || null,
    description: document.querySelector('meta[name=description]')?.content || null,
    canonical: document.querySelector('link[rel=canonical]')?.href || null,
    readyState: document.readyState,
    signals: signals(),
    scroll: scrollState(),
    counts: { links: document.links.length, forms: document.forms.length, images: document.images.length, frames: document.querySelectorAll('iframe').length },
  });

  const setBase = base => { next = Math.max(next, base || 0); };
  // Forgets references so later calls with them fail as stale.
  const expire = list => {
    const gone = Array.isArray(list) ? list.map(Number) : [...els.keys()];
    for (const id of gone) { els.delete(id); prints.delete(id); }
    return { expired: gone.length };
  };
  const element = id => lookup(id).el || null;
  const rect = id => {
    const found = lookup(id);
    if (found.error) return found;
    found.el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const box = found.el.getBoundingClientRect();
    const [fx, fy] = frameOffset(found.el);
    return { x: Math.max(0, box.left + fx + scrollX), y: Math.max(0, box.top + fy + scrollY), w: box.width, h: box.height };
  };

  // Result records of a search page. Works on the common shape of Google,
  // Bing and DuckDuckGo: a link whose title is a heading.
  const searchResults = limit => {
    const seen = new Set();
    const results = [];
    for (const heading of document.querySelectorAll('a h3, h2 a, h3 a')) {
      const a = heading.closest('a');
      if (!a || !visible(a)) continue;
      let url = a.href;
      try {
        const parsed = new URL(url);
        if (parsed.pathname === '/url' && parsed.searchParams.get('q')) url = parsed.searchParams.get('q');
        else if (parsed.origin === location.origin && parsed.pathname === location.pathname) continue;
      } catch { continue; }
      if (!/^https?:/.test(url) || seen.has(url)) continue;
      seen.add(url);
      const title = clean(heading.innerText, 160);
      if (!title) continue;
      let box = a.parentElement;
      for (let up = 0; up < 5 && box && box.parentElement && (box.innerText || '').length < title.length + 40; up += 1) box = box.parentElement;
      const snippet = box ? clean((box.innerText || '').replace(heading.innerText, ''), 260) : '';
      results.push({ rank: results.length + 1, title, url, snippet, ref: refOf(a) });
      if (results.length >= limit) break;
    }
    return { results, signals: signals() };
  };

  // Text the page has selected, in a field or in the document.
  const selection = () => {
    const el = document.activeElement;
    if (el && /^(INPUT|TEXTAREA)$/.test(el.tagName) && typeof el.selectionStart === 'number' && el.type !== 'password') {
      return { text: el.value.slice(el.selectionStart, el.selectionEnd) };
    }
    return { text: String(getSelection()) };
  };

  // Find in page: counts the matches, then selects and shows match `index`.
  const find = (needle, index = 0) => {
    const want = String(needle).toLowerCase();
    if (!want) return { error: 'empty', detail: 'text to find is empty' };
    const hits = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node && hits.length < 500; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || /^(SCRIPT|STYLE|NOSCRIPT)$/.test(parent.tagName) || !visible(parent)) continue;
      const text = node.nodeValue.toLowerCase();
      for (let at = text.indexOf(want); at !== -1 && hits.length < 500; at = text.indexOf(want, at + want.length)) hits.push({ node, at });
    }
    if (!hits.length) return { count: 0, matches: [] };
    const pick = Math.min(Math.max(0, index), hits.length - 1);
    const range = document.createRange();
    range.setStart(hits[pick].node, hits[pick].at);
    range.setEnd(hits[pick].node, hits[pick].at + want.length);
    const selected = getSelection();
    selected.removeAllRanges();
    selected.addRange(range);
    hits[pick].node.parentElement.scrollIntoView({ block: 'center', inline: 'nearest' });
    const context = hit => clean(hit.node.nodeValue.slice(Math.max(0, hit.at - 40), hit.at + want.length + 40), 120);
    return { count: hits.length, index: pick, matches: hits.slice(pick, pick + 5).map((hit, offset) => ({ index: pick + offset, context: context(hit) })) };
  };

  // Types a saved sign-in into the visible login form. The values arrive from
  // the browser and are never echoed back.
  const fillLogin = (username, password) => {
    const set = (el, value) => {
      const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      el.focus();
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const usable = el => visible(el) && !el.disabled && !el.readOnly;
    const pass = [...document.querySelectorAll('input[type=password]')].find(usable);
    const scope = pass?.form || document;
    const users = [...scope.querySelectorAll('input[type=text],input[type=email],input:not([type]),input[type=tel]')].filter(usable);
    const user = users.find(el => /user|email|login|name|account|id/i.test(`${el.name} ${el.id} ${el.autocomplete} ${el.getAttribute('aria-label') || ''} ${el.placeholder}`))
      || (pass ? users.filter(el => el.compareDocumentPosition(pass) & Node.DOCUMENT_POSITION_FOLLOWING).pop() : users.length === 1 ? users[0] : null);
    if (!pass && !user) return { error: 'no-login-form', detail: 'no visible sign-in fields on this page' };
    const filled = [];
    if (user) { set(user, username); filled.push('username'); }
    if (pass) { set(pass, password); filled.push('password'); }
    return { filled, next: pass ? 'submit the form' : 'submit, then call credentials.fill again for the password step' };
  };

  // ---- The agent's presence on the page ------------------------------------
  // The agent has its own pointer and keyboard. They are drawn here, in a
  // closed shadow tree that takes no input, so the person can watch the agent
  // work while their own mouse and keyboard stay theirs. Everything moves
  // with the Web Animations API: no attribute changes for the page to notice.
  let stage = null;
  let parts = null;
  let at = null;
  let idle = 0;
  const ensureStage = () => {
    if (stage && stage.isConnected) return parts;
    stage = document.createElement('agentgo-stage');
    stage.setAttribute('aria-hidden', 'true');
    stage.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647;';
    const root = stage.attachShadow({ mode: 'closed' });
    root.innerHTML = `<style>
      :host { all: initial; }
      * { box-sizing: border-box; }
      .pointer { position:absolute; left:0; top:0; width:26px; height:26px; margin:-3px 0 0 -4px; opacity:0; will-change:transform,opacity;
        filter:drop-shadow(0 2px 5px rgba(0,0,0,.45)); }
      .ring { position:absolute; left:0; top:0; width:34px; height:34px; margin:-17px 0 0 -17px; border-radius:50%;
        border:2px solid #FF6A2B; opacity:0; will-change:transform,opacity; }
      .focus { position:absolute; left:0; top:0; border-radius:8px; opacity:0; will-change:opacity;
        box-shadow:0 0 0 2px #FF6A2B, 0 0 0 6px rgba(255,106,43,.22); }
      .note, .banner { position:absolute; font:600 12px/1 "Segoe UI",system-ui,sans-serif; color:#1C0D06; background:#FF6A2B;
        border-radius:999px; white-space:nowrap; opacity:0; will-change:transform,opacity; box-shadow:0 6px 18px rgba(0,0,0,.28); }
      .note { left:0; top:0; padding:6px 10px; }
      .banner { left:50%; top:14px; padding:9px 16px; max-width:70vw; overflow:hidden; text-overflow:ellipsis; }
      @media (prefers-reduced-motion: reduce) { .ring { display:none; } }
    </style>
    <div class="focus"></div>
    <div class="ring"></div>
    <svg class="pointer" viewBox="0 0 26 26"><path d="M5 3.5 20.5 13l-6.6 1.5L10.6 21z" fill="#FF6A2B" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>
    <div class="note"></div>
    <div class="banner"></div>`;
    (document.documentElement || document).appendChild(stage);
    parts = Object.fromEntries(['pointer', 'ring', 'focus', 'note', 'banner'].map(name => [name, root.querySelector('.' + name)]));
    return parts;
  };
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const place = (el, x, y) => `translate3d(${Math.round(x)}px,${Math.round(y)}px,0)`;
  const rest = () => {
    clearTimeout(idle);
    idle = setTimeout(() => parts && parts.pointer.animate([{ opacity: .35 }], { duration: 600, fill: 'forwards' }), 2600);
  };

  // Glides the agent's pointer to a point; `kind` click adds a ripple there.
  const pointTo = async (x, y, kind) => {
    const ui = ensureStage();
    const from = at || { x: Math.min(innerWidth - 40, Math.max(40, x + 160)), y: Math.max(30, y - 120) };
    const distance = Math.hypot(x - from.x, y - from.y);
    const duration = reduced() ? 0 : Math.min(420, Math.max(140, 110 + distance * 0.42));
    at = { x, y };
    const move = ui.pointer.animate(
      [{ transform: place(ui.pointer, from.x, from.y), opacity: 1 }, { transform: place(ui.pointer, x, y), opacity: 1 }],
      { duration, easing: 'cubic-bezier(.22,.8,.24,1)', fill: 'forwards' });
    await move.finished.catch(() => {});
    if (kind === 'click' && !reduced()) {
      ui.ring.animate(
        [{ transform: place(ui.ring, x, y) + ' scale(.35)', opacity: .95 }, { transform: place(ui.ring, x, y) + ' scale(1.25)', opacity: 0 }],
        { duration: 420, easing: 'cubic-bezier(.2,.7,.3,1)' });
      ui.pointer.animate(
        [{ transform: place(ui.pointer, x, y) + ' scale(1)' }, { transform: place(ui.pointer, x, y) + ' scale(.82)' }, { transform: place(ui.pointer, x, y) + ' scale(1)' }],
        { duration: 180 });
    }
    rest();
    return { ms: Math.round(duration) };
  };

  // Marks the field the agent is typing into.
  const typing = label => {
    const ui = ensureStage();
    const el = document.activeElement;
    if (!el || el === document.body) return { shown: false };
    const box = el.getBoundingClientRect();
    ui.focus.style.cssText = `width:${box.width}px;height:${box.height}px;transform:${place(ui.focus, box.left, box.top)}`;
    ui.focus.animate([{ opacity: 0 }, { opacity: 1, offset: .12 }, { opacity: 1, offset: .8 }, { opacity: 0 }], { duration: 1500 });
    ui.note.textContent = label || 'Agent is typing';
    const y = box.top > 44 ? box.top - 34 : box.bottom + 8;
    ui.note.animate(
      [{ transform: place(ui.note, box.left, y + 6), opacity: 0 }, { transform: place(ui.note, box.left, y), opacity: 1, offset: .12 },
        { transform: place(ui.note, box.left, y), opacity: 1, offset: .8 }, { transform: place(ui.note, box.left, y - 4), opacity: 0 }],
      { duration: 1500, easing: 'ease-out' });
    return { shown: true };
  };

  // A short notice at the top of the page (navigation, scrolling, waiting).
  const banner = text => {
    const ui = ensureStage();
    ui.banner.textContent = text;
    ui.banner.animate(
      [{ transform: 'translate3d(-50%,-14px,0)', opacity: 0 }, { transform: 'translate3d(-50%,0,0)', opacity: 1, offset: .14 },
        { transform: 'translate3d(-50%,0,0)', opacity: 1, offset: .78 }, { transform: 'translate3d(-50%,-8px,0)', opacity: 0 }],
      { duration: reduced() ? 1200 : 2000, easing: 'ease-out' });
    return { shown: true };
  };

  // Points at a referenced element before the agent acts on it.
  const pointAt = async (id, kind) => {
    const el = element(id);
    if (!el || !el.getBoundingClientRect) return { shown: false };
    el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    const box = el.getBoundingClientRect();
    return pointTo(box.left + Math.min(box.width / 2, 24), box.top + box.height / 2, kind);
  };

  // Screenshots leave the agent's presence out.
  const hideStage = hidden => {
    if (stage) stage.style.visibility = hidden ? 'hidden' : '';
    return { hidden: Boolean(stage && hidden) };
  };

  // A key press: noted next to the focused field, or at the top of the page.
  const keyNote = key => {
    const el = document.activeElement;
    return el && el !== document.body ? typing(`Agent pressed ${key}`) : banner(`Agent pressed ${key}`);
  };

  // Tells the browser (never the page) when a sign-in or sign-up form is sent,
  // so it can offer to save it. `__agReport` exists only in this world.
  let lastSignIn = '';
  const noticeSignIn = scope => {
    const root = scope && scope.querySelectorAll ? scope : document;
    const pass = [...root.querySelectorAll('input[type=password]')].find(el => el.value);
    if (!pass) return;
    const users = [...(pass.form || root).querySelectorAll('input[type=text],input[type=email],input:not([type]),input[type=tel]')].filter(el => el.value);
    const user = users.find(el => /user|email|login|name|account|id/i.test(`${el.name} ${el.id} ${el.autocomplete}`))
      || users.filter(el => el.compareDocumentPosition(pass) & Node.DOCUMENT_POSITION_FOLLOWING).pop();
    if (!user) return;
    const seen = `${user.value}\n${pass.value}`;
    if (seen === lastSignIn || typeof globalThis.__agReport !== 'function') return;
    lastSignIn = seen;
    globalThis.__agReport(JSON.stringify({ kind: 'login', username: user.value, password: pass.value }));
  };
  addEventListener('submit', event => noticeSignIn(event.target), true);
  addEventListener('click', event => {
    const button = event.target.closest && event.target.closest('button,input[type=submit],[role=button]');
    if (button) noticeSignIn(button.form || document);
  }, true);
  addEventListener('keydown', event => {
    if (event.key === 'Enter' && event.target.tagName === 'INPUT') noticeSignIn(event.target.form || document);
  }, true);

  globalThis.__ag = { expire, focusedEditable, pointerAt: () => at, pointTo, pointAt, keyNote, hideStage, typing, banner, fillLogin, searchResults, selection, find, setBase, element, rect, enterRisk, snapshot, target, focusForInput, choose, valueOf, text, links, tables, forms, settle, scroll, waitFor, meta, signals, injectionChunks, translateText, translatePage };
  // Helpers for the page tools (tools.js), which extend this agent.
  globalThis.__ag.h = { lookup, visible, clean, refOf, describe, signals };
})();
