// "Wire-Man": Pac-Man for the offline page, in AgentGo's live-wire style.
// The maze is drawn as orange wire, the player is ink (white on the dark
// theme, black on the light one), the ghosts are four shades of orange.
// Arrows or WASD move, Space pauses, R restarts.
(() => {
  "use strict";
  const canvas = document.getElementById('game');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const MAZE = [
    '###################',
    '#........#........#',
    '#o##.###.#.###.##o#',
    '#.................#',
    '#.##.#.#####.#.##.#',
    '#....#...#...#....#',
    '####.### # ###.####',
    '   #.#   G   #.#   ',
    '####.# ##-## #.####',
    '    .  #GGG#  .    ',
    '####.# ##### #.####',
    '   #.#       #.#   ',
    '####.# ##### #.####',
    '#........#........#',
    '#.##.###.#.###.##.#',
    '#o.#.....P.....#.o#',
    '##.#.#.#####.#.#.##',
    '#....#...#...#....#',
    '#.######.#.######.#',
    '#.................#',
    '###################',
  ];
  const W = MAZE[0].length, H = MAZE.length;
  const DIRS = { up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] };
  const OPPOSITE = { up: 'down', down: 'up', left: 'right', right: 'left' };
  const DOOR = { x: 9, y: 7 };                    // the tile just above the ghost door
  const $ = id => document.getElementById(id);
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;

  let colors = {};
  const readColors = () => {
    const css = getComputedStyle(document.documentElement);
    const v = name => css.getPropertyValue(name).trim();
    colors = { ink: v('--ink'), ink2: v('--ink-2'), muted: v('--muted'), faint: v('--faint'), surface: v('--surface'), bg: v('--bg-2'),
      accent: v('--accent'), hi: v('--accent-hi'), deep: v('--accent-deep'), hot: v('--hot') };
    colors.ghosts = [colors.accent, colors.hi, colors.deep, colors.hot];
  };

  const wall = (x, y) => { const row = MAZE[y]; return !row || row[x] === '#'; };
  const isDoor = (x, y) => MAZE[y]?.[x] === '-';
  const wrapX = x => (x + W) % W;
  // The door only lets ghosts glide in and out of the house (scripted), so
  // for walking it is a wall.
  const open = (x, y) => { const tx = wrapX(x); if (y < 0 || y >= H) return false; return !isDoor(tx, y) && !wall(tx, y); };
  // corridors: every cell reachable from the start (the house counts too)
  const reachable = new Set();
  { const queue = [[9, 15]]; reachable.add('9,15');
    while (queue.length) { const [x, y] = queue.shift();
      for (const [dx, dy] of Object.values(DIRS)) { const nx = wrapX(x + dx), ny = y + dy, k = `${nx},${ny}`;
        if (!reachable.has(k) && open(nx, ny)) { reachable.add(k); queue.push([nx, ny]); } } }
    MAZE.forEach((row, y) => [...row].forEach((c, x) => { if (c === 'G' || c === '-') reachable.add(`${x},${y}`); })); }

  // ---- state ---------------------------------------------------------------
  let pellets, power, score = 0, lives = 3, level = 1, best = 0, state = 'ready', stateTime = 0, frightUntil = 0, modeClock = 0, eatChain = 0;
  try { best = Number(localStorage.getItem('agentgo-wireman-best')) || 0; } catch (_) {}
  const pac = { x: 9, y: 15, dir: 'left', want: 'left', mouth: 0 };
  let ghosts = [];

  const resetPellets = () => {
    pellets = new Set(); power = new Set();
    MAZE.forEach((row, y) => [...row].forEach((c, x) => { if (c === '.') pellets.add(`${x},${y}`); if (c === 'o') power.add(`${x},${y}`); }));
  };
  const resetActors = () => {
    Object.assign(pac, { x: 9, y: 15, dir: 'left', want: 'left' });
    ghosts = [
      { x: 9, y: 7, dir: 'left', home: false, release: 0, corner: [W - 2, 0], kind: 0 },
      { x: 9, y: 9, dir: 'up', home: true, release: 1.5, corner: [1, 0], kind: 1 },
      { x: 8, y: 9, dir: 'up', home: true, release: 4, corner: [W - 2, H - 1], kind: 2 },
      { x: 10, y: 9, dir: 'up', home: true, release: 7, corner: [1, H - 1], kind: 3 },
    ].map(g => ({ ...g, start: { x: g.x, y: g.y }, eaten: false, exitT: 0 }));
    modeClock = 0; frightUntil = 0;
  };
  const newGame = () => { score = 0; lives = 3; level = 1; resetPellets(); resetActors(); setState('ready'); };
  const setState = s => { state = s; stateTime = 0; hud(); };

  // ---- input ---------------------------------------------------------------
  const KEYS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', w: 'up', s: 'down', a: 'left', d: 'right', W: 'up', S: 'down', A: 'left', D: 'right' };
  addEventListener('keydown', e => {
    if (e.target.closest?.('input,textarea')) return;
    const dir = KEYS[e.key];
    if (dir) {
      e.preventDefault();
      pac.want = dir;
      if (state === 'ready' || state === 'over') { if (state === 'over') newGame(); setState('play'); }
    } else if (e.key === ' ') {
      e.preventDefault();
      if (state === 'play') setState('pause'); else if (state === 'pause' || state === 'ready') setState('play'); else if (state === 'over') { newGame(); setState('play'); }
    } else if (e.key === 'r' || e.key === 'R') { newGame(); }
  });
  canvas.addEventListener('pointerdown', () => { canvas.focus(); if (state === 'ready') setState('play'); });

  // ---- movement --------------------------------------------------------------
  const center = v => Math.abs(v - Math.round(v)) < 1e-6;
  const step = (actor, speed, dt, choose) => {
    let left = speed * dt;
    while (left > 1e-6) {
      const atCenter = center(actor.x) && center(actor.y);
      if (atCenter) {
        actor.x = wrapX(Math.round(actor.x)); actor.y = Math.round(actor.y);
        choose(actor);
        const [dx, dy] = DIRS[actor.dir];
        if (!open(actor.x + dx, actor.y + dy)) return;
      }
      const [dx, dy] = DIRS[actor.dir];
      // distance to the next tile center along the current direction
      const pos = dx ? actor.x : actor.y, sign = dx || dy;
      const next = sign > 0 ? Math.floor(pos + 1e-6) + 1 : Math.ceil(pos - 1e-6) - 1;
      const room = Math.abs(next - pos);
      const move = Math.min(room, left);
      if (dx) actor.x += dx * move; else actor.y += dy * move;
      if (Math.abs(move - room) < 1e-6) { if (dx) actor.x = next; else actor.y = next; }
      left -= move;
      if (actor.x < -0.5) actor.x += W; if (actor.x > W - 0.5) actor.x -= W;
    }
  };

  const choosePac = p => {
    if (open(p.x + DIRS[p.want][0], p.y + DIRS[p.want][1])) p.dir = p.want;
  };
  const target = g => {
    if (g.eaten) return [DOOR.x, DOOR.y];
    const scatter = (modeClock % 27) < 7;
    if (scatter) return g.corner;
    const [dx, dy] = DIRS[pac.dir];
    const px = Math.round(pac.x), py = Math.round(pac.y);
    if (g.kind === 0) return [px, py];
    if (g.kind === 1) return [px + dx * 4, py + dy * 4];
    if (g.kind === 2) { const b = ghosts[0]; return [px + dx * 2 + (px + dx * 2 - Math.round(b.x)), py + dy * 2 + (py + dy * 2 - Math.round(b.y))]; }
    return Math.hypot(g.x - px, g.y - py) > 6 ? [px, py] : g.corner;
  };
  const chooseGhost = g => {
    const options = Object.keys(DIRS).filter(d => d !== OPPOSITE[g.dir] && open(g.x + DIRS[d][0], g.y + DIRS[d][1]));
    if (!options.length) { g.dir = OPPOSITE[g.dir]; return; }
    if (performance.now() < frightUntil && !g.eaten) { g.dir = options[Math.floor(Math.random() * options.length)]; return; }
    const [tx, ty] = target(g);
    options.sort((a, b) => Math.hypot(g.x + DIRS[a][0] - tx, g.y + DIRS[a][1] - ty) - Math.hypot(g.x + DIRS[b][0] - tx, g.y + DIRS[b][1] - ty));
    g.dir = options[0];
  };

  // ---- the game loop --------------------------------------------------------
  let last = performance.now(), elapsed = 0;
  const update = dt => {
    stateTime += dt;
    if (state === 'dying' && stateTime > 1.4) { if (--lives <= 0) { setState('over'); saveBest(); } else { resetActors(); setState('ready'); } return; }
    if (state === 'clear' && stateTime > 1.8) { level++; resetPellets(); resetActors(); setState('ready'); return; }
    if (state !== 'play') return;
    elapsed += dt; modeClock += dt;
    const fast = 1 + (level - 1) * .08;
    step(pac, 7.2 * fast, dt, choosePac);
    pac.mouth += dt * 14;
    const key = `${Math.round(pac.x) % W},${Math.round(pac.y)}`;
    if (pellets.delete(key)) score += 10;
    if (power.delete(key)) { score += 50; frightUntil = performance.now() + Math.max(2500, 7000 - level * 600); eatChain = 0; ghosts.forEach(g => { if (!g.home && !g.eaten) g.dir = OPPOSITE[g.dir]; }); }
    const frightened = performance.now() < frightUntil;
    for (const g of ghosts) {
      if (g.home) {
        g.release -= dt;
        g.y = 9 + Math.sin(elapsed * 6 + g.kind) * .18;
        if (g.release <= 0) { g.home = false; g.exitT = 0; g.exitFrom = { x: g.x, y: 9 }; }
        continue;
      }
      if (g.exitFrom) { // glide out of the house through the door
        g.exitT += dt * 1.8;
        const t = Math.min(1, g.exitT);
        g.x = g.exitFrom.x + (DOOR.x - g.exitFrom.x) * t; g.y = g.exitFrom.y + (DOOR.y - g.exitFrom.y) * t;
        if (t >= 1) { g.exitFrom = null; g.x = DOOR.x; g.y = DOOR.y; g.dir = Math.random() < .5 ? 'left' : 'right'; }
        continue;
      }
      const speed = g.eaten ? 15 : frightened ? 4.2 : (6.6 + level * .15) * (Math.round(g.y) === 9 && (g.x < 5 || g.x > 13) ? .55 : 1);
      step(g, speed, dt, chooseGhost);
      if (g.eaten && Math.round(g.x) === DOOR.x && Math.round(g.y) === DOOR.y && center(g.x) && center(g.y)) {
        g.eaten = false; g.exitFrom = { x: DOOR.x, y: 9 }; g.exitT = 0; g.x = DOOR.x; g.y = 9;
      }
      const dist = Math.hypot(((g.x - pac.x + W * 1.5) % W) - W / 2, g.y - pac.y);
      if (dist < .62 && !g.eaten) {
        if (frightened) { g.eaten = true; eatChain++; score += 200 * 2 ** (eatChain - 1); }
        else { setState('dying'); return; }
      }
    }
    if (!pellets.size && !power.size) { setState('clear'); saveBest(); }
    hud();
  };
  const saveBest = () => { if (score > best) { best = score; try { localStorage.setItem('agentgo-wireman-best', String(best)); } catch (_) {} } hud(); };

  // ---- drawing: pixel art at arcade resolution --------------------------------
  // The board is drawn at 8 pixels per tile and scaled up without smoothing,
  // so every sprite, wire and pellet is made of big, crisp pixels.
  const P = 8;
  let scale = 3;
  const fit = () => {
    const box = canvas.parentElement.getBoundingClientRect();
    scale = Math.max(1, Math.floor(Math.min((box.width - 24) / (W * P), (box.height - 24) / (H * P))));
    canvas.width = W * P; canvas.height = H * P;
    canvas.style.width = `${W * P * scale}px`; canvas.style.height = `${H * P * scale}px`;
    canvas.style.imageRendering = 'pixelated';
    ctx.imageSmoothingEnabled = false;
  };
  const alpha = (hex, a) => {
    const n = hex.replace('#', ''); const v = n.length === 3 ? n.split('').map(c => c + c).join('') : n;
    return `rgba(${parseInt(v.slice(0, 2), 16)},${parseInt(v.slice(2, 4), 16)},${parseInt(v.slice(4, 6), 16)},${a})`;
  };
  const rgb = hex => { const n = hex.replace('#', ''); const v = n.length === 3 ? n.split('').map(c => c + c).join('') : n; return [0, 2, 4].map(i => parseInt(v.slice(i, i + 2), 16)); };
  const mix = (a, b, t) => { const [x, y] = [rgb(a), rgb(b)]; return `rgb(${x.map((c, i) => Math.round(c * t + y[i] * (1 - t))).join(',')})`; };
  const px = (x, y, w = 1, h = 1) => ctx.fillRect(Math.round(x), Math.round(y), w, h);

  // the maze as wire: one line through the middle of every wall cell that
  // borders a corridor, joined to its wall neighbours
  const mazeSegments = () => {
    const segs = [];
    const solid = (x, y) => y >= 0 && y < H && x >= 0 && x < W && wall(x, y);
    const edge = (x, y) => solid(x, y) && [[1,0],[-1,0],[0,1],[0,-1],[1,1],[1,-1],[-1,1],[-1,-1]].some(([dx, dy]) => reachable.has(`${x + dx},${y + dy}`));
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
      if (!edge(x, y)) continue;
      if (edge(x + 1, y)) segs.push([x, y, x + 1, y]);
      if (edge(x, y + 1)) segs.push([x, y, x, y + 1]);
      segs.push([x, y, x, y]);
    }
    return segs;
  };
  let segs = [];
  const wire = (width, offset) => {
    for (const [x1, y1, x2, y2] of segs) {
      const ax = x1 * P + 3 - offset, ay = y1 * P + 3 - offset;
      const bx = x2 * P + 3 - offset, by = y2 * P + 3 - offset;
      px(Math.min(ax, bx), Math.min(ay, by), Math.abs(bx - ax) + width, Math.abs(by - ay) + width);
    }
  };

  // ghost sprite (14 x 14), two skirt frames
  const GHOST = [
    '.....####.....', '...########...', '..##########..', '.############.', '.############.', '.############.',
    '##############', '##############', '##############', '##############', '##############', '##############',
  ];
  const SKIRT = [['##.###..###.##', '#...##..##...#'], ['####.####.####', '.##...##...##.']];
  const ghostRows = frame => GHOST.concat(SKIRT[frame]);
  const outline = rows => {
    const out = [];
    rows.forEach((row, y) => [...row].forEach((c, x) => {
      if (c !== '#') return;
      const edge = [[1, 0], [-1, 0], [0, 1], [0, -1]].some(([dx, dy]) => (rows[y + dy]?.[x + dx] ?? '.') !== '#');
      if (edge) out.push([x, y]);
    }));
    return out;
  };

  const drawGhost = (g, now) => {
    const ox = Math.round(g.x * P + 4 - 7), oy = Math.round(g.y * P + 4 - 7);
    const frame = Math.floor(now / 140) % 2;
    const rows = ghostRows(frame);
    const frightened = now < frightUntil && !g.eaten;
    const ending = frightened && frightUntil - now < 1800 && Math.floor(now / 200) % 2 === 0;
    if (!g.eaten) {
      if (frightened) {
        // hollow pixel outline with a zigzag mouth
        ctx.fillStyle = alpha(ending ? colors.accent : colors.ink, .14);
        rows.forEach((row, y) => [...row].forEach((c, x) => { if (c === '#') px(ox + x, oy + y); }));
        ctx.fillStyle = ending ? colors.accent : colors.ink2;
        for (const [x, y] of outline(rows)) px(ox + x, oy + y);
        px(ox + 4, oy + 5, 2, 2); px(ox + 8, oy + 5, 2, 2);
        for (let i = 0; i < 8; i++) px(ox + 3 + i, oy + 9 + (i % 2));
        return;
      }
      ctx.fillStyle = colors.ghosts[g.kind];
      rows.forEach((row, y) => [...row].forEach((c, x) => { if (c === '#') px(ox + x, oy + y); }));
    }
    // eyes: cream whites, dark pupils looking where it goes
    const [dx, dy] = DIRS[g.dir];
    for (const ex of [2, 8]) {
      ctx.fillStyle = '#FFF8F3';
      px(ox + ex + 1 + dx, oy + 3 + dy, 2, 1); px(ox + ex + dx, oy + 4 + dy, 4, 3); px(ox + ex + 1 + dx, oy + 7 + dy, 2, 1);
      ctx.fillStyle = '#17110C';
      px(ox + ex + 1 + dx * 2, oy + 5 + dy * 2, 2, 2);
    }
  };

  // the player: a pixel disc with a chomping mouth
  const drawPlayer = (cxPix, cyPix, radius, mouth, angle) => {
    ctx.fillStyle = colors.ink;
    const r = radius, size = Math.ceil(r);
    for (let y = -size; y <= size; y++) for (let x = -size; x <= size; x++) {
      if (x * x + y * y > r * r) continue;
      let a = Math.atan2(y, x) - angle;
      a = Math.atan2(Math.sin(a), Math.cos(a));
      if (Math.abs(a) < mouth * Math.PI && x * x + y * y > 1) continue;
      px(cxPix + x, cyPix + y);
    }
  };

  const draw = now => {
    ctx.clearRect(0, 0, W * P, H * P);
    const flash = state === 'clear' && Math.floor(stateTime * 6) % 2 === 0;
    const wireColor = flash ? colors.ink : colors.accent;
    // a stepped pixel glow (an opaque mix, so overlapping pieces do not
    // double up into dots), then the 1-pixel wire
    ctx.fillStyle = mix(wireColor, colors.bg, .26); wire(3, 1);
    ctx.fillStyle = wireColor; wire(1, 0);
    // the ghost door
    ctx.fillStyle = colors.ink2; px(9 * P + 1, 8 * P + 3, P - 2, 1);
    // pellets: 2 x 2; power pellets blink
    ctx.fillStyle = colors.ink2;
    for (const key of pellets) { const [x, y] = key.split(',').map(Number); px(x * P + 3, y * P + 3, 2, 2); }
    if (Math.floor(now / 220) % 2 === 0 || state !== 'play') {
      for (const key of power) {
        const [x, y] = key.split(',').map(Number), cx = x * P + 3.5, cy = y * P + 3.5;
        ctx.fillStyle = colors.accent;
        px(cx - 2, cy - 3, 4, 1); px(cx - 3, cy - 2, 6, 4); px(cx - 2, cy + 2, 4, 1);
      }
    }
    // the player, on both edges while it wraps through the side tunnel
    const copies = x => [x - W, x, x + W].filter(v => v > -1 && v < W);
    const angle = { right: 0, down: Math.PI / 2, left: Math.PI, up: -Math.PI / 2 }[pac.dir];
    let mouth = state === 'play' ? (Math.round((Math.sin(pac.mouth) * .5 + .5) * 3) / 3) * .26 + .02 : .16;
    let radius = 6.5;
    if (state === 'dying') { mouth = Math.min(1, .15 + stateTime * .75); radius = 6.5 * Math.max(0, 1 - stateTime / 1.3); }
    if (radius > .5) for (const x of copies(pac.x)) drawPlayer(Math.round(x * P + 3.5), Math.round(pac.y * P + 3.5), radius, mouth, angle);
    if (state !== 'dying') ghosts.forEach(g => copies(g.x).forEach(x => drawGhost({ ...g, x }, now)));
  };

  const hud = () => {
    $('gScore').textContent = String(score).padStart(5, '0');
    $('gBest').textContent = String(Math.max(best, score)).padStart(5, '0');
    $('gLevel').textContent = level;
    $('gLives').innerHTML = '<i></i>'.repeat(Math.max(0, lives));
    const msg = { ready: 'Press an arrow key to play', pause: 'Paused · Space to continue', over: 'Game over · press an arrow to play again', clear: `Level ${level} cleared` }[state] || '';
    const box = $('gMsg'); box.textContent = msg; box.hidden = !msg;
  };

  const frame = now => {
    const dt = Math.min(.05, (now - last) / 1000); last = now;
    update(dt); draw(now);
    requestAnimationFrame(frame);
  };

  window.__agentgoThemeApplied = () => { readColors(); };
  readColors(); fit(); segs = mazeSegments(); newGame();
  addEventListener('resize', fit);
  if (still) { draw(performance.now()); }
  requestAnimationFrame(frame);
  window.wireman = { state: () => ({ state, score, lives, level, pellets: pellets.size, pac: { ...pac } }) };
})();
