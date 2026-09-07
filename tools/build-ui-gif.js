'use strict';

// Creates the second README motion plate from the UI prototype's "Watch it work"
// idea: a real-looking workspace, cursor movement, click feedback, and verified
// progress. It is deliberately self-contained so the asset can be rebuilt without
// a browser renderer or a third-party package.

const fs = require('fs');
const path = require('path');

const WIDTH = 800;
const HEIGHT = 450;
const FRAMES = 42;
const DELAY = 6;
const OUTPUT = path.join(__dirname, '..', 'assets', 'agentgo-workspace.gif');

function mix(a, b, t) { return Math.round(a + (b - a) * t); }
function ramp(a, b, count) {
  return Array.from({ length: count }, (_, i) => {
    const t = count === 1 ? 0 : i / (count - 1);
    return [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)];
  });
}
const palette = [
  ...ramp([7, 9, 14], [49, 48, 57], 22),
  ...ramp([52, 17, 8], [255, 106, 43], 15),
  ...ramp([80, 84, 96], [244, 240, 232], 13),
  ...ramp([24, 38, 59], [102, 143, 224], 6),
  ...ramp([18, 52, 45], [47, 198, 126], 6),
  ...ramp([13, 17, 25], [24, 28, 38], 2),
];
while (palette.length < 64) palette.push(palette[palette.length - 1]);
const nearestCache = new Map();
function nearest(r, g, b) {
  const key = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
  if (nearestCache.has(key)) return nearestCache.get(key);
  let best = 0;
  let distance = Infinity;
  palette.forEach((p, i) => {
    const dr = r - p[0]; const dg = g - p[1]; const db = b - p[2];
    const d = dr * dr * 1.12 + dg * dg + db * db * .92;
    if (d < distance) { distance = d; best = i; }
  });
  nearestCache.set(key, best);
  return best;
}

