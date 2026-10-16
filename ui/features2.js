/* Website dark mode card, site permission row, and the permission request box.
   Loaded after tools.js and features.js; uses their globals. */

/* ---------- dark mode for websites (sits right under Theme) ---------- */
const SD_STEPS = {
  siteDarkBrightness: { min: 70, max: 130, label: 'Brightness', low: 'Dimmer', high: 'Brighter' },
  siteDarkContrast: { min: 70, max: 130, label: 'Contrast', low: 'Softer', high: 'Sharper' },
};
const sdValue = (settings, key) => {
  const n = Number(settings[key]);
  return Number.isFinite(n) && n >= 50 && n <= 150 ? Math.round(n) : 100;
};

const siteDarkCard = settings => {
  const on = settings.siteDark === true;
  const excluded = settings.siteDarkExclude || [];
  const slider = key => {
    const spec = SD_STEPS[key];
    const value = sdValue(settings, key);
    return `<label class="sd-slider"><span class="sd-sl-top"><span class="sd-sl-name">${spec.label}</span><output data-sd-out="${key}">${value}%</output></span>
      <input type="range" min="${spec.min}" max="${spec.max}" step="5" value="${value}" data-sd-range="${key}" aria-label="${spec.label}" style="--p:${((value - spec.min) / (spec.max - spec.min)) * 100}%">
      <span class="sd-sl-ends"><span>${spec.low}</span><span>${spec.high}</span></span></label>`;
  };
  return `<div class="setting sd${on ? ' on' : ''}" data-sd-card>
    <div class="sd-preview" aria-hidden="true">
      <div class="sd-page"><i class="sd-bar"></i><i class="sd-line w1"></i><i class="sd-line w2"></i><i class="sd-img"></i><i class="sd-line w3"></i></div>
      <svg class="sd-orb" viewBox="0 0 24 24"><circle class="sun" cx="12" cy="12" r="4.2"/><path class="rays" d="M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.3 5.3l1.5 1.5M17.2 17.2l1.5 1.5M5.3 18.7l1.5-1.5M17.2 6.8l1.5-1.5"/><path class="moon" d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/></svg>
    </div>
    <div class="sd-copy"><b>Dark mode for websites</b>
      <span>Turns light pages dark while you read. This is separate from the Theme above, which only changes AgentGo itself.</span></div>
    <button class="switch${on ? ' on' : ''}" type="button" role="switch" aria-checked="${on}" aria-label="Dark mode for websites" data-sd="toggle"></button>
    <div class="sd-more"><div class="sd-more-in">
      <div class="sd-sliders">${slider('siteDarkBrightness')}${slider('siteDarkContrast')}</div>
      <div class="sd-skip"><span class="sd-skip-title">Left as they are</span>
        <div class="sd-chips">${excluded.length
          ? excluded.map(host => `<button type="button" class="sd-chip" data-sd-unskip="${esc(host)}" title="Dark mode on again for ${esc(host)}"><span>${esc(host)}</span><svg viewBox="0 0 24 24"><path d="m7 7 10 10M17 7 7 17"/></svg></button>`).join('')
          : '<span class="sd-none">No sites. Press Ctrl+K on a page and choose “Dark mode: toggle for this site”.</span>'}</div></div>
    </div></div>
  </div>`;
};

let sdTimer = 0;
const sdSave = patch => settingsPatch(patch).then(() => { Object.assign(settingsCache, patch); });

toolBody.addEventListener('click', async event => {
  const toggle = event.target.closest('[data-sd="toggle"]');
  if (toggle) {
    const next = toggle.getAttribute('aria-checked') !== 'true';
    toggle.setAttribute('aria-checked', String(next));
    toggle.classList.toggle('on', next);
    toggle.classList.remove('pop'); void toggle.offsetWidth; toggle.classList.add('pop');
    toggle.closest('[data-sd-card]').classList.toggle('on', next);
    await sdSave({ siteDark: next });
    return;
  }
  const chip = event.target.closest('[data-sd-unskip]');
  if (chip) {
    const host = chip.dataset.sdUnskip;
    const left = (settingsCache.siteDarkExclude || []).filter(h => h !== host);
    chip.classList.add('gone');
    await sdSave({ siteDarkExclude: left });
    setTimeout(() => {
      chip.remove();
      const box = toolBody.querySelector('.sd-chips');
      if (box && !box.children.length) box.innerHTML = '<span class="sd-none">No sites. Press Ctrl+K on a page and choose “Dark mode: toggle for this site”.</span>';
    }, 180);
  }
});

