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
  // The row a repeated control was shown in ("Dismiss" in "Invoice #4471 is
  // overdue"), checked again before the reference is acted on.
  const rows = new Map();
  let next = 0;
  // Elements DevTools reported a click/mousedown listener on (see
  // listenerCandidates); script-driven controls with no pointer cursor and no
  // text (icons, calendar cells, chart slices) are only found this way.
  const listened = new WeakSet();
  // How often an element was asked about: pages often bind their listeners a
  // moment after drawing the element, so a miss is asked again (3 times).
  const checked = new WeakMap();
  let candidateBatch = [];

  const TAG_ROLE = { A: 'link', BUTTON: 'button', SELECT: 'combobox', TEXTAREA: 'textbox', SUMMARY: 'button', OPTION: 'option', IMG: 'img' };
  const INPUT_ROLE = { button: 'button', submit: 'button', reset: 'button', image: 'button', checkbox: 'checkbox', radio: 'radio', range: 'slider', search: 'searchbox', file: 'file' };
  const ROLES = new Set(['button', 'link', 'checkbox', 'radio', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'switch', 'combobox', 'textbox', 'searchbox', 'slider', 'spinbutton', 'treeitem', 'listbox']);
  const MONEY = /\b(buy|purchase|pay|place (your )?order|check ?out|subscribe|donate|bid|confirm (order|payment|purchase)|complete (order|purchase)|send money|transfer funds|upgrade (now|plan))\b/i;
  // Words that are a payment only when a price is in sight: a newsletter
  // "Subscribe" or a "Donate" link alone moves no money.
  const SOFT_MONEY = /\b(subscribe|donate|bid|upgrade (now|plan))\b/i;
  const HARD_MONEY = /\b(buy|purchase|pay|place (your )?order|check ?out|confirm (order|payment|purchase)|complete (order|purchase)|send money|transfer funds)\b/i;
  const PRICE = /([$€£¥₹]\s?\d)|(\d[\d.,]*\s?([$€£¥₹]|\b(usd|eur|gbp|sar|aed|jod|egp|kwd|qar|try|inr|jpy|cny)\b))|(\b(usd|eur|gbp|sar|aed|jod|egp|kwd|qar|try|inr|jpy|cny)\s?\d)|(\/\s?(mo|month|yr|year)\b)|\bper (month|year|week)\b/i;
  // Inputs whose value is set as a whole (a picker, not typed characters).
  const VALUE_TYPES = /^(date|time|datetime-local|month|week|color|range)$/;
  const CARD = 'input[autocomplete^="cc-"],input[name*="card" i],input[name*="cvv" i],input[name*="cvc" i],input[id*="cardnumber" i]';
  // The same words in other languages (no \b: it only knows ASCII letters).
  const MONEY_INTL = /(jetzt kaufen|kaufen|bezahlen|zahlungspflichtig|kostenpflichtig bestellen|acheter|payer|commander et payer|comprar|pagar|finalizar compra|acquista|paga ora|ordina e paga|kopen|betalen|afrekenen|köp|betala|kup teraz|zapłać|купить|оплатить|оформить заказ|購入|支払|注文を確定|구매|결제|购买|支付|付款|提交订单|شراء|ادفع|إتمام الشراء|satın al|öde)/i;
  // Card fields a payment provider draws in its own frame.
  const PAY_FRAME = /stripe|adyen|braintree|paypal|checkout|klarna|squareup|worldpay|authorize\.net|recurly|chargebee|mollie|razorpay|payment|card/i;

  const clean = (text, max = 80) => {
    const value = String(text ?? '').replace(/\s+/g, ' ').trim();
    return value.length > max ? value.slice(0, max - 1) + '…' : value;
  };
  const quote = text => JSON.stringify(text);
  // A field's value as it is (edge spaces and line breaks kept), only cut:
  // an agent copies it, and a trimmed copy is a different value.
  const exact = (text, max = 60) => {
    const value = String(text ?? '');
    return value.length > max ? value.slice(0, max - 1) + '…' : value;
  };
  // What identifies an outline line across re-renders: without the value
  // and the row context, which change while the element stays the same.
  // State words go too: a button that becomes enabled, a box that is ticked,
  // is the same element.
  const printOf = line => line.replace(/ in ".*"$/, '').replace(/ value=.*$/, '')
    .replace(/( (checked|selected|expanded|collapsed|haspopup|disabled|readonly|required))+$/, '');

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
    // A list shown open (select multiple, or with a size): a listbox.
    if (el.tagName === 'SELECT' && (el.multiple || el.size > 1)) return 'listbox';
    if (el.tagName === 'A') return el.hasAttribute('href') ? 'link' : 'clickable';
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

  // A name for a control that has no text at all (an icon): its id or class
  // words, as "(icon: send reply)", so the agent can tell icons apart.
  const GENERIC_CLASS = /^(icon|icons|ico|btn|button|fa|fas|far|fab|material-icons|glyphicon|svg|img|ui|js|is|has|active|hover|selected|clickable|pointer|hide|hidden|show|row|col|item|wrapper|container|inner|outer)$/i;
  const words = s => String(s || '').replace(/([a-z])([A-Z])/g, '$1 $2').replace(/(\p{L})[-_]+(?=\p{L})/gu, '$1 ').replace(/_+/g, ' ').replace(/\b(fa|icon|ico|btn|glyphicon)\b/gi, ' ').replace(/\s+/g, ' ').trim().toLowerCase();
  const iconHint = el => {
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) || el.getAttribute('role')) return '';
    for (const node of [el, ...el.querySelectorAll('*')].slice(0, 4)) {
      const own = words(node.id);
      if (own && !/\d{3,}/.test(own)) return `(icon: ${own})`;
      const cls = [...(node.classList || [])].filter(c => !GENERIC_CLASS.test(c) && !/\d{3,}|^[a-z]{1,2}$|^(css|sc|jsx|_)/i.test(c));
      if (cls.length) return `(icon: ${[...new Set(words(cls.slice(0, 2).join(' ')).split(' '))].join(' ')})`;
    }
    return '';
  };

  // A field with no label element but a text right before it ("Genre" in a
  // div, then the input), as many sites build forms: that text names it.
  const visualLabel = el => {
    if (!/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName)) return '';
    for (let prev = el.previousElementSibling, steps = 0; prev && steps < 2; prev = prev.previousElementSibling, steps++) {
      if (prev.matches('input,select,textarea,button')) break;
      // A label is one short line, not a list or table that happens to come
      // before the field.
      if (/^(UL|OL|TABLE|DL)$/.test(prev.tagName)) break;
      const text = (prev.innerText || '').trim();
      if (text && text.length <= 40 && !text.includes('\n') && !prev.querySelector('input,select,textarea,button')) return text;
    }
    // The text of the field's own small box ("Name: [field]"), measured
    // before it is cut: a whole page around the field is not its label.
    const parent = el.parentElement;
    const own = parent && parent !== document.body ? ownText(parent).replace(/\s+/g, ' ').trim() : '';
    if (own.length >= 2 && own.length <= 40) return own;
    // A field alone in its own box ("Departure Date" in a div, then a div
    // holding only the input): the text before the box names it.
    const box = el.parentElement;
    if (box && box.children.length === 1 && box !== document.body) {
      const prev = box.previousElementSibling;
      const text = (prev?.innerText || '').trim();
      if (prev && text && text.length <= 40 && !text.includes('\n') && !prev.querySelector('input,select,textarea,button')) return text;
    }
    return '';
  };

  // The text an element shows without the nested lists it holds: an open
  // tree folder is "Keli", not "Keli Briana Jess".
  const shownText = el => {
    // innerText is what is shown; textContent (hidden text too) only for SVG,
    // which has no innerText.
    if (![...el.children].some(child => /^(UL|OL)$/.test(child.tagName) || child.getAttribute('role') === 'group')) return el.innerText ?? el.textContent;
    return [...el.childNodes].filter(node => !(node.nodeType === 1 && (/^(UL|OL)$/.test(node.tagName) || node.getAttribute('role') === 'group'))).map(node => node.nodeType === 1 ? node.innerText : node.textContent).join(' ');
  };
  // "radio 2 of 3" for a radio or checkbox nothing names: "the 2nd radio
  // button" then needs no counting.
  const ordinal = el => {
    if (!/^(radio|checkbox)$/.test(el.type) || !el.name) return '';
    const group = [...(el.form || document).querySelectorAll(`input[type=${el.type}]`)].filter(other => other.name === el.name);
    return group.length > 1 ? `${group.indexOf(el) + 1} of ${group.length}` : '';
  };

  const nameOf = el => clean(
    el.getAttribute('aria-label') || labelText(el) ||
    (el.tagName === 'INPUT' && /^(button|submit|reset)$/.test(el.type) ? el.value : '') ||
    el.getAttribute('placeholder') || el.getAttribute('alt') ||
    (/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) ? el.getAttribute('title') || visualLabel(el) || (/^(radio|checkbox)$/.test(el.type) ? '' : el.name) : shownText(el)) ||
    el.getAttribute('title') || el.querySelector?.('img[alt]')?.alt || el.querySelector?.('svg title')?.textContent ||
    // A control with no name of its own (a bare checkbox in a list row) is
    // named by the row it sits in, as a person reads it.
    (/^(checkbox|radio)$/.test(el.type || '') || !el.textContent?.trim()
      ? (el.closest('li,tr,[role=row],[role=listitem],label')?.innerText || '').slice(0, 80)
      : '') || ordinal(el) || (/^(radio|checkbox)$/.test(el.type || '') ? el.name : '') || iconHint(el) || ''
  );

  const interactive = el => {
    const tag = el.tagName;
    if (tag === 'INPUT') return el.type !== 'hidden';
    if (listened.has(el)) return true;
    if (tag === 'A' && el.hasAttribute('href')) return true;
    // A link without an address that a script handles (a datepicker's Prev
    // and Next): it says so with a title or a data-handler.
    if (tag === 'A' && (el.hasAttribute('title') || [...el.attributes].some(a => /^data-(handler|action|event)$/.test(a.name)))) return true;
    if (tag === 'BUTTON' || tag === 'SELECT' || tag === 'TEXTAREA' || tag === 'SUMMARY') return true;
    if (ROLES.has(el.getAttribute('role'))) return true;
    if (el.isContentEditable && !el.parentElement?.isContentEditable) return true;
    if (el.hasAttribute('onclick') || (el.hasAttribute('tabindex') && el.tabIndex >= 0)) return true;
    if (el.getAttribute('draggable') === 'true') return true;
    // Script-driven controls (a div with a click listener) usually show the
    // pointer cursor. Only the outermost such element counts, not its text.
    if (/^(A|DIV|SPAN|LI|IMG|SVG|I|TD|TR|P|SECTION|ARTICLE|LABEL|H[1-6]|DT|SUMMARY)$/i.test(tag) && el.firstChild !== null || tag === 'IMG' || tag === 'A') {
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
      if (el.value) line += ` value=${quote(exact(el.value))}`;
    } else if (el.tagName === 'SELECT' && (el.multiple || el.size > 1)) {
      // An open list shows its options, not one value: name a few of them.
      const chosen = [...el.selectedOptions].map(o => clean(o.textContent, 30)).join(', ');
      line += ` value=${quote(clean(chosen, 60))}${el.multiple ? ' multiple' : ''}`;
      const all = [...el.options].map(o => clean(o.textContent, 24));
      line += ` options=${quote(all.slice(0, 6).join(' | ') + (all.length > 6 ? ` … ${all.length} in all` : ''))}`;
    } else if (el.tagName === 'SELECT') {
      line += ` value=${quote(clean(el.selectedOptions[0]?.textContent, 40))}`;
    } else if (el.isContentEditable) {
      const text = clean(el.innerText, 60);
      if (text) line = `textbox ${quote(clean(el.getAttribute('aria-label') || el.getAttribute('data-placeholder') || 'editor'))} value=${quote(text)}`;
    }
    // "+", "x" or an icon means nothing on its own when a page has many of
    // them: say which row it belongs to ("+" in "Chicken wings 2").
    const own = nameOf(el);
    let context = '';
    if (!/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) && (own.length <= 1 || !/\p{L}{2}/u.test(own) || own.startsWith('(icon:'))) {
      for (let up = el.parentElement, depth = 0; up && depth < 4 && up !== document.body; up = up.parentElement, depth++) {
        // Without the symbols of the row's own buttons ("Chicken wings - +").
        const around = (up.innerText || '').replace(/(^|\s)[^\p{L}\p{N}\s]{1,2}(?=\s|$)/gu, ' ').replace(/\s+/g, ' ').trim();
        // Past a row's size it is a card or a page, not this control's row.
        if (around.length > 80) break;
        if (around.length > own.length + 2) {
          // A row of numbers (a calendar week) says nothing more than the label.
          if (/\p{L}{3,}/u.test(around)) context = ` in ${quote(clean(around, 60))}`;
          break;
        }
      }
    }
    if (el.checked || el.getAttribute('aria-checked') === 'true') line += ' checked';
    if (el.getAttribute('aria-selected') === 'true') line += ' selected';
    // A tree item that holds a nested list (or the label of one) says
    // whether it is open: a folder is opened, a file is the answer.
    const nestedList = node => [...(node?.children || [])].find(child => /^(UL|OL)$/.test(child.tagName) || child.getAttribute('role') === 'group');
    const holder = nestedList(el) ? el : (el.parentElement?.firstElementChild === el && nestedList(el.parentElement) ? el.parentElement : null);
    if (!el.getAttribute('aria-expanded') && holder && !/^(INPUT|SELECT|TEXTAREA|BUTTON|A)$/.test(el.tagName)) {
      const list = nestedList(holder);
      line += visible(list) ? ' expanded' : ' collapsed';
    }
    if (el.getAttribute('aria-expanded')) line += el.getAttribute('aria-expanded') === 'true' ? ' expanded' : ' collapsed';
    // A menu item with a submenu opens it on hover; clicking may choose it.
    else if (/^(true|menu)$/.test(el.getAttribute('aria-haspopup') || '')) line += ' haspopup';
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') line += ' disabled';
    else if (el.readOnly && /^(INPUT|TEXTAREA)$/.test(el.tagName)) line += ' readonly';
    if (el.required) line += ' required';
    return line + context;
  };

  // A popup's question ("Exit to home page?") is plain text, so the outline
  // showed only its OK/Cancel. Dialogs, and containers named like popups that
  // sit above the page with a button in them, get one text line.
  const POPUP_NAME = /popup|modal|dialog|overlay|toast|alert|consent|cookie|lightbox|datepicker|calendar|picker/i;
  const overlayText = el => {
    const role = el.getAttribute('role');
    const marked = role === 'dialog' || role === 'alertdialog' || el.getAttribute('aria-modal') === 'true';
    if (!marked) {
      if (!/^(DIV|SECTION|ASIDE|FORM)$/.test(el.tagName) || !POPUP_NAME.test(`${el.id} ${typeof el.className === 'string' ? el.className : ''}`)) return '';
      if (!el.querySelector('button,[role=button],input[type=button],input[type=submit],a[href]')) return '';
      if (!/^(fixed|absolute|sticky)$/.test(getComputedStyle(el).position)) return '';
    }
    if (!visible(el)) return '';
    let text = el.innerText || '';
    for (const control of el.querySelectorAll('button,[role=button],a[href],input,select,textarea')) {
      const label = (control.innerText || control.value || '').trim();
      if (label) text = text.replace(new RegExp(`(^|\\s)${label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?=\\s|$)`), '$1 ');
    }
    return clean(text, 160);
  };

  // Text of an element with that of its inline children (not controls),
  // which are added to `absorbed` so they are not read a second time.
  const INLINE = /^(SPAN|B|I|EM|STRONG|SMALL|SUB|SUP|MARK|CODE|KBD|ABBR|TIME|Q|S|U|FONT|BDI|DATA|VAR|CITE|DFN)$/;
  const inlineText = (el, absorbed) => [...el.childNodes].map(node => {
    if (node.nodeType === 3) return node.textContent;
    if (node.nodeType !== 1 || !INLINE.test(node.tagName) || interactive(node) || node.querySelector('a[href],button,input,select,textarea')) return ' ';
    absorbed.add(node);
    return inlineText(node, absorbed);
  }).join('');

  // The text of the row a control sits in (a list item, a table row, a
  // card), without the symbols of the row's buttons and without the
  // control's own name; '' when the control has no row of its own.
  const plain = text => String(text || '').replace(/(^|\s)[^\p{L}\p{N}\s]{1,2}(?=\s|$)/gu, ' ').replace(/\s+/g, ' ').trim();
  const rowOf = el => {
    const own = plain(nameOf(el));
    for (let up = el.parentElement, depth = 0; up && depth < 5 && up !== document.body; up = up.parentElement, depth++) {
      let around = plain(up.innerText);
      if (around.length > 160) break;
      if (around.length > own.length + 2) {
        if (own.length > 1) around = around.replace(own, ' ').replace(/\s+/g, ' ').trim();
        return /[\p{L}\p{N}]{2}/u.test(around) ? around : '';
      }
    }
    return '';
  };
  // How much two row texts are the same row: shared words first, shared
  // numbers only to tell rows with the same words apart ("Invoice #4471" and
  // "Invoice #4478"); a score or a rank that changed keeps a row the same.
  const tokens = (text, digits) => new Set(String(text).toLowerCase().match(digits ? /\d+/g : /\p{L}{2,}/gu) || []);
  const overlap = (a, b) => {
    if (!a.size && !b.size) return 1;
    let n = 0;
    for (const t of a) if (b.has(t)) n++;
    return n / (a.size + b.size - n);
  };
  const likeness = (a, b) => overlap(tokens(a), tokens(b)) + 0.01 * overlap(tokens(a, true), tokens(b, true));

  const collect = (opts = {}) => {
    const items = [];
    const absorbed = new WeakSet();
    const height = innerHeight, width = innerWidth;
    walk(document.documentElement, (el, offset) => {
      if (el.getAttribute('aria-hidden') === 'true' && !el.contains(document.activeElement)) return false;
      const isInteractive = interactive(el);
      // A heading that opens something (an accordion section) is a control.
      const heading = /^H[1-4]$/.test(el.tagName) && !isInteractive;
      if (!heading && !isInteractive) {
        const said = overlayText(el);
        if (said.length >= 4) items.push({ line: `dialog ${quote(said)}` });
      }
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
        prints.set(id, printOf(line));
        items.push({ id, line, el });
        // Controls inside a link/button are part of that one target.
        return !/^(A|BUTTON|SUMMARY|SELECT)$/.test(el.tagName) && !/^(button|link|tab|menuitem|option)$/.test(el.getAttribute('role') || '');
      }
      if (opts.text && /^(P|LI|TD|TH|DT|DD|BLOCKQUOTE|PRE|FIGCAPTION|SPAN|DIV)$/.test(el.tagName) && !absorbed.has(el)) {
        // A sentence with a word in its own inline element ("lower than
        // <span>5</span>.") is read whole, and that element not again.
        const own = clean(inlineText(el, absorbed), 200);
        if (own.length > 2) items.push({ line: `· ${own}` });
      }
      return true;
    });
    // A clickable inside a clickable with the same name (a menu row and the
    // wrapper of its text, a tree row and its label) is one target: keep the
    // inner one, whose centre is surely on it.
    const kept = items.filter((item, i) => {
      if (!item.el || roleOf(item.el) !== 'clickable') return true;
      const name = nameOf(item.el);
      // A drawn shape named only by its classes and the text label drawn on
      // it (a pie menu's slice and its letter) are one target: keep the label.
      // Keep the shape (the larger target, still hit while the menu moves)
      // under the label's name, and drop the label's own line.
      if (item.merged) return false;
      const next = items[i + 1];
      if (name.startsWith('(icon:') && next?.el && !item.el.contains(next.el) && !next.el.contains(item.el) && !nameOf(next.el).startsWith('(icon:')) {
        const box = item.el.getBoundingClientRect(), label = next.el.getBoundingClientRect();
        const cx = label.left + label.width / 2, cy = label.top + label.height / 2;
        if (cx >= box.left && cx <= box.right && cy >= box.top && cy <= box.bottom && label.width * label.height < box.width * box.height) {
          item.line = item.line.replace(/^(\S+) "[^"]*"/, `$1 ${quote(nameOf(next.el))}`);
          prints.set(item.id, printOf(item.line));
          next.merged = true;
        }
      }
      for (let j = i + 1; j < Math.min(items.length, i + 4); j++) {
        const inner = items[j].el;
        // ...also when the inner one says what it is (a menuitem, an option,
        // a link): clicking the wrapper of a menu item may not choose it.
        if (inner && item.el.contains(inner) && nameOf(inner) === name) return false;
      }
      return true;
    });
    // Several controls that read the same (a "Dismiss" per notification, a
    // "Like (12)" per post; numbers aside) each say which row they are in, so
    // the agent need not count, and the row is kept to check the reference
    // against before it is acted on.
    const same = line => printOf(line).replace(/\d+/g, '#');
    const count = new Map();
    for (const item of kept) if (item.id) count.set(same(item.line), (count.get(same(item.line)) || 0) + 1);
    for (const item of kept) {
      if (!item.id || count.get(same(item.line)) < 2) continue;
      const row = rowOf(item.el);
      if (!row) continue;
      rows.set(item.id, row);
      if (!item.line.includes(' in "')) item.line += ` in ${quote(clean(row, 60))}`;
    }
    return kept;
  };

  // Cloudflare's full-page check ("Just a moment..."). Recognised by its
  // structure rather than its wording, so translated titles still match, and
  // never by the analytics script (/challenge-platform/scripts/jsd/) that
  // many ordinary Cloudflare sites load. Detection only: AgentGo waits for
  // the page's own check to finish or hands the tab to the human.
  const interstitial = () => {
    if (document.querySelector('#challenge-form,#challenge-running,#challenge-stage,#cf-challenge-running,#cf-please-wait,.cf-browser-verification,[id^="cf-chl-widget"]')) return true;
    if (document.querySelector('script[src*="/cdn-cgi/challenge-platform/"][src*="/orchestrate/"],script[src*="/cdn-cgi/challenge-platform/"][src*="/chl_page/"]')) return true;
    // AWS WAF's browser check: a nearly empty page that loads its challenge
    // script and reloads itself when it passes.
    // Sites behind AWS WAF also load its token script on ordinary pages
    // (IMDb, Hugging Face): only a page with next to no text is the check.
    if (document.body.children.length < 12 && (document.body.innerText || '').trim().length < 300 && document.querySelector('script[src*="awswaf"],script[src*="/challenge.js"],script[src*="/challenge.compact.js"]')) return true;
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

  // A challenge widget counts only when a person would see one to solve.
  // Sites load invisible reCAPTCHA (v3, "size=invisible", the corner badge)
  // and hidden Turnstile frames on ordinary pages; those are not challenges.
  const CHALLENGE = 'iframe[src*="recaptcha"],iframe[src*="hcaptcha"],iframe[src*="challenges.cloudflare.com"],iframe[src*="captcha-delivery.com"],iframe[src*="arkoselabs"],iframe[src*="funcaptcha"],.g-recaptcha,.h-captcha,.cf-turnstile,[data-sitekey]';
  const challengeShown = el => {
    if (el.closest('.grecaptcha-badge') || el.getAttribute('data-size') === 'invisible') return false;
    if (/[?&]size=invisible/.test(el.getAttribute('src') || '')) return false;
    if (!visible(el)) return false;
    if (el.tagName !== 'IFRAME') return true;
    const rect = el.getBoundingClientRect();
    return rect.width >= 60 && rect.height >= 30;
  };

  const signals = () => {
    const out = {};
    const body = document.body;
    if (!body) return out;
    if (interstitial()) {
      out.captcha = true;
      out.challenge = 'interstitial';
    }
    if (document.querySelector('#cf-challenge-running') || [...document.querySelectorAll(CHALLENGE)].some(challengeShown) ||
        /verify (that )?you('| a)re (a )?human|i'?m not a robot|unusual traffic from your|complete the security check|are you a robot/i.test(document.title + ' ' + (body.innerText || '').slice(0, 1500))) out.captcha = true;
    if ([...document.querySelectorAll('input[type=password]')].some(visible)) out.login = true;
    if ([...document.querySelectorAll(CARD)].some(visible)) out.payment = true;
    return out;
  };

  // Elements the outline would skip that might be script-driven controls:
  // visible, not already a target, not inside one, not page-sized (a page or
  // section-wide listener is delegation, not a control). The native layer
  // asks DevTools which of them have a click/mousedown listener and passes
  // the indexes to markListened. Each element is checked once.
  const listenerCandidates = (limit = 300) => {
    const out = [];
    // Text-less elements (icons) go first: nothing else finds them, while a
    // text element usually shows a pointer cursor too.
    const later = [];
    const area = innerWidth * innerHeight;
    let scanned = 0;
    walk(document.documentElement, el => {
      if (out.length >= limit || ++scanned > 6000) return false;
      if (el === document.body || /^(HTML|HEAD|BR|HR|OPTION|IFRAME)$/.test(el.tagName)) return true;
      if (/^(A|BUTTON|SELECT|TEXTAREA|INPUT|SUMMARY|LABEL)$/.test(el.tagName)) return false;
      const tries = checked.get(el) || 0;
      if (tries >= 3) return true;
      if (interactive(el) || !visible(el)) return true;
      checked.set(el, tries + 1);
      const rect = el.getBoundingClientRect();
      if (rect.width * rect.height > area * 0.35) return true;
      if (/^(presentation|none)$/.test(el.getAttribute('role') || '')) return true;
      // Text inside a row that is already a target (a date, a rating in a
      // clickable list row) is part of that row; an icon there is its own.
      if ((el.innerText || '').trim()) {
        let covered = false;
        // ...unless the element is one of a list of alike rows under that
        // listener (menu items under a menu that delegates).
        for (let up = el.parentElement, child = el, depth = 0; up && depth < 4 && !covered; child = up, up = up.parentElement, depth++) {
          const alike = [...up.children].filter(other => other.tagName === child.tagName).length;
          covered = (listened.has(up) || interactive(up)) && alike < 2;
        }
        if (covered) return true;
        later.push(el);
        return true;
      }
      out.push(el);
      return true;
    });
    const all = out.concat(later.slice(0, Math.max(0, limit - out.length)));
    candidateBatch = all;
    return all;
  };
  const markListened = indexes => {
    const marked = [];
    for (const i of indexes) {
      const el = candidateBatch[i];
      if (el?.isConnected) { listened.add(el); marked.push(el); }
    }
    candidateBatch = [];
    // A container of controls (a menu, a list with a delegated listener) is
    // not itself a target, and a wrapper whose text is that of the control
    // inside it is that control: keep only the control.
    const CONTROL = 'a[href],button,input,select,textarea,[role=button],[role=link],[role=menuitem],[role=option],[role=tab],[role=checkbox],[role=radio],[role=treeitem]';
    for (const el of marked) {
      // Controls inside: native ones and script-driven ones found now or
      // before (a row's trash and star icons).
      const inner = [...el.querySelectorAll('*')].filter(node => (node.matches(CONTROL) || listened.has(node)) && visible(node));
      const text = (el.innerText || '').trim();
      if (inner.length > 1 || (inner.length === 1 && (inner[0].innerText || '').trim() === text)) listened.delete(el);
    }
    // A listener on a row and another on its text are one target to a
    // person: keep the outer one when both show the same text.
    for (const el of marked) {
      const text = (el.innerText || '').trim();
      for (let up = el.parentElement, depth = 0; up && depth < 3; up = up.parentElement, depth++) {
        if ((listened.has(up) || interactive(up)) && (up.innerText || '').trim() === text && text) { listened.delete(el); break; }
      }
    }
    return { marked: marked.filter(el => listened.has(el)).length };
  };

  // Sets a picker-style input (date, time, color, range...) to a value given
  // in the field's own format or a common written one, with the events a
  // person's change fires. Returns the value the field holds afterwards.
  const pad = n => String(n).padStart(2, '0');
  const normalizeValue = (type, raw) => {
    const text = String(raw).trim();
    const date = s => {
      let m = s.match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/);
      if (m) return `${m[1]}-${pad(m[2])}-${pad(m[3])}`;
      m = s.match(/^(\d{1,2})[\/.](\d{1,2})[\/.](\d{4})$/);
      if (m) {
        let [a, b] = [+m[1], +m[2]];
        // Month first unless the first number cannot be a month.
        if (a > 12 && b <= 12) [a, b] = [b, a];
        return `${m[3]}-${pad(a)}-${pad(b)}`;
      }
      const parsed = new Date(s);
      return isNaN(parsed) ? s : `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())}`;
    };
    const time = s => {
      const m = s.match(/^(\d{1,2})(?::(\d{2}))?(?::(\d{2}))?\s*([ap]\.?m\.?)?$/i);
      if (!m) return s;
      let h = +m[1];
      if (m[4]) { const pm = /p/i.test(m[4]); if (h === 12) h = pm ? 12 : 0; else if (pm) h += 12; }
      return `${pad(h)}:${m[2] || '00'}${m[3] ? ':' + m[3] : ''}`;
    };
    switch (type) {
      case 'date': return date(text);
      case 'time': return time(text);
      case 'datetime-local': {
        const m = text.match(/^(.+?)[T\s,]+(\d{1,2}(?::\d{2}){0,2}\s*(?:[ap]\.?m\.?)?)$/i);
        return m ? `${date(m[1])}T${time(m[2])}` : text;
      }
      case 'month': {
        const m = text.match(/^(\d{1,2})[\/.-](\d{4})$/);
        return m ? `${m[2]}-${pad(m[1])}` : text;
      }
      case 'color': {
        const m = text.match(/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i);
        if (!m) return text.toLowerCase();
        const hex = m[1].length === 3 ? m[1].split('').map(c => c + c).join('') : m[1];
        return '#' + hex.toLowerCase();
      }
      default: return text;
    }
  };
  const setValue = (id, raw) => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    if (el.disabled || el.readOnly) return { error: 'disabled', detail: `${describe(el)} is read-only or disabled` };
    const wanted = normalizeValue(el.type, raw);
    el.focus();
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(el, wanted);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    if (el.value !== wanted && !(el.type === 'range' && el.value === String(Number(wanted)))) {
      const format = { date: 'YYYY-MM-DD or MM/DD/YYYY', time: 'HH:MM', 'datetime-local': 'YYYY-MM-DDTHH:MM', month: 'YYYY-MM', week: 'YYYY-Www', color: '#rrggbb', range: `a number from ${el.min || 0} to ${el.max || 100}` }[el.type] || 'its own format';
      return { error: 'invalid', detail: `${describe(el)} did not take ${quote(String(raw))}; it expects ${format}` };
    }
    return { value: el.value, line: describe(el) };
  };

  // Whether a field holds a supplied value: the same text, the same value in
  // a picker's own format (09/20/2019 is 2019-09-20), or a select showing it.
  const holds = (id, raw) => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el, want = String(raw);
    if (el.tagName === 'SELECT') {
      const option = el.selectedOptions[0];
      const low = want.trim().toLowerCase();
      return { holds: !!option && (option.value.toLowerCase() === low || option.textContent.trim().toLowerCase() === low) };
    }
    const value = el.value ?? el.innerText ?? '';
    if (value === want) return { holds: true };
    if (el.tagName === 'INPUT' && VALUE_TYPES.test(el.type)) return { holds: value === normalizeValue(el.type, want) };
    return { holds: false };
  };

  const scrollState = () => {
    const root = document.scrollingElement || document.documentElement;
    return { y: Math.round(root.scrollTop), h: root.scrollHeight, vh: innerHeight };
  };

  const snapshot = (opts = {}) => {
    const max = opts.max || 150;
    const start = opts.offset || 0;
    // A short page is shown whole (its text lines too): one read instead of
    // an outline and then the text.
    const small = !opts.text && !opts.viewport && (document.body?.innerText || '').length <= 800;
    let items = collect(small ? { ...opts, text: true } : opts);
    if (small) {
      // Text that only repeats a control's name (its label) is already there.
      const names = items.filter(item => item.id).map(item => ((item.line.match(/"(.*?)"/) || [])[1] || '').toLowerCase()).filter(Boolean);
      items = items.filter(item => {
        if (item.id || !item.line.startsWith('· ')) return true;
        const text = item.line.slice(2).toLowerCase().replace(/[:*]\s*$/, '').trim();
        // "Username:" repeats the field "Username"; "5 - x = 79" does not
        // repeat the field "x =" it explains.
        return !names.some(name => name === text || name.includes(text) || (text.length <= 40 && text.includes(name) && name.length * 2 >= text.length));
      });
    }
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
    const row = rows.get(id);
    if (el?.isConnected) {
      if (!row) return { el };
      const now = rowOf(el);
      if (now === row) return { el };
      // A list that re-rendered by position (React's key={index}, a recycled
      // virtual row) shows another item in the same node: act on the element
      // that shows the row the agent was shown, or say what changed, never on
      // whatever the node shows now.
      const print = prints.get(id), here = likeness(now, row);
      let best = null, score = here;
      for (const item of collect()) {
        if (!item.id || item.el === el || printOf(item.line) !== print) continue;
        const s = likeness(rowOf(item.el), row);
        if (s > score) { best = item; score = s; }
      }
      if (!best && here >= 0.5) { rows.set(id, now); return { el }; }
      if (best && score >= 0.5) { rows.set(best.id, row); return { el: best.el, healed: best.id }; }
      return { error: 'stale', detail: `reference ${id} now shows ${quote(clean(now, 60))}, not ${quote(clean(row, 60))} (the list re-rendered)` };
    }
    const print = prints.get(id);
    if (!print) return { error: 'stale', detail: `reference ${id} does not exist in this document` };
    let matches = collect().filter(item => item.id && printOf(item.line) === print);
    // A repeated control that was re-created: the one in the same row.
    if (row && matches.length > 1) {
      const scored = matches.map(item => [item, likeness(rowOf(item.el), row)]).sort((a, b) => b[1] - a[1]);
      if (scored[0][1] >= 0.5 && scored[0][1] > scored[1][1]) matches = [scored[0][0]];
    }
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
    if (MONEY.test(label) || MONEY_INTL.test(label)) {
      // "Subscribe", "Donate" or a ticked box move money only with a price
      // in sight (on the control or in its small surrounding block).
      const soft = SOFT_MONEY.test(label) && !HARD_MONEY.test(label) && !MONEY_INTL.test(label);
      const choice = /^(checkbox|radio)$/.test(el.type || '');
      if (soft || choice) {
        const around = (el.closest('form,fieldset,li,tr,section,article') || el.parentElement)?.innerText || '';
        if (!PRICE.test(label) && !PRICE.test(around.slice(0, 600))) return null;
      }
      return clean(label, 60);
    }
    if (isSubmitter(el) && formRisk(el) === 'money') return 'submits a form with payment card fields';
    return null;
  };

  // A frame this world cannot read that looks like a payment provider's, or
  // any unreadable frame on a page that takes card details.
  const payFrame = frame => {
    let readable = false;
    try { readable = !!frame.contentDocument; } catch (_) {}
    if (readable) return false;
    const pageTakesCards = !!document.querySelector(CARD) || [...document.querySelectorAll('iframe')].some(f => PAY_FRAME.test(`${f.src} ${f.name} ${f.title}`));
    return PAY_FRAME.test(`${frame.src} ${frame.name} ${frame.title}`) || pageTakesCards;
  };

  // What a click at a viewport point would press, with the same money and
  // submission checks a click by reference gets.
  const riskAt = (x, y) => {
    const hit = deepHit(x, y);
    if (!hit) return { line: 'nothing', money: null, submits: false };
    if (/^I?FRAME$/.test(hit.tagName)) {
      return { line: describe(hit), money: payFrame(hit) ? 'a payment provider frame' : null, submits: false };
    }
    // The control under the point, or the nearest element drawn as clickable
    // (a div with a click handler); plain page background presses nothing.
    let el = hit.closest('button,input,a,select,textarea,summary,label,[role=button],[role=link],[role=menuitem],[onclick]');
    for (let up = hit; !el && up && up !== document.body && up !== document.documentElement; up = up.parentElement) {
      if (getComputedStyle(up).cursor === 'pointer') el = up;
    }
    if (!el) return { line: describe(hit), money: null, submits: false };
    const control = el.tagName === 'LABEL' && el.control ? el.control : el;
    return { line: describe(el), money: moneyRisk(el) || moneyRisk(control), submits: isSubmitter(control) && formRisk(control) === 'submit' };
  };

  // Whether text typed now lands in a payment card field, and what pressing
  // the focused control would do.
  const focusedMoney = () => {
    let el = document.activeElement;
    while (el?.shadowRoot?.activeElement) el = el.shadowRoot.activeElement;
    while (el?.tagName === 'IFRAME') {
      let inner = null;
      try { inner = el.contentDocument?.activeElement; } catch (_) {}
      if (!inner) return { money: payFrame(el) ? 'a payment provider frame' : null, line: describe(el) };
      el = inner;
    }
    if (!el || el === document.body) return { money: null, line: 'page' };
    // Enter or Space on a focused control presses it.
    return { money: el.matches?.(CARD) ? 'payment card field' : null, line: describe(el), press: moneyRisk(el), pressSubmits: isSubmitter(el) && formRisk(el) === 'submit' };
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
    // An open tree folder's box holds its children: its centre is on one of
    // them. Aim at the folder's own label (its first part that is not the
    // nested list).
    const nested = node => node.nodeType === 1 && (/^(UL|OL)$/.test(node.tagName) || node.getAttribute('role') === 'group');
    const head = [...el.children].some(nested) ? [...el.children].find(child => !nested(child) && visible(child)) : null;
    (head || el).scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    let rect = (head || el).getBoundingClientRect();
    if (!head && [...el.children].some(nested)) {
      const text = [...el.childNodes].find(node => node.nodeType === 3 && node.textContent.trim());
      if (text) {
        const range = document.createRange();
        range.selectNodeContents(text);
        rect = range.getBoundingClientRect();
      }
    }
    const [fx, fy] = frameOffset(el);
    const x = Math.round(fx + Math.min(Math.max(rect.left + rect.width / 2, 1), innerWidth - 1));
    const y = Math.round(fy + Math.min(Math.max(rect.top + rect.height / 2, 1), innerHeight - 1));
    const hit = deepHit(x, y);
    const ok = hit && (hit === el || el.contains(hit) || hit.contains(el) || [...(el.labels || [])].some(label => label.contains(hit)));
    // A repeated control says which row it was in, so the reply shows which
    // "Dismiss" was pressed.
    const shown = rows.has(found.healed || id) ? rowOf(el) : '';
    const out = {
      x, y, line: describe(el) + (shown && !describe(el).includes(' in "') ? ` in ${quote(clean(shown, 60))}` : ''), tag: el.tagName, type: el.type || null,
      submits: isSubmitter(el) && formRisk(el) === 'submit',
      disabled: !!(el.disabled || el.getAttribute('aria-disabled') === 'true'),
      editable: /^(INPUT|TEXTAREA)$/.test(el.tagName) || el.isContentEditable,
      money: moneyRisk(el), healed: found.healed || null,
      href: el.tagName === 'A' ? el.href : null,
    };
    if (!ok && !opts.force) {
      out.covered = hit ? describe(hit) : 'nothing at that point';
      // An autocomplete's suggestions, left open over the form, close with
      // Escape; the caller does that and asks again.
      if (hit && hit.closest(SUGGESTIONS) && !el.closest(SUGGESTIONS)) out.coverMenu = true;
    }
    return out;
  };

  // After text goes into a field that suggests (an autocomplete), waits for
  // its suggestion list (scripts show it after a delay, often 300 ms) so the
  // change set has it. Returns whether one is showing.
  const SUGGESTIONS = '.ui-autocomplete,.ui-menu,[role=listbox],.autocomplete-suggestions,.tt-menu,.pac-container,.awesomplete ul,.typeahead.dropdown-menu';
  const awaitSuggestions = async id => {
    const el = els.get(id)?.deref() || activeDeep();
    if (!el || !(el.matches?.('.ui-autocomplete-input,[aria-autocomplete],[role=combobox],[list],[data-autocomplete],.typeahead,.tt-input'))) return { suggests: false };
    for (let waited = 0; waited < 900; waited += 60) {
      if ([...document.querySelectorAll(SUGGESTIONS)].some(menu => visible(menu) && menu.querySelector('li,[role=option],div'))) return { suggests: true, shown: true };
      await new Promise(resolve => setTimeout(resolve, 60));
    }
    return { suggests: true, shown: false };
  };

  // The field that has the keyboard focus, if it takes text.
  const focusedEditable = () => {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    const editable = Boolean(el) && el !== document.body && (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable);
    return { editable, line: editable ? describe(el) : '' };
  };

  const editable = el => !!el && (/^(INPUT|TEXTAREA)$/.test(el.tagName) || el.isContentEditable) && !el.disabled && !el.readOnly;
  const activeDeep = () => {
    let at = document.activeElement;
    while (at && at.shadowRoot && at.shadowRoot.activeElement) at = at.shadowRoot.activeElement;
    return at;
  };
  const focusForInput = async id => {
    const found = lookup(id);
    if (found.error) return found;
    let el = found.el;
    if (el.tagName === 'SELECT') return { kind: 'select' };
    if (el.tagName === 'INPUT' && VALUE_TYPES.test(el.type)) return { kind: 'value', type: el.type };
    if (!(/^(INPUT|TEXTAREA)$/.test(el.tagName) || el.isContentEditable)) return { error: 'not-editable', detail: `${describe(el)} does not accept text` };
    if (el.disabled) return { error: 'disabled', detail: `${describe(el)} is disabled` };
    if (el.readOnly) return { error: 'disabled', detail: `${describe(el)} is read-only: click it to open its picker, then choose the value there` };
    el.focus();
    // Some sites swap a field for a richer one the moment it takes the focus
    // (Wikipedia's search box): the typing goes to the field that has the
    // focus now, and the reference follows it.
    if (!el.isConnected || (activeDeep() !== el && !el.contains(activeDeep()))) {
      for (let i = 0; i < 10 && !editable(activeDeep()); i++) await new Promise(r => setTimeout(r, 30));
      const now = activeDeep();
      if (!el.isConnected && editable(now)) {
        el = now;
        els.set(id, new WeakRef(el));
        ids.set(el, id);
      }
    }
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

  // After typing: a field the site swapped out while it took the focus is
  // followed to the field that holds the focus (and the typed text) now.
  const adoptFocused = id => {
    const el = els.get(id)?.deref();
    const now = activeDeep();
    if (el && !el.isConnected && editable(now)) {
      els.set(id, new WeakRef(now));
      ids.set(now, id);
      prints.set(id, printOf(describe(now)));
    }
    return valueOf(id);
  };

  const choose = (id, wanted) => {
    const found = lookup(id);
    if (found.error) return found;
    const el = found.el;
    if (el.tagName !== 'SELECT') return { error: 'not-select', detail: `${describe(el)} is not a native select` };
    const find = wanted => {
      const want = String(wanted).trim().toLowerCase();
      return [...el.options].find(o => o.value.toLowerCase() === want || o.textContent.trim().toLowerCase() === want) ||
        [...el.options].find(o => o.textContent.trim().toLowerCase().includes(want));
    };
    // Several values for a list that takes several (select multiple): these
    // options and no others, with one input and change event.
    const many = Array.isArray(wanted) ? wanted : null;
    if (many && !el.multiple) return { error: 'invalid', detail: `${describe(el)} takes one option; pass one value` };
    const options = (many || [wanted]).map(find);
    const missing = (many || [wanted]).filter((_, i) => !options[i]);
    if (missing.length) return { error: 'no-option', detail: `no option ${missing.map(quote).join(', ')}; options: ${[...el.options].slice(0, 30).map(o => clean(o.textContent, 30)).join(' | ')}` };
    if (many) for (const o of el.options) o.selected = options.includes(o);
    else el.value = options[0].value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { value: many ? options.map(o => clean(o.textContent, 40)) : clean(options[0].textContent, 60) };
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
        // A space between two inline elements (<b>a</b> <b>b</b>) keeps the words apart.
        if (value.trim() || (value && !pre)) out.push(value.trim() ? value : ' ');
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
      else if (el.type !== 'password' && el.type !== 'file') field.value = exact(el.value ?? el.innerText, 2000);
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
  // `still`: when nothing at all changed for that long after an action, the
  // page is not reacting and the full quiet window would only cost time.
  const settle = (quiet = 250, timeout = 4000, still = 0) => new Promise(resolve => {
    const started = performance.now();
    let last = started, changed = false;
    const observer = new MutationObserver(() => { last = performance.now(); changed = true; });
    observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
    const check = () => {
      const now = performance.now();
      const quietFor = now - last;
      const unmoved = still > 0 && !changed && now - started >= still && document.readyState === 'complete';
      if (unmoved || (quietFor >= quiet && document.readyState === 'complete') || now - started >= timeout) {
        observer.disconnect();
        resolve({ stable: quietFor >= quiet, ms: Math.round(now - started), changed });
      } else setTimeout(check, 50);
    };
    setTimeout(check, Math.min(quiet, still || 60, 60));
  });

  // What scrolls: the page, or when the page cannot (a fixed layout with a
  // scrolled list, a virtual list), the largest visible scrolled box.
  const scrollable = el => el.scrollHeight > el.clientHeight + 8 && /(auto|scroll|overlay)/.test(getComputedStyle(el).overflowY);
  const scroller = (near = null) => {
    for (let el = near; el && el !== document.body && el !== document.documentElement; el = el.parentElement) {
      if (scrollable(el)) return el;
    }
    const root = document.scrollingElement || document.documentElement;
    if (near || root.scrollHeight > innerHeight + 8) return root;
    let best = null, area = 0;
    for (const el of document.querySelectorAll('div,section,main,ul,ol,table,tbody,[role=list],[role=grid],[role=listbox]')) {
      if (!scrollable(el) || !visible(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.width * r.height > area) { best = el; area = r.width * r.height; }
    }
    return best || root;
  };

  const scroll = (opts = {}) => {
    const root = scroller();
    const page = root === (document.scrollingElement || document.documentElement);
    const step = Math.round((page ? innerHeight : root.clientHeight) * 0.85);
    if (opts.to === 'top') root.scrollTo({ top: 0, behavior: 'instant' });
    else if (opts.to === 'bottom') root.scrollTo({ top: root.scrollHeight, behavior: 'instant' });
    else root.scrollBy({ top: opts.dy ?? step * (opts.pages ?? 1), left: opts.dx || 0, behavior: 'instant' });
    return page ? scrollState() : { ...scrollState(), list: { y: Math.round(root.scrollTop), h: root.scrollHeight, vh: root.clientHeight } };
  };

  // Scrolls inside a box (a textarea, a list) to its top or bottom: `ref` is
  // the box or something in it.
  const scrollIn = (opts = {}) => {
    const found = lookup(opts.ref);
    if (found.error) return found;
    const box = scrollable(found.el) || found.el.tagName === 'TEXTAREA' ? found.el : scroller(found.el);
    box.scrollTo({ top: opts.to === 'top' ? 0 : box.scrollHeight, behavior: 'instant' });
    box.dispatchEvent(new Event('scroll'));
    return { box: describe(box).slice(0, 80), y: Math.round(box.scrollTop), h: box.scrollHeight, vh: box.clientHeight };
  };

  // Scrolls (the page, or the list `ref` is in) until `until` is in view,
  // waiting for each part a lazy or virtual list loads: one call instead of
  // a scroll and a read per screen.
  const scrollUntil = async (opts = {}) => {
    const needle = String(opts.until || '').toLowerCase();
    if (!needle) return { error: 'params', detail: 'scroll until needs a text' };
    const near = opts.ref != null ? lookup(opts.ref).el || null : null;
    const root = scroller(near);
    const page = root === (document.scrollingElement || document.documentElement);
    const shown = () => {
      const box = page ? { top: 0, bottom: innerHeight } : root.getBoundingClientRect();
      const walker = document.createTreeWalker(page ? document.body : root, NodeFilter.SHOW_TEXT);
      for (let node; (node = walker.nextNode());) {
        if (!node.nodeValue.toLowerCase().includes(needle)) continue;
        const el = node.parentElement;
        if (!el || !visible(el)) continue;
        const r = el.getBoundingClientRect();
        if (r.bottom > box.top && r.top < box.bottom) return el;
      }
      return null;
    };
    if (opts.from === 'top') root.scrollTo({ top: 0, behavior: 'instant' });
    const steps = Math.min(opts.max || 60, 120);
    for (let i = 0; i <= steps; i++) {
      const hit = shown();
      if (hit) {
        hit.scrollIntoView({ block: 'center', behavior: 'instant' });
        return { found: true, steps: i, scroll: scrollState() };
      }
      const before = [root.scrollTop, root.scrollHeight];
      root.scrollBy({ top: Math.round((page ? innerHeight : root.clientHeight) * 0.85), behavior: 'instant' });
      if (root.scrollTop === before[0]) {
        // At the end: a list that loads more when its end shows gets time to
        // answer (its height grows).
        for (let waited = 0; waited < 2500 && root.scrollHeight === before[1]; waited += 100) await new Promise(r => setTimeout(r, 100));
        if (root.scrollHeight !== before[1]) { await settle(150, 900, 100); continue; }
      } else if (!(await settle(150, 900, 120)).changed) {
        // A virtual list redraws on its scroll event, which a tab that is
        // not painting may not send: send it.
        (page ? window : root).dispatchEvent(new Event('scroll'));
        await settle(100, 500, 80);
      }
      if (root.scrollTop === before[0] && root.scrollHeight === before[1]) {
        const last = shown();
        if (last) { last.scrollIntoView({ block: 'center', behavior: 'instant' }); return { found: true, steps: i + 1, scroll: scrollState() }; }
        return { found: false, steps: i + 1, end: true, scroll: scrollState() };
      }
    }
    return { found: false, steps, scroll: scrollState() };
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
      // A control that becomes usable later (Deploy once the build passed).
      if (ok && opts.enabled != null) {
        const found = lookup(opts.enabled);
        ok = !!found.el && !found.el.disabled && found.el.getAttribute('aria-disabled') !== 'true';
      }
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
  const fillLogin = (username, password, origin) => {
    // The page may have moved on since the sign-in was chosen for its site.
    if (origin && location.origin !== origin) return { error: 'stale', detail: `the page is now on ${location.origin}, not ${origin}` };
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
  // The stage leaves the page a while after the agent's last action, so a
  // page sees nothing of AgentGo while nobody is acting on it.
  let gone = 0;
  const rest = () => {
    clearTimeout(idle);
    clearTimeout(gone);
    idle = setTimeout(() => parts && parts.pointer.animate([{ opacity: .35 }], { duration: 600, fill: 'forwards' }), 2600);
    gone = setTimeout(() => { if (stage) stage.remove(); stage = null; parts = null; }, 8000);
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
    // The frame's own origin: a sign-in typed into another site's embedded
    // frame is that site's, not the page's.
    globalThis.__agReport(JSON.stringify({ kind: 'login', origin: location.origin, username: user.value, password: pass.value }));
  };
  // The Chrome Web Store asks to install the extension of its page
  // (webstore.js, in the page's own world).
  addEventListener('message', event => {
    const data = event.source === window && event.data;
    if (!data || data.__agentgo !== 'ext-install' || location.hostname !== 'chromewebstore.google.com') return;
    if (typeof globalThis.__agReport === 'function' && /^[a-p]{32}$/.test(String(data.id))) {
      globalThis.__agReport(JSON.stringify({ kind: 'extInstall', id: String(data.id) }));
    }
  });
  addEventListener('submit', event => noticeSignIn(event.target), true);
  addEventListener('click', event => {
    const button = event.target.closest && event.target.closest('button,input[type=submit],[role=button]');
    if (button) noticeSignIn(button.form || document);
  }, true);
  addEventListener('keydown', event => {
    if (event.key === 'Enter' && event.target.tagName === 'INPUT') noticeSignIn(event.target.form || document);
  }, true);

  globalThis.__ag = { expire, focusedEditable, adoptFocused, riskAt, focusedMoney, pointerAt: () => at, pointTo, pointAt, keyNote, hideStage, typing, banner, fillLogin, searchResults, selection, find, setBase, element, rect, enterRisk, snapshot, target, focusForInput, choose, valueOf, text, links, tables, forms, settle, scroll, scrollIn, scrollUntil, waitFor, meta, signals, injectionChunks, translateText, translatePage, listenerCandidates, markListened, setValue, holds, awaitSuggestions };
  // Helpers for the page tools (tools.js), which extend this agent.
  globalThis.__ag.h = { lookup, visible, clean, refOf, describe, signals, risk: el => moneyRisk(el) || (isSubmitter(el) && formRisk(el)) || null };
})();
