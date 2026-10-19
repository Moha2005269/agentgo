// The theme switch, shared by the toolbar, the panels and the New Tab page.
// The browser tells all three at the same moment, each with the click point
// in its own coordinates and the same radius, so the three surfaces play one
// continuous circle across the window. An ember ring rides its edge.
// Cheap on purpose (4K screens): the circle is a clip on the new snapshot,
// and the ring is a few strokes on a canvas, both on the same curve.
(() => {
  const still = matchMedia('(prefers-reduced-motion: reduce)');
  const resolve = setting => setting === 'light' || (setting === 'system' && matchMedia('(prefers-color-scheme: light)').matches) ? 'light' : 'dark';
  const DURATION = 700;
  const CURVE = [.3, 0, .15, 1];          // quick to leave, soft to land
  let running = null;
  let heading = null;               // the theme a running switch is going to

  // cubic-bezier(x1, y1, x2, y2) as a function of time, same as CSS
  const bezier = ([x1, y1, x2, y2]) => {
    const at = (a, b, t) => ((1 - 3 * b + 3 * a) * t + (3 * b - 6 * a)) * t * t + 3 * a * t;
    const slope = (a, b, t) => 3 * (1 - 3 * b + 3 * a) * t * t + 2 * (3 * b - 6 * a) * t + 3 * a;
    return x => {
      let t = x;
      for (let i = 0; i < 6; i++) { const d = slope(x1, x2, t); if (Math.abs(d) < 1e-6) break; t -= (at(x1, x2, t) - x) / d; }
      return at(y1, y2, Math.min(1, Math.max(0, t)));
    };
  };
  const ease = bezier(CURVE);

  const ring = (x, y, r) => {
    const canvas = document.createElement('canvas');
    const dpr = Math.min(2, devicePixelRatio || 1);
    canvas.width = innerWidth * dpr; canvas.height = innerHeight * dpr;
    canvas.style.cssText = 'position:fixed;inset:0;width:100vw;height:100vh;pointer-events:none;z-index:2147483647';
    document.body.appendChild(canvas);
    const ctx = canvas.getContext('2d');
    const css = getComputedStyle(document.documentElement);
    const accent = css.getPropertyValue('--accent').trim(), hi = css.getPropertyValue('--accent-hi').trim();
    const start = performance.now();
    const frame = now => {
      const t = Math.min(1, (now - start) / DURATION), radius = Math.max(0, r * ease(t) - 3);
      const fade = t < .75 ? 1 : 1 - (t - .75) / .25;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, innerWidth, innerHeight);
      if (radius > 1) {
        for (const [width, alpha, color] of [[26, .07, accent], [12, .16, accent], [4, .55, hi], [1.6, 1, hi]]) {
          ctx.globalAlpha = alpha * fade; ctx.lineWidth = width; ctx.strokeStyle = color;
          ctx.beginPath(); ctx.arc(x, y, radius, 0, Math.PI * 2); ctx.stroke();
        }
      }
      if (t < 1) requestAnimationFrame(frame); else canvas.remove();
    };
    requestAnimationFrame(frame);
  };

  // setting: "dark" | "light" | "system"; origin: {x, y, r} or none (no animation)
  window.agentgoSetTheme = (setting, origin) => {
    const root = document.documentElement;
    const theme = resolve(setting);
    const first = !root.dataset.themed;
    root.dataset.themed = '1';
    // A switch already on its way to this theme owns the change: applying
    // it here would flip this surface before the circle reaches it.
    if (running && heading === theme) return;
    if (root.dataset.theme === theme) return;
    const apply = () => { root.dataset.theme = theme; window.__agentgoThemeApplied?.(theme); };
    if (first || !origin || !document.startViewTransition || still.matches) { apply(); return; }
    running?.skipTransition();
    document.dispatchEvent(new CustomEvent('agentgo:themeswitch', { detail: DURATION }));
    const transition = document.startViewTransition(apply);
    running = transition;
    heading = theme;
    transition.ready.then(() => {
      const { x, y, r } = origin;
      // Percentages of the surface, not pixels: in a freshly opened page the
      // snapshot can be measured in device pixels, which shrank and moved the
      // circle by the screen scale (it then stalled and jumped to the end).
      const w = innerWidth, h = innerHeight;
      const px = (x / w) * 100, py = (y / h) * 100, pr = (r / (Math.hypot(w, h) / Math.SQRT2)) * 100;
      root.animate(
        { clipPath: [`circle(0% at ${px}% ${py}%)`, `circle(${pr}% at ${px}% ${py}%)`] },
        { duration: DURATION, easing: `cubic-bezier(${CURVE.join(',')})`, pseudoElement: '::view-transition-new(root)', fill: 'both' },
      );
      ring(x, y, r);
    }).catch(() => {});
    transition.finished.finally(() => { if (running === transition) running = null; });
  };

  // The accent colour (Settings > Appearance). The default ember keeps the
  // hand-tuned palettes in theme.css; any other colour gets a matching set of
  // shades for both themes.
  const DEFAULT_ACCENT = '#ff6a2b';
  const mix = (rgb, to, amount) => rgb.map((c, i) => Math.round(c + (to[i] - c) * amount));
  const hex = rgb => '#' + rgb.map(c => c.toString(16).padStart(2, '0')).join('');
  const rgba = (rgb, a) => `rgba(${rgb.join(', ')}, ${a})`;
  window.agentgoSetAccent = value => {
    const color = String(value || '').trim().toLowerCase();
    let style = document.getElementById('ag-accent');
    if (!/^#[0-9a-f]{6}$/.test(color) || color === DEFAULT_ACCENT) { style?.remove(); return; }
    const rgb = [1, 3, 5].map(i => parseInt(color.slice(i, i + 2), 16));
    const bright = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255 > 0.6;
    const ink = bright ? '#141010' : '#fffaf6';
    const css = `:root{--accent:${color};--accent-hi:${hex(mix(rgb, [255, 255, 255], 0.25))};--accent-deep:${hex(mix(rgb, [0, 0, 0], 0.18))};--accent-ink:${ink};--glow:${rgba(rgb, 0.55)};--glow-soft:${rgba(rgb, 0.16)}}`
      + `html[data-theme="light"]{--accent:${hex(mix(rgb, [0, 0, 0], 0.12))};--accent-hi:${color};--accent-deep:${hex(mix(rgb, [0, 0, 0], 0.3))};--accent-ink:${ink};--glow:${rgba(rgb, 0.38)};--glow-soft:${rgba(rgb, 0.1)}}`;
    if (!style) { style = document.createElement('style'); style.id = 'ag-accent'; document.head.appendChild(style); }
    if (style.textContent !== css) style.textContent = css;
  };

  window.__agentgoThemeSync = ({ theme, accent, x, y, r }) => {
    if (accent !== undefined) window.agentgoSetAccent(accent);
    window.agentgoSetTheme(theme, { x, y, r });
  };
})();
