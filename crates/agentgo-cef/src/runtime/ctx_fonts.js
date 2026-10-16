(() => {
  const sel = getSelection();
  let el = (sel && sel.rangeCount && sel.toString().trim()) ? sel.getRangeAt(0).commonAncestorContainer : document.elementFromPoint(__X__, __Y__);
  if (el && el.nodeType === 3) el = el.parentElement;
  el = el || document.body;
  const s = getComputedStyle(el);
  const families = s.fontFamily.split(',').map(f => f.trim().replace(/^["']|["']$/g, ''));
  const loaded = [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family.replace(/^["']|["']$/g, ''));
  return {
    element: el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.classList[0] ? '.' + el.classList[0] : ''),
    families, webFontsLoaded: [...new Set(loaded)].slice(0, 12),
    size: s.fontSize, weight: s.fontWeight, style: s.fontStyle, lineHeight: s.lineHeight,
    letterSpacing: s.letterSpacing, color: s.color, transform: s.textTransform,
    sample: (el.innerText || '').trim().slice(0, 60)
  };
})()