function render(frame) {
  const rgb = new Uint8Array(WIDTH * HEIGHT * 3);
  const phase = (frame / FRAMES) * Math.PI * 2;
  for (let y = 0; y < HEIGHT; y += 1) {
    for (let x = 0; x < WIDTH; x += 1) {
      const dx = (x - 400) / 610;
      const dy = (y - 210) / 420;
      const light = Math.max(0, 1 - Math.sqrt(dx * dx + dy * dy)) * 11;
      const orange = Math.max(0, 1 - Math.sqrt(((x - 300) / 470) ** 2 + ((y - 230) / 340) ** 2)) * 5;
      const p = (y * WIDTH + x) * 3;
      rgb[p] = Math.round(8 + light + orange);
      rgb[p + 1] = Math.round(10 + light * .86 + orange * .18);
      rgb[p + 2] = Math.round(15 + light * 1.1 + orange * .15);
    }
  }
  function blend(x, y, color, alpha = 1) {
    if (x < 0 || y < 0 || x >= WIDTH || y >= HEIGHT || alpha <= 0) return;
    const p = (Math.floor(y) * WIDTH + Math.floor(x)) * 3;
    const a = Math.min(1, alpha);
    rgb[p] = Math.round(rgb[p] * (1 - a) + color[0] * a);
    rgb[p + 1] = Math.round(rgb[p + 1] * (1 - a) + color[1] * a);
    rgb[p + 2] = Math.round(rgb[p + 2] * (1 - a) + color[2] * a);
  }
  function fillRect(x, y, w, h, color, alpha = 1) {
    for (let py = Math.max(0, Math.floor(y)); py < Math.min(HEIGHT, Math.ceil(y + h)); py += 1) {
      for (let px = Math.max(0, Math.floor(x)); px < Math.min(WIDTH, Math.ceil(x + w)); px += 1) blend(px, py, color, alpha);
    }
  }
  function fillRound(x, y, w, h, radius, color, alpha = 1) {
    const minX = Math.max(0, Math.floor(x)); const maxX = Math.min(WIDTH, Math.ceil(x + w));
    const minY = Math.max(0, Math.floor(y)); const maxY = Math.min(HEIGHT, Math.ceil(y + h));
    for (let py = minY; py < maxY; py += 1) {
      for (let px = minX; px < maxX; px += 1) {
        const cx = Math.max(x + radius, Math.min(px, x + w - radius));
        const cy = Math.max(y + radius, Math.min(py, y + h - radius));
        if ((px - cx) ** 2 + (py - cy) ** 2 <= radius ** 2) blend(px, py, color, alpha);
      }
    }
  }
  function disc(x, y, radius, color, alpha = 1) {
    for (let py = Math.floor(y - radius - 1); py <= Math.ceil(y + radius + 1); py += 1) {
      for (let px = Math.floor(x - radius - 1); px <= Math.ceil(x + radius + 1); px += 1) {
        const d = Math.sqrt((px - x) ** 2 + (py - y) ** 2);
        if (d <= radius + 1) blend(px, py, color, alpha * Math.max(0, Math.min(1, radius + 1 - d)));
      }
    }
  }
  function line(x1, y1, x2, y2, width, color, alpha = 1) {
    const steps = Math.max(1, Math.ceil(Math.max(Math.abs(x2 - x1), Math.abs(y2 - y1))));
    for (let i = 0; i <= steps; i += 1) {
      const t = i / steps;
      disc(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t, width / 2, color, alpha);
    }
  }
  function polygon(points, color, alpha = 1) {
    const minX = Math.max(0, Math.floor(Math.min(...points.map((p) => p[0]))));
    const maxX = Math.min(WIDTH - 1, Math.ceil(Math.max(...points.map((p) => p[0]))));
    const minY = Math.max(0, Math.floor(Math.min(...points.map((p) => p[1]))));
    const maxY = Math.min(HEIGHT - 1, Math.ceil(Math.max(...points.map((p) => p[1]))));
    for (let py = minY; py <= maxY; py += 1) {
      for (let px = minX; px <= maxX; px += 1) {
        let inside = false;
        for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
          const xi = points[i][0]; const yi = points[i][1];
          const xj = points[j][0]; const yj = points[j][1];
          const crosses = ((yi > py) !== (yj > py)) && (px < ((xj - xi) * (py - yi)) / (yj - yi || 1e-6) + xi);
          if (crosses) inside = !inside;
        }
        if (inside) blend(px, py, color, alpha);
      }
    }
  }
  function textBars(x, y, widths, color, height = 3, gap = 5, alpha = 1) {
    widths.forEach((w, i) => fillRound(x, y + i * (height + gap), w, height, height / 2, color, alpha));
  }
  function button(x, y, w, labelState, active = false) {
    fillRound(x, y, w, 27, 8, active ? [255, 106, 43] : [34, 39, 51], 1);
    fillRound(x + 10, y + 11, labelState === 'BOOK' ? 27 : 26, 3, 2, active ? [255, 239, 225] : [153, 158, 171], .86);
  }
  function cursor(x, y, click = false) {
    // A closed pointer polygon prevents the tip/edge from looking clipped in
    // scaled-down GitHub previews.
    const points = [[x, y], [x, y + 35], [x + 10, y + 26], [x + 20, y + 40], [x + 28, y + 35], [x + 17, y + 22], [x + 32, y + 22]];
    const shadow = points.map(([px, py]) => [px + 2, py + 3]);
    polygon(shadow, [3, 5, 9], .92);
    polygon(points, [250, 249, 245], 1);
    for (let i = 0; i < points.length; i += 1) {
      const a = points[i]; const b = points[(i + 1) % points.length];
      line(a[0], a[1], b[0], b[1], 1.8, [18, 20, 27], .92);
    }
    if (click) {
      const radius = 8 + ((frame % 6) / 6) * 17;
      disc(x + 9, y + 11, radius, [255, 106, 43], .08);
      for (let a = 0; a < Math.PI * 2; a += Math.PI / 28) {
        const px = x + 9 + Math.cos(a) * radius;
        const py = y + 11 + Math.sin(a) * radius;
        disc(px, py, 1.3, [255, 160, 113], .75);
      }
    }
  }
  function pointAt(points, t) {
    const scaled = t * (points.length - 1);
    const i = Math.min(points.length - 2, Math.floor(scaled));
    const local = scaled - i;
    return [mix(points[i][0], points[i + 1][0], local), mix(points[i][1], points[i + 1][1], local)];
  }

  const ink = [244, 240, 232];
  const muted = [143, 149, 164];
  const quiet = [80, 88, 104];
  const panel = [18, 22, 31];
  const panel2 = [24, 29, 40];
  const blue = [98, 144, 231];
  const green = [47, 198, 126];
  const orange = [255, 106, 43];

  // Browser workspace panel, inspired by the UI prototype's Watch it work card.
  fillRound(40, 42, 720, 366, 19, [3, 5, 9], .46);
  fillRound(48, 50, 704, 350, 17, panel, 1);
  line(48, 87, 752, 87, 1, [54, 61, 76], 1);
  disc(70, 68, 4, [58, 64, 78]); disc(84, 68, 4, [58, 64, 78]); disc(98, 68, 4, [58, 64, 78]);
  fillRound(190, 59, 280, 18, 8, panel2, 1);
  fillRound(206, 66, 98, 3, 2, quiet, .7);
  fillRound(620, 64, 88, 5, 2, orange, .75);
  fillRound(62, 103, 408, 273, 13, [14, 18, 26], 1);
  fillRound(494, 103, 242, 273, 13, [22, 27, 37], 1);
  fillRound(77, 118, 378, 23, 7, [30, 36, 48], 1);
  fillRound(90, 128, 134, 3, 2, muted, .63);
  const fieldYs = [155, 187, 219];
  fieldYs.forEach((y, i) => {
    fillRound(77, y, i === 1 ? 184 : 150, 23, 7, panel2, 1);
    disc(89, y + 11.5, 3.2, i === 0 ? orange : blue, .92);
    fillRound(101, y + 10, i === 0 ? 82 : 66, 3, 2, muted, .72);
  });
  const highlight = frame >= 15 && frame < 36;
  const rows = [
    { y: 253, accent: blue, price: [112, 132], book: false },
    { y: 289, accent: orange, price: [93, 118], book: true },
    { y: 325, accent: [168, 118, 238], price: [106, 126], book: false },
  ];
  rows.forEach((row, i) => {
    fillRound(77, row.y, 378, 28, 8, highlight && i === 1 ? [56, 31, 27] : panel2, 1);
    fillRound(88, row.y + 7, 14, 14, 5, row.accent, 1);
    textBars(114, row.y + 8, row.price, quiet, 3, 4, .9);
    fillRound(278, row.y + 10, i === 1 ? 38 : 33, 4, 2, i === 1 ? orange : muted, .86);
    button(390, row.y + 1, 55, row.book ? 'BOOK' : 'VIEW', highlight && i === 1);
  });

  // Agent progress panel.
  fillRound(512, 119, 79, 16, 7, orange, .16);
  fillRound(522, 126, 28, 3, 2, orange, .9);
  fillRound(605, 126, 72, 3, 2, quiet, .75);
  textBars(515, 157, [144, 122, 96], muted, 4, 5, .72);
  const stepsDone = frame < 11 ? 1 : frame < 22 ? 2 : frame < 31 ? 3 : frame < 37 ? 4 : 5;
  for (let i = 0; i < 5; i += 1) {
    const y = 213 + i * 29;
    disc(523, y, 8, i < stepsDone ? [15, 76, 61] : [40, 47, 60], 1);
    if (i < stepsDone) {
      line(519, y, 522, y + 3, 1.8, green, 1);
      line(522, y + 3, 528, y - 4, 1.8, green, 1);
    } else {
      fillRound(520, y - 1, 6, 2, 1, quiet, .65);
    }
    fillRound(542, y - 2, 112 - i * 10, 4, 2, i < stepsDone ? ink : muted, i < stepsDone ? .86 : .48);
  }
  fillRound(515, 357, 205, 3, 2, [43, 50, 63], 1);
  fillRound(515, 357, Math.min(205, 38 + stepsDone * 33), 3, 2, orange, .9);

  // Cursor choreography: move to the cheapest result, click, then verify the state.
  const route = [[118, 170], [170, 178], [332, 247], [352, 296], [492, 325], [611, 184]];
  let cursorT;
  if (frame < 13) cursorT = frame / 13 * .56;
  else if (frame < 20) cursorT = .56;
  else if (frame < 34) cursorT = .56 + (frame - 20) / 14 * .44;
  else cursorT = 1;
  const [cursorX, cursorY] = pointAt(route, cursorT);
  cursor(cursorX, cursorY, frame >= 13 && frame <= 19);

  const indexed = new Uint8Array(WIDTH * HEIGHT);
  for (let i = 0, p = 0; i < rgb.length; i += 3, p += 1) indexed[p] = nearest(rgb[i], rgb[i + 1], rgb[i + 2]);
  return indexed;
}