toolBody.addEventListener('input', event => {
  const range = event.target.closest('[data-sd-range]');
  if (!range) return;
  const key = range.dataset.sdRange;
  const spec = SD_STEPS[key];
  range.style.setProperty('--p', `${((range.value - spec.min) / (spec.max - spec.min)) * 100}%`);
  toolBody.querySelector(`[data-sd-out="${key}"]`).textContent = `${range.value}%`;
  clearTimeout(sdTimer);
  sdTimer = setTimeout(() => sdSave({ [key]: Number(range.value) }), 160);
});

/* ---------- site permissions row ---------- */
const PERM_LABEL = { camera: 'Camera', microphone: 'Microphone', geolocation: 'Location', notifications: 'Notifications', clipboard: 'Clipboard', midi: 'MIDI', storage: 'Storage access', fonts: 'Local fonts', 'window-management': 'Window management', sensors: 'Motion sensors', 'idle-detection': 'Idle detection', 'pointer-lock': 'Pointer lock', 'keyboard-lock': 'Keyboard lock', downloads: 'Automatic downloads' };
const permLabel = kind => PERM_LABEL[kind] || kind;

const permissionRow = () => {
  queueMicrotask(loadPermList);
  return `<div class="setting perm"><div><b>Site permissions</b>
      <span>Camera, microphone, location and notifications are blocked for every site. When a site asks, a box lets you allow it once or for good. Sites you allow are listed here.</span>
      <div class="perm-list" data-perm-list><span class="perm-none">Loading…</span></div></div>
      <span class="value">Blocked by default</span></div>`;
};

const loadPermList = async () => {
  const box = toolBody.querySelector('[data-perm-list]');
  if (!box) return;
  let sites = {};
  try { sites = (await callBridge('ui.permission_list')).sites || {}; } catch { /* leave empty */ }
  const origins = Object.keys(sites);
  box.innerHTML = origins.length
    ? origins.map(origin => `<div class="perm-site"><span class="perm-host">${esc(origin.replace(/^https?:\/\//, ''))}</span>
        <span class="perm-kinds">${sites[origin].map(k => `<i>${esc(permLabel(k))}</i>`).join('')}</span>
        <button type="button" class="btn sm" data-perm-revoke="${esc(origin)}">Block again</button></div>`).join('')
    : '<span class="perm-none">No site is allowed right now.</span>';
};

toolBody.addEventListener('click', async event => {
  const button = event.target.closest('[data-perm-revoke]');
  if (!button) return;
  button.disabled = true;
  await callBridge('ui.permission_revoke', { origin: button.dataset.permRevoke }).catch(() => {});
  loadPermList();
});

/* ---------- the permission request box ---------- */
const PERM_ICON = {
  camera: 'M3 7.5A2.5 2.5 0 0 1 5.5 5h8A2.5 2.5 0 0 1 16 7.5v9a2.5 2.5 0 0 1-2.5 2.5h-8A2.5 2.5 0 0 1 3 16.5zM16 10l5-3v10l-5-3',
  microphone: 'M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM5 11a7 7 0 0 0 14 0M12 18v3',
  geolocation: 'M12 21s-7-6.2-7-11.2A7 7 0 0 1 19 9.8C19 14.8 12 21 12 21zM12 12.5a2.6 2.6 0 1 0 0-5.2 2.6 2.6 0 0 0 0 5.2z',
  notifications: 'M6 17V11a6 6 0 0 1 12 0v6l1.5 2h-15zM10 21h4',
  other: 'M12 3l8 3v6c0 4.5-3.4 8-8 9-4.6-1-8-4.5-8-9V6z',
};

const permSentence = kinds => {
  const names = kinds.map(k => permLabel(k).toLowerCase());
  if (names.length <= 1) return names[0] || 'something';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
};

