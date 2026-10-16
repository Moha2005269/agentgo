// "How it sees": a looping film of an agent reading and using a page.
// Every change is a step on one timeline, so pausing, looping and jumping to a
// scene are exact. It only plays while it is on screen.
(() => {
  const film = document.getElementById('film');
  if (!film) return;
  const $f = selector => film.querySelector(selector);
  const page = $f('#fmPage');
  const url = $f('#fmUrl');
  const live = $f('#fmLive');
  const lines = $f('#fmLines');
  const tok = $f('#fmTok');
  const email = $f('#fmEmail');
  const pass = $f('#fmPass');
  const ad = $f('#fmAd');
  const cookie = $f('#fmCookie');
  const ask = $f('#fmAsk');
  const agent = $f('#fmAgent');
  const you = $f('#fmYou');
  const chips = $f('#fmChips');

  const SCENES = [['Read', 0], ['Act', 5000], ['Ask you', 11000], ['Done', 15000]];
  const END = 20500;
  const steps = [];
  const at = (time, run) => steps.push([time, run]);
  let fast = false;

  const line = (kind, html) => {
    const row = document.createElement('div');
    row.className = `fm-ln ${kind}`;
    row.innerHTML = html;
    lines.append(row);
    while (lines.childElementCount > 24) lines.firstElementChild.remove();
  };
  const say = (time, kind, html) => at(time, () => line(kind, html));
  const typeInto = (el, text, time, per, mask) => {
    for (let i = 0; i < text.length; i++) at(time + i * per, () => { el.textContent = mask ? '•'.repeat(i + 1) : text.slice(0, i + 1); });
  };
  const place = (cursor, x, y, ms) => {
    cursor.style.transition = ms ? `transform ${ms}ms cubic-bezier(.65,.05,.36,1), opacity .35s` : 'opacity .35s';
    cursor.style.transform = `translate3d(${x}px,${y}px,0)`;
    cursor.dataset.x = x;
    cursor.dataset.y = y;
  };
  const move = (cursor, selector, time, ms, fx = 0.5, fy = 0.6) => at(time, () => {
    const target = $f(selector);
    if (!target) return;
    const box = page.getBoundingClientRect();
    const r = target.getBoundingClientRect();
    place(cursor, r.left - box.left + r.width * fx, r.top - box.top + r.height * fy, ms);
  });
  const click = (cursor, time, pressed) => {
    at(time, () => {
      cursor.classList.add('down');
      if (pressed) $f(pressed).classList.add('down');
      if (fast) return;
      const ring = document.createElement('span');
      ring.className = `fm-ripple${cursor === you ? ' you' : ''}`;
      ring.style.left = `${cursor.dataset.x}px`;
      ring.style.top = `${cursor.dataset.y}px`;
      ring.addEventListener('animationend', () => ring.remove());
      page.append(ring);
    });
    at(time + 170, () => {
      cursor.classList.remove('down');
      if (pressed) $f(pressed).classList.remove('down');
    });
  };
  const focus = el => {
    film.querySelectorAll('.fm-input.focus').forEach(input => input.classList.remove('focus'));
    if (el) el.classList.add('focus');
  };
  const refs = (list, time, gap) => list.forEach((n, i) => at(time + i * gap, () => $f(`[data-ref="${n}"]`).classList.add('ref')));
  const tokens = (time, to) => {
    const frames = 16;
    for (let i = 1; i <= frames; i++) {
      at(time + i * 55, () => {
        const value = Math.round(to * (1 - (1 - i / frames) ** 3));
        tok.innerHTML = `<s>41,872 raw HTML</s><b>${value}</b> tokens`;
      });
    }
  };

  // 01 Read: the page loads, junk is removed, and the agent gets a short outline.
  typeInto(url, 'northwind.shop/checkout', 150, 26);
  at(850, () => film.classList.add('loaded'));
  at(1150, () => cookie.classList.remove('gone'));
  say(1300, 'c', '<i>›</i>observe.page');
  at(1450, () => film.classList.add('scan'));
  at(1700, () => ad.classList.add('hit'));
  say(1750, 'm', '# ad blocked before it loaded');
  at(2150, () => cookie.classList.add('gone'));
  say(2200, 'm', '# cookie banner dismissed');
  at(2350, () => ad.classList.add('blocked'));
  at(2950, () => film.classList.remove('scan'));
  refs(['1', '2', '3', '4'], 2650, 230);
  say(2700, 'o', '# Sign in to check out');
  say(2930, 'o', '[1] textbox(email) <s>"Email"</s> required');
  say(3160, 'o', '[2] textbox(password) <s>"Password"</s> required');
  say(3390, 'o', '[3] checkbox <s>"Keep me signed in"</s>');
  say(3620, 'o', '[4] button <s>"Continue"</s>');
  tokens(3700, 186);
  say(4500, 'm', '# 186 tokens, not 41,872 of raw HTML');

  // 02 Act: fill, tick, click. Each action answers with only what changed.
  at(5000, () => { place(agent, 330, 330, 0); agent.classList.add('show'); });
  say(5100, 'c', '<i>›</i>act.fill {ref: 1, text: "ada@lovelace.dev"}');
  move(agent, '#fmEmail', 5200, 700, 0.12);
  click(agent, 5950);
  at(5980, () => focus(email));
  typeInto(email, 'ada@lovelace.dev', 6100, 45);
  say(6900, 'd', '<i>~</i>[1] textbox(email) value=<s>"ada@lovelace.dev"</s>');
  say(7100, 'c', '<i>›</i>act.fill {ref: 2, secret: true}');
  move(agent, '#fmPass', 7150, 520, 0.12);
  click(agent, 7720);
  at(7750, () => focus(pass));
  typeInto(pass, 'correct-horse', 7850, 40, true);
  say(8450, 'd', '<i>~</i>[2] value=•••••••• <em># never logged</em>');
  move(agent, '[data-ref="3"] i', 8600, 450);
  click(agent, 9080);
  at(9100, () => { focus(null); $f('[data-ref="3"]').classList.add('on'); });
  say(9180, 'd', '<i>~</i>[3] checkbox checked');
  say(9380, 'c', '<i>›</i>act.click {ref: 4}');
  move(agent, '.fm-form .fm-btn', 9420, 500);
  click(agent, 9950, '.fm-form .fm-btn');
  at(10300, () => {
    film.querySelectorAll('.ref').forEach(el => el.classList.remove('ref'));
    film.classList.add('pay');
    url.textContent = 'northwind.shop/checkout/pay';
  });
  say(10350, 'd', '<i>~</i>navigated → /checkout/pay');
  refs(['5'], 10650, 0);
  say(10700, 'o', '[5] button <s>"Pay $129.00"</s>');

  // 03 Ask you: payments stop for the person, in every mode.
  say(11050, 'c', '<i>›</i>act.click {ref: 5}');
  move(agent, '.fm-pay .fm-btn', 11100, 620);
  click(agent, 11750, '.fm-pay .fm-btn');
  at(12050, () => { film.classList.add('ask', 'wait'); live.textContent = 'Waiting for you'; });
  say(12100, 'w', '<i>!</i>paused · payment needs your approval');
  at(12550, () => { place(you, 40, 330, 0); you.classList.add('show'); });
  move(you, '.fm-ok', 12650, 950, 0.55, 0.65);
  click(you, 13700, '.fm-ok');
  at(13950, () => ask.classList.add('approved'));
  say(14000, 'ok', '<i>✓</i>approved by you · resuming');
  at(14700, () => {
    film.classList.remove('ask', 'wait');
    live.textContent = 'Agent · Claude';
    you.classList.remove('show');
  });

  // 04 Done.
  at(15100, () => { film.classList.add('done'); url.textContent = 'northwind.shop/order/A-20471'; });
  at(15300, () => agent.classList.remove('show'));
  say(15400, 'd', '<i>~</i>heading <s>"Order placed"</s> #A-20471');
  say(16100, 'm', '# 9 actions · 0 raw HTML · 1 approval');
  say(16800, 'ok', '<i>✓</i>task complete');
  at(END - 700, () => film.classList.add('outro'));

  steps.sort((a, b) => a[0] - b[0]);

  chips.innerHTML = SCENES.map(([name], i) => `<button class="fm-chip" type="button" data-scene="${i}"><span><b>0${i + 1}</b>${name}</span><u><i></i></u></button>`).join('');
  const bars = [...chips.querySelectorAll('.fm-chip')];

  const reset = () => {
    film.classList.remove('loaded', 'scan', 'pay', 'ask', 'wait', 'done', 'outro');
    film.querySelectorAll('.ref,.on,.down,.focus,.show').forEach(el => el.classList.remove('ref', 'on', 'down', 'focus', 'show'));
    film.querySelectorAll('.fm-ripple').forEach(el => el.remove());
    ad.classList.remove('hit', 'blocked');
    ask.classList.remove('approved');
    cookie.classList.add('gone');
    lines.textContent = '';
    email.textContent = '';
    pass.textContent = '';
    url.textContent = '';
    tok.textContent = '—';
    live.textContent = 'Agent · Claude';
  };

  let start = 0;
  let elapsed = 0;
  let next = 0;
  let playing = false;
  let frame = 0;

  const paint = () => {
    bars.forEach((bar, i) => {
      const from = SCENES[i][1];
      const to = i + 1 < SCENES.length ? SCENES[i + 1][1] : END;
      const p = Math.min(1, Math.max(0, (elapsed - from) / (to - from)));
      bar.classList.toggle('on', elapsed >= from && elapsed < to);
      bar.lastElementChild.firstElementChild.style.transform = `scaleX(${p})`;
    });
  };

  const seek = ms => {
    reset();
    film.classList.add('instant');
    fast = true;
    next = 0;
    while (next < steps.length && steps[next][0] < ms) steps[next++][1]();
    fast = false;
    void film.offsetWidth;
    requestAnimationFrame(() => film.classList.remove('instant'));
    elapsed = ms;
    start = performance.now() - ms;
    paint();
  };

  const tick = now => {
    elapsed = now - start;
    while (next < steps.length && steps[next][0] <= elapsed) steps[next++][1]();
    if (elapsed >= END) seek(0);
    paint();
    frame = requestAnimationFrame(tick);
  };

  const play = () => {
    if (playing) return;
    playing = true;
    start = performance.now() - elapsed;
    frame = requestAnimationFrame(tick);
  };
  const pause = () => {
    playing = false;
    cancelAnimationFrame(frame);
  };

  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let held = still;
  let onScreen = false;
  const update = () => {
    film.classList.toggle('paused', held);
    $f('#fmPlay').setAttribute('aria-label', held ? 'Play' : 'Pause');
    if (!held && onScreen && !document.hidden) play();
    else pause();
  };

  $f('#fmPlay').addEventListener('click', () => { held = !held; update(); });
  chips.addEventListener('click', event => {
    const chip = event.target.closest('[data-scene]');
    if (!chip) return;
    pause();
    seek(SCENES[Number(chip.dataset.scene)][1]);
    update();
  });
  document.addEventListener('visibilitychange', update);
  new IntersectionObserver(([entry]) => { onScreen = entry.isIntersecting; update(); }, { root: document.getElementById('viewport'), threshold: 0.25 }).observe(film);

  seek(still ? 4900 : 0);
  update();
})();