function lzwEncode(indices, minimumCodeSize) {
  const clear = 1 << minimumCodeSize;
  const end = clear + 1;
  let next = end + 1;
  let codeSize = minimumCodeSize + 1;
  let dictionary = new Map();
  let buffer = 0;
  let bits = 0;
  const bytes = [];
  function reset() { dictionary = new Map(); next = end + 1; codeSize = minimumCodeSize + 1; }
  function emit(code) {
    buffer |= code << bits;
    bits += codeSize;
    while (bits >= 8) { bytes.push(buffer & 255); buffer >>>= 8; bits -= 8; }
  }
  reset(); emit(clear);
  let prefix = indices[0];
  for (let i = 1; i < indices.length; i += 1) {
    const value = indices[i];
    const key = `${prefix}:${value}`;
    if (dictionary.has(key)) { prefix = dictionary.get(key); continue; }
    emit(prefix);
    if (next < 4096) {
      dictionary.set(key, next++);
      if (next > (1 << codeSize) && codeSize < 12) codeSize += 1;
    } else { emit(clear); reset(); }
    prefix = value;
  }
  emit(prefix); emit(end);
  if (bits > 0) bytes.push(buffer & 255);
  return Buffer.from(bytes);
}
function u16(n) { return Buffer.from([n & 255, (n >> 8) & 255]); }
function blocks(data) {
  const out = [];
  for (let i = 0; i < data.length; i += 255) { const part = data.subarray(i, Math.min(i + 255, data.length)); out.push(Buffer.from([part.length]), part); }
  out.push(Buffer.from([0]));
  return Buffer.concat(out);
}