let permCurrent = null;
const renderPermissionNow = () => {
  const data = permCurrent;
  if (!data) { window.__agentgoRequestClose(); return; }
  const host = String(data.origin || '').replace(/^https?:\/\//, '');
  const secure = String(data.origin || '').startsWith('https://');
  const icons = (data.kinds.length ? data.kinds : ['other']).map(k => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${PERM_ICON[k] || PERM_ICON.other}"/></svg>`).join('');
  toolBody.innerHTML = `<div class="pq" data-id="${data.id}">
    <div class="pq-head"><span class="pq-icons">${icons}</span>
      <div class="pq-title"><strong>${esc(host)}</strong><span>wants to use your ${esc(permSentence(data.kinds))}</span></div></div>
    <p class="pq-note">${secure ? '' : 'This site is not using a secure connection. '}It is blocked unless you allow it. Allowing applies to this site only.${data.more ? ` ${data.more} more request${data.more === 1 ? '' : 's'} waiting.` : ''}</p>
    <div class="pq-actions">
      <button type="button" class="btn pq-block" data-pq="block">Keep blocked</button>
      <button type="button" class="btn" data-pq="once">Allow this time</button>
      <button type="button" class="btn primary" data-pq="always">Always allow</button></div>
    <i class="pq-timer" aria-hidden="true"></i></div>`;
};

const renderPermission = async () => {
  permCurrent = shellData().data || null;
  renderPermissionNow();
};

toolBody.addEventListener('click', async event => {
  const button = event.target.closest('[data-pq]');
  if (!button || activeTool !== 'permission' || !permCurrent) return;
  button.closest('.pq').classList.add('answered');
  let next = null;
  try { next = (await callBridge('ui.permission_answer', { id: permCurrent.id, choice: button.dataset.pq })).next; } catch { /* closes below */ }
  permCurrent = next && next.id ? next : null;
  if (permCurrent) renderPermissionNow(); else window.__agentgoRequestClose();
});

/* ---------- translation: language picker, page panel, selection card ---------- */
const LANG_CODES = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'ar', 'fa', 'he', 'tr', 'hi', 'bn', 'ur', 'zh-CN', 'zh-TW', 'ja', 'ko', 'vi', 'th', 'id', 'ms', 'nl', 'pl', 'uk', 'cs', 'sv', 'da', 'fi', 'no', 'el', 'hu', 'ro', 'bg', 'sr', 'hr', 'sk', 'sl', 'lt', 'lv', 'et', 'ca', 'fil', 'sw', 'af', 'ta', 'te', 'mr', 'gu', 'kn', 'ml', 'pa', 'ne', 'si', 'my', 'km'];
const uiLocale = () => navigator.language || 'en';
const langName = (code, locale = uiLocale()) => {
  try { return new Intl.DisplayNames([locale], { type: 'language' }).of(code) || code; } catch { return code; }
};
const defaultTarget = () => {
  const guess = uiLocale();
  return LANG_CODES.includes(guess) ? guess : guess.split('-')[0];
};
const targetLang = async () => {
  const settings = await callBridge('settings.get').catch(() => ({}));
  return settings.translateTo || defaultTarget();
};

const pickerHtml = (current, mode) => `<div class="lp" data-lp="${mode}">
  <div class="lp-top"><button type="button" class="lp-back" data-lp-back aria-label="Back"><svg viewBox="0 0 24 24"><path d="m14 6-6 6 6 6"/></svg></button><strong>Translate to</strong></div>
  <input class="lp-search" type="text" placeholder="Search languages" autocomplete="off" spellcheck="false" aria-label="Search languages">
  <div class="lp-list">${LANG_CODES.map((code, i) => {
    const name = langName(code);
    const native = langName(code, code);
    return `<button type="button" class="lp-row${code === current ? ' on' : ''}" data-lp-pick="${code}" data-q="${esc((name + ' ' + native + ' ' + code).toLowerCase())}" style="--r:${Math.min(i, 10)}"><span>${esc(name)}</span><em>${native !== name ? esc(native) : ''}</em><svg viewBox="0 0 24 24"><path d="m6 12.5 4 4 8-9"/></svg></button>`;
  }).join('')}</div></div>`;

const wirePicker = () => {
  const search = toolBody.querySelector('.lp-search');
  if (!search) return;
  search.focus({ preventScroll: true });
  const list = toolBody.querySelector('.lp-list');
  const on = list.querySelector('.lp-row.on');
  if (on) list.scrollTop = Math.max(0, on.offsetTop - list.clientHeight / 2 + on.offsetHeight / 2);
  toolBody.scrollTop = 0;
  search.addEventListener('input', () => {
    const q = search.value.trim().toLowerCase();
    toolBody.querySelectorAll('.lp-row').forEach(row => { row.hidden = Boolean(q) && !row.dataset.q.includes(q); });
  });
};

/* the whole-page panel */
const trp = { data: null, lang: 'en', phase: 'idle', picking: false, error: '' };

const paintTranslate = () => {
  if (activeTool !== 'translate') return;
  if (trp.picking) {
    toolBody.innerHTML = pickerHtml(trp.lang, 'page');
    wirePicker();
    return;
  }
  const name = langName(trp.lang);
  let title, sub;
  if (trp.phase === 'working') { title = `Translating to ${name}`; sub = 'Reading the page and sending its text to Google Translate…'; }
  else if (trp.phase === 'done') { title = `Translated to ${name}`; sub = 'The page is translated in place. Nothing else on it changed.'; }
  else if (trp.phase === 'same') { title = `This page is already in ${name}`; sub = 'Pick another language to translate it into.'; }
  else if (trp.phase === 'original') { title = 'Showing the original'; sub = 'The page is back in its own language.'; }
  else { title = 'Could not translate'; sub = trp.error || 'The translation service did not answer.'; }
  toolBody.innerHTML = `<div class="pq trp${trp.phase === 'working' ? ' busy' : ''}">
    <div class="pq-head"><span class="pq-icons"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h8M8 3v2M6 5c0 4 3 7 6 8M10 5c0 3-2 6-6 8M13 20l4-9 4 9M14.5 17h5"/></svg></span>
      <div class="pq-title"><strong>${esc(title)}</strong><span>${esc(sub)}</span></div></div>
    <div class="trp-bar">${trp.phase === 'working' ? '<i></i><i></i><i></i>' : ''}</div>
    <button type="button" class="trp-lang" data-trp="pick"><span class="trp-k">Translate to</span><span class="trp-v">${esc(name)}</span><svg viewBox="0 0 24 24"><path d="m7 10 5 5 5-5"/></svg></button>
    <div class="pq-actions">
      ${trp.phase === 'done' ? '<button type="button" class="btn" data-trp="original">Show original</button>' : ''}
      ${trp.phase === 'original' || trp.phase === 'error' ? '<button type="button" class="btn" data-trp="again">Translate again</button>' : ''}
      <button type="button" class="btn primary" data-trp="close">Done</button></div></div>`;
};

const runPageTranslate = async (restore = false) => {
  trp.phase = restore ? 'original' : 'working';
  paintTranslate();
  try {
    const out = await callBridge('browser_ui.translate', { to: trp.lang, force: !restore, restore });
    if (!restore) trp.phase = out.result?.skipped ? 'same' : 'done';
  } catch (error) {
    trp.phase = 'error';
    trp.error = error.message;
  }
  paintTranslate();
};

const renderTranslate = async () => {
  const data = shellData();
  if (trp.data === data) { paintTranslate(); return; }
  trp.data = data;
  trp.picking = false;
  trp.lang = await targetLang();
  runPageTranslate();
};

/* clicks for pickers and the page panel */
toolBody.addEventListener('click', async event => {
  const pick = event.target.closest('[data-lp-pick]');
  const back = event.target.closest('[data-lp-back]');
  const trpButton = event.target.closest('[data-trp]');
  const selPick = event.target.closest('[data-tr-pick]');
  if (event.target.closest('[data-tr-close]')) { window.__agentgoRequestClose(); return; }

  if (selPick) {
    toolBody.dataset.keep = toolBody.innerHTML;
    toolBody.innerHTML = pickerHtml(selPick.dataset.trPick, 'sel');
    wirePicker();
    return;
  }
  if (back) {
    if (trp.picking && activeTool === 'translate') { trp.picking = false; paintTranslate(); }
    else if (toolBody.dataset.keep) { toolBody.innerHTML = toolBody.dataset.keep; delete toolBody.dataset.keep; }
    return;
  }
  if (pick) {
    const code = pick.dataset.lpPick;
    await settingsPatch({ translateTo: code });
    if (pick.closest('.lp').dataset.lp === 'page') {
      trp.lang = code; trp.picking = false;
      runPageTranslate();
    } else {
      const original = toolBody.dataset.original || '';
      delete toolBody.dataset.keep;
      await callBridge('ui.context_action', { action: 'translate', selection: original, to: code });
    }
    return;
  }
  if (trpButton && activeTool === 'translate') {
    const what = trpButton.dataset.trp;
    if (what === 'pick') { trp.picking = true; paintTranslate(); }
    else if (what === 'original') runPageTranslate(true);
    else if (what === 'again') runPageTranslate();
    else if (what === 'close') window.__agentgoRequestClose();
  }
});

/* primary translation language in Settings */
const translateRow = settings => {
  const current = settings.translateTo || defaultTarget();
  const options = LANG_CODES.map(code => `<option value="${code}"${code === current ? ' selected' : ''}>${esc(langName(code))}</option>`).join('');
  return `<div class="setting"><div><b>Translate into</b><span>The language page and selection translation use first. You can still switch it on the translation box.</span></div>
    <select class="btn" data-translate-to aria-label="Translate into">${options}</select></div>`;
};
toolBody.addEventListener('change', event => {
  if (event.target.matches('[data-translate-to]')) settingsPatch({ translateTo: event.target.value });
});