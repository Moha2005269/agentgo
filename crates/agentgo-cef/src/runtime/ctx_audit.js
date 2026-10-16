(() => {
  const out = [];
  const add = (id, impact, help, nodes) => {
    if (nodes.length) out.push({ id, impact, help, count: nodes.length, examples: nodes.slice(0, 4).map(n => (n.outerHTML || String(n)).replace(/\s+/g, ' ').slice(0, 110)) });
  };
  const all = sel => [...document.querySelectorAll(sel)];
  const shown = el => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const name = el => (el.getAttribute('aria-label') || el.getAttribute('aria-labelledby') || el.getAttribute('title') ||
    (el.innerText || el.textContent || '').trim() || el.querySelector('img[alt]:not([alt=""])') || '');
  add('image-alt', 'critical', 'Images need alternative text', all('img:not([alt]):not([role=presentation])').filter(shown));
  add('button-name', 'critical', 'Buttons need a name', all('button,[role=button]').filter(shown).filter(b => !name(b) && !b.value));
  add('link-name', 'serious', 'Links need a name', all('a[href]').filter(shown).filter(a => !name(a)));
  add('label', 'critical', 'Form fields need a label', all('input:not([type=hidden]):not([type=submit]):not([type=button]):not([type=image]),select,textarea')
    .filter(shown).filter(f => !(f.labels && f.labels.length) && !f.getAttribute('aria-label') && !f.getAttribute('aria-labelledby') && !f.title));
  add('html-has-lang', 'serious', 'The page needs a language', document.documentElement.getAttribute('lang') ? [] : [document.documentElement]);
  add('document-title', 'serious', 'The page needs a title', document.title.trim() ? [] : [document.documentElement]);
  add('frame-title', 'serious', 'Frames need a title', all('iframe:not([title])').filter(shown));
  add('empty-heading', 'minor', 'Headings should not be empty', all('h1,h2,h3,h4,h5,h6').filter(h => !h.textContent.trim()));
  const heads = all('h1,h2,h3,h4,h5,h6').filter(shown);
  const skips = [];
  heads.forEach((h, i) => {
    const level = +h.tagName[1];
    const prev = i ? +heads[i - 1].tagName[1] : 0;
    if (level - prev > 1) skips.push(h);
  });
  add('heading-order', 'moderate', 'Heading levels should go up one at a time', skips);
  add('page-has-heading-one', 'moderate', 'The page needs a main heading', all('h1').length ? [] : [document.body]);
  const ids = {};
  all('[id]').forEach(e => { (ids[e.id] = ids[e.id] || []).push(e); });
  add('duplicate-id', 'minor', 'IDs must be unique', Object.values(ids).filter(l => l.length > 1).map(l => l[0]));
  add('tabindex', 'serious', 'Avoid tabindex above 0', all('[tabindex]').filter(e => +e.getAttribute('tabindex') > 0));
  add('meta-viewport', 'critical', 'Zooming must not be disabled', all('meta[name=viewport]').filter(m => /user-scalable\s*=\s*(no|0)|maximum-scale\s*=\s*1(\.0)?\b/i.test(m.content)));
  const lum = c => {
    const [r, g, b] = c.map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const rgba = s => {
    const m = s.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  };
  const backdrop = el => {
    for (let n = el; n; n = n.parentElement) {
      const c = rgba(getComputedStyle(n).backgroundColor);
      if (c && c[3] > 0.95) return c;
    }
    return [255, 255, 255, 1];
  };
  const low = [];
  for (const el of all('p,span,a,li,td,th,label,button,h1,h2,h3,h4,h5,h6').slice(0, 1500)) {
    if (![...el.childNodes].some(n => n.nodeType === 3 && n.textContent.trim().length > 1) || !shown(el)) continue;
    const s = getComputedStyle(el);
    const fg = rgba(s.color);
    if (!fg || fg[3] < 1) continue;
    const a = lum(fg), b = lum(backdrop(el));
    const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    const big = parseFloat(s.fontSize) >= 24 || (parseFloat(s.fontSize) >= 18.66 && +s.fontWeight >= 700);
    if (ratio < (big ? 3 : 4.5)) low.push(el);
  }
  add('color-contrast', 'serious', 'Text needs enough contrast with its background', low);
  const weight = { critical: 4, serious: 3, moderate: 2, minor: 1 };
  out.sort((a, b) => weight[b.impact] - weight[a.impact]);
  return { url: location.href, violations: out, total: out.reduce((n, v) => n + v.count, 0) };
})()