const output = [Buffer.from('GIF89a', 'ascii')];
const screen = Buffer.alloc(7);
screen.writeUInt16LE(WIDTH, 0); screen.writeUInt16LE(HEIGHT, 2); screen[4] = 0xf5;
output.push(screen);
const table = Buffer.alloc(64 * 3);
palette.forEach((c, i) => { table[i * 3] = c[0]; table[i * 3 + 1] = c[1]; table[i * 3 + 2] = c[2]; });
output.push(table);
output.push(Buffer.from([0x21, 0xff, 0x0b]), Buffer.from('NETSCAPE2.0', 'ascii'), Buffer.from([0x03, 0x01, 0x00, 0x00, 0x00]));
for (let frame = 0; frame < FRAMES; frame += 1) {
  output.push(Buffer.from([0x21, 0xf9, 0x04, 0x08]), u16(DELAY), Buffer.from([0, 0]));
  const descriptor = Buffer.alloc(10);
  descriptor[0] = 0x2c; descriptor.writeUInt16LE(0, 1); descriptor.writeUInt16LE(0, 3); descriptor.writeUInt16LE(WIDTH, 5); descriptor.writeUInt16LE(HEIGHT, 7);
  output.push(descriptor, Buffer.from([6]), blocks(lzwEncode(render(frame), 6)));
}
output.push(Buffer.from([0x3b]));
fs.writeFileSync(OUTPUT, Buffer.concat(output));
console.log(`Wrote ${OUTPUT} (${fs.statSync(OUTPUT).size} bytes)`);
