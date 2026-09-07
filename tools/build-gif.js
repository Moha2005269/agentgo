'use strict';

// Dependency-free renderer for the README motion plate.
// It intentionally draws continuous feather strokes rather than particle clouds,
// so the exported GIF stays crisp, small, and faithful to the AgentGo mark.

const fs = require('fs');
const path = require('path');

// Match the vector source's 16:9 canvas so the README GIF keeps the same
// proportions and breathing room as the HTML/SVG version.
const WIDTH = 960;
const HEIGHT = 540;
const FRAMES = 30;
const DELAY = 7; // 70ms per frame
const OUTPUT = path.join(__dirname, '..', 'assets', 'agentgo-loop.gif');

function mix(a, b, t) { return Math.round(a + (b - a) * t); }
function ramp(from, to, count) {
  const result = [];
  for (let i = 0; i < count; i += 1) {
    const t = count === 1 ? 0 : i / (count - 1);
    result.push([mix(from[0], to[0], t), mix(from[1], to[1], t), mix(from[2], to[2], t)]);
  }
  return result;
}

const palette = [
  ...ramp([7, 9, 14], [48, 45, 51], 24),
  ...ramp([65, 22, 9], [255, 106, 43], 16),
  ...ramp([88, 91, 101], [244, 240, 232], 12),
  ...ramp([28, 41, 59], [107, 143, 204], 6),
  ...ramp([21, 53, 48], [49, 194, 128], 4),
  ...ramp([5, 7, 11], [18, 20, 27], 2),
];
while (palette.length < 64) palette.push(palette[palette.length - 1]);

const paletteCache = new Map();
function nearestPaletteIndex(r, g, b) {
  const key = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
  const cached = paletteCache.get(key);
  if (cached !== undefined) return cached;
  let best = 0;
  let bestDistance = Infinity;
  for (let i = 0; i < palette.length; i += 1) {
    const p = palette[i];
    const dr = r - p[0];
    const dg = g - p[1];
    const db = b - p[2];
    const distance = dr * dr * 1.12 + dg * dg + db * db * 0.92;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = i;
    }
  }
  paletteCache.set(key, best);
  return best;
}

function frameBuffer(phase) {
  const rgb = new Uint8Array(WIDTH * HEIGHT * 3);
  const centerX = WIDTH * 0.5;
  const centerY = HEIGHT * 0.42;
  for (let y = 0; y < HEIGHT; y += 1) {
    for (let x = 0; x < WIDTH; x += 1) {
      const dx = (x - centerX) / WIDTH;
      const dy = (y - centerY) / HEIGHT;
      const radial = Math.max(0, 1 - Math.sqrt(dx * dx + dy * dy) * 2.2);
      const warm = Math.max(0, 1 - Math.sqrt(((x - centerX) / (WIDTH * 0.58)) ** 2 + ((y - HEIGHT * 0.46) / (HEIGHT * 0.72)) ** 2));
      const i = (y * WIDTH + x) * 3;
      rgb[i] = Math.round(9 + radial * 17 + warm * 8);
      rgb[i + 1] = Math.round(11 + radial * 14 + warm * 3);
      rgb[i + 2] = Math.round(16 + radial * 15 + warm * 4);
    }
  }

  function blendPixel(x, y, r, g, b, alpha) {
    if (x < 0 || y < 0 || x >= WIDTH || y >= HEIGHT || alpha <= 0) return;
    const i = (Math.floor(y) * WIDTH + Math.floor(x)) * 3;
    const a = Math.min(1, alpha);
    rgb[i] = Math.round(rgb[i] * (1 - a) + r * a);
    rgb[i + 1] = Math.round(rgb[i + 1] * (1 - a) + g * a);
    rgb[i + 2] = Math.round(rgb[i + 2] * (1 - a) + b * a);
  }

  function disc(x, y, radius, color, alpha = 1) {
    const minX = Math.floor(x - radius - 1);
    const maxX = Math.ceil(x + radius + 1);
    const minY = Math.floor(y - radius - 1);
    const maxY = Math.ceil(y + radius + 1);
    const radiusSquared = radius * radius;
    for (let py = minY; py <= maxY; py += 1) {
      for (let px = minX; px <= maxX; px += 1) {
        const distanceSquared = (px - x) ** 2 + (py - y) ** 2;
        if (distanceSquared <= radiusSquared + 1) {
          const edge = Math.max(0, Math.min(1, radius + 1 - Math.sqrt(distanceSquared)));
          blendPixel(px, py, color[0], color[1], color[2], alpha * edge);
        }
      }
    }
  }

  function line(x1, y1, x2, y2, width, color, alpha = 1) {
    const length = Math.max(Math.abs(x2 - x1), Math.abs(y2 - y1));
    const steps = Math.max(1, Math.ceil(length * 0.9));
    const radius = width * 0.5;
    for (let i = 0; i <= steps; i += 1) {
      const t = i / steps;
      disc(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t, radius, color, alpha);
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
          const [xi, yi] = points[i];
          const [xj, yj] = points[j];
          const crosses = ((yi > py) !== (yj > py)) && (px < ((xj - xi) * (py - yi)) / (yj - yi || 1e-6) + xi);
          if (crosses) inside = !inside;
        }
        if (inside) blendPixel(px, py, color[0], color[1], color[2], alpha);
      }
    }
  }

  function curve(points, width, color, alpha = 1) {
    const [p0, p1, p2, p3] = points;
    const samples = 42;
    let previous = p0;
    for (let i = 1; i <= samples; i += 1) {
      const t = i / samples;
      const mt = 1 - t;
      const point = [
        mt ** 3 * p0[0] + 3 * mt ** 2 * t * p1[0] + 3 * mt * t ** 2 * p2[0] + t ** 3 * p3[0],
        mt ** 3 * p0[1] + 3 * mt ** 2 * t * p1[1] + 3 * mt * t ** 2 * p2[1] + t ** 3 * p3[1],
      ];
      line(previous[0], previous[1], point[0], point[1], width, color, alpha);
      previous = point;
    }
  }

  function ellipse(cx, cy, rx, ry, width, color, alpha = 1, start = 0, end = Math.PI * 2) {
    const samples = Math.ceil(Math.abs(end - start) * Math.max(rx, ry) * 0.42);
    let previous = [cx + Math.cos(start) * rx, cy + Math.sin(start) * ry];
    for (let i = 1; i <= samples; i += 1) {
      const t = start + (end - start) * (i / samples);
      const point = [cx + Math.cos(t) * rx, cy + Math.sin(t) * ry];
      line(previous[0], previous[1], point[0], point[1], width, color, alpha);
      previous = point;
    }
  }

  const cx = centerX;
  const wingY = Math.min(HEIGHT * 0.40, 470) + Math.sin(phase) * 1.8;
  const wingSize = Math.min(WIDTH * 0.40, 540);
  const orange = [255, 106, 43];
  const warmWhite = [244, 240, 232];
  const smoke = [115, 116, 126];

  // The HTML backdrop uses one broad, quiet halo above a filled feather form.
  const haloY = Math.max(104, wingY - wingSize * 0.44);
  const haloRX = wingSize * 0.30;
  const haloRY = haloRX * 0.26;
  ellipse(cx, haloY, haloRX, haloRY, 12, orange, 0.05);
  ellipse(cx, haloY, haloRX, haloRY, 3, orange, 0.9);
  const sweep = -Math.PI * 0.08 + ((phase / (Math.PI * 2)) % 1) * Math.PI * 2;
  ellipse(cx, haloY, haloRX, haloRY, 3.2, warmWhite, 0.8, sweep, sweep + 0.48);

  function feather(x, y, angle, length, width, bend, color, alpha) {
    const endX = x + Math.cos(angle) * length;
    const endY = y + Math.sin(angle) * length;
    const controlX = x + Math.cos(angle) * length * 0.55 - Math.sin(angle) * bend * length * 0.35;
    const controlY = y + Math.sin(angle) * length * 0.55 + Math.cos(angle) * bend * length * 0.35;
    const left = [];
    const right = [];
    const samples = 14;
    for (let i = 0; i <= samples; i += 1) {
      const t = i / samples;
      const mt = 1 - t;
      const qx = mt * mt * x + 2 * mt * t * controlX + t * t * endX;
      const qy = mt * mt * y + 2 * mt * t * controlY + t * t * endY;
      const tx = 2 * mt * (controlX - x) + 2 * t * (endX - controlX);
      const ty = 2 * mt * (controlY - y) + 2 * t * (endY - controlY);
      const tangent = Math.atan2(ty, tx);
      const normalX = -Math.sin(tangent);
      const normalY = Math.cos(tangent);
      const taper = Math.sin(Math.PI * t) * 0.78 + 0.22;
      const halfWidth = width * taper;
      left.push([qx + normalX * halfWidth, qy + normalY * halfWidth]);
      right.push([qx - normalX * halfWidth, qy - normalY * halfWidth]);
    }
    polygon([...left, ...right.reverse()], color, alpha);
    const light = color[0] > 200 && color[1] < 180 ? [255, 187, 142] : [255, 255, 255];
    const innerLeft = left.map(([px, py]) => [x + (px - x) * 0.66, y + (py - y) * 0.66]);
    const innerRight = right.map(([px, py]) => [x + (px - x) * 0.66, y + (py - y) * 0.66]);
    polygon([...innerLeft, ...innerRight.reverse()], light, alpha * 0.18);
    curve([[x, y], [controlX, controlY], [controlX, controlY], [endX, endY]], 1.5, [255, 255, 255], alpha * 0.34);
  }

  function drawWing(side, featherColor, secondaryColor) {
    const R = (angle) => side > 0 ? angle : Math.PI - angle;
    const rx = cx + side * 26;
    const ry = wingY;
    const sway = Math.sin(phase + side * 0.55) * 2.5;
    const topAngle = R(-0.62);
    const topX = rx + Math.cos(topAngle) * wingSize * 0.98;
    const topY = ry + Math.sin(topAngle) * wingSize * 0.98;
    const shoulder = [
      [rx, ry - wingSize * 0.05],
      [rx + Math.cos(R(-0.15)) * wingSize * 0.55, ry + Math.sin(R(-0.15)) * wingSize * 0.55 - wingSize * 0.145],
      [topX, topY + sway],
      [rx + Math.cos(R(-0.15)) * wingSize * 0.55, ry + Math.sin(R(-0.15)) * wingSize * 0.55 + wingSize * 0.075],
      [rx, ry + wingSize * 0.075],
    ];
    polygon(shoulder, featherColor, 0.12);

    const layers = [
      { n: 10, a0: -0.66, a1: 1.06, l0: 1.08, l1: 0.60, w: 0.090, al: 0.28, bend: 0.24, off: 0.00 },
      { n: 12, a0: -0.62, a1: 1.02, l0: 1.00, l1: 0.52, w: 0.080, al: 0.54, bend: 0.22, off: 0.02 },
      { n: 11, a0: -0.50, a1: 1.06, l0: 0.82, l1: 0.44, w: 0.090, al: 0.68, bend: 0.20, off: 0.04 },
      { n: 10, a0: -0.34, a1: 1.12, l0: 0.62, l1: 0.35, w: 0.100, al: 0.82, bend: 0.17, off: 0.06 },
      { n: 9, a0: -0.16, a1: 1.20, l0: 0.42, l1: 0.26, w: 0.115, al: 0.92, bend: 0.14, off: 0.09 },
      { n: 7, a0: -0.02, a1: 1.28, l0: 0.24, l1: 0.16, w: 0.145, al: 1.00, bend: 0.11, off: 0.12 },
    ];
    layers.forEach((layer, layerIndex) => {
      for (let i = 0; i < layer.n; i += 1) {
        const t = layer.n === 1 ? 0 : i / (layer.n - 1);
        const jitter = (Math.sin((i + 1) * 12.9898 + layerIndex * 78.233) * 0.5 + 0.5 - 0.5) * 0.045;
        const angle = R(layer.a0 + (layer.a1 - layer.a0) * t + jitter);
        const length = wingSize * (layer.l0 + (layer.l1 - layer.l0) * t) * (0.94 + 0.10 * (Math.sin(i * 4.17 + layerIndex) * 0.5 + 0.5));
        const offsetX = Math.cos(R(-0.5)) * wingSize * layer.off;
        const offsetY = Math.sin(R(-0.5)) * wingSize * layer.off;
        feather(rx + offsetX, ry + offsetY, angle, length, length * layer.w, layer.bend * side, featherColor, layer.al);
      }
    });
    // Dark seams and a low contour keep the feather layers legible at README scale.
    for (let f = 0; f < 4; f += 1) {
      curve([
        [rx + side * (44 + f * 9), ry + 19 + f * 5],
        [cx + side * (112 + f * 18), ry + 28 + f * 4],
        [cx + side * (220 + f * 20), ry + 45 + f * 4],
        [cx + side * (326 - f * 9), ry + 35 + f * 7],
      ], 2.4, [25, 24, 30], 0.36);
    }
    curve([[rx, ry + 18], [cx + side * 104, ry + 70], [cx + side * 220, ry + 66], [cx + side * 330, ry + 37]], 2.2, secondaryColor, 0.46);
  }

  drawWing(-1, orange, [222, 126, 87]);
  drawWing(1, warmWhite, smoke);

  // Mark: drawn on top of the wings and under the halo, so the visual hierarchy stays clear.
  line(cx - 48, 270, cx - 12, 178, 7, warmWhite, 1);
  line(cx - 12, 178, cx + 42, 270, 7, warmWhite, 1);
  line(cx - 35, 240, cx + 17, 240, 5.5, warmWhite, 0.98);
  line(cx - 53, 257, cx + 45, 200, 6, orange, 1);
  line(cx + 21, 201, cx + 45, 200, 5.5, orange, 1);
  line(cx + 45, 200, cx + 44, 220, 5.5, orange, 1);
  // The emblem itself keeps the small, broken halo from the HTML source.
  ellipse(cx, 155, 31, 8, 4.5, orange, 0.96, 0.18, Math.PI * 1.90);
  // Sparse atmospheric points are background texture only; the wing itself has no particle breaks.
  for (let i = 0; i < 14; i += 1) {
    const x = (i * 137 + 49) % WIDTH;
    const y = (i * 71 + 32) % 365;
    const twinkle = 0.12 + 0.08 * (0.5 + 0.5 * Math.sin(phase * 1.4 + i));
    disc(x, y, i % 3 === 0 ? 1.8 : 1.1, i % 3 === 0 ? orange : smoke, twinkle);
  }

  const indexed = new Uint8Array(WIDTH * HEIGHT);
  for (let i = 0, p = 0; i < rgb.length; i += 3, p += 1) {
    indexed[p] = nearestPaletteIndex(rgb[i], rgb[i + 1], rgb[i + 2]);
  }
  return indexed;
}

function lzwEncode(indices, minimumCodeSize) {
  const clearCode = 1 << minimumCodeSize;
  const endCode = clearCode + 1;
  let nextCode = endCode + 1;
  let codeSize = minimumCodeSize + 1;
  let dictionary = new Map();
  let bitBuffer = 0;
  let bitCount = 0;
  const bytes = [];

  function reset() {
    dictionary = new Map();
    nextCode = endCode + 1;
    codeSize = minimumCodeSize + 1;
  }
  function emit(code) {
    bitBuffer |= code << bitCount;
    bitCount += codeSize;
    while (bitCount >= 8) {
      bytes.push(bitBuffer & 255);
      bitBuffer >>>= 8;
      bitCount -= 8;
    }
  }

  reset();
  emit(clearCode);
  let prefix = indices[0];
  for (let i = 1; i < indices.length; i += 1) {
    const value = indices[i];
    const key = `${prefix}:${value}`;
    if (dictionary.has(key)) {
      prefix = dictionary.get(key);
      continue;
    }
    emit(prefix);
    if (nextCode < 4096) {
      dictionary.set(key, nextCode);
      nextCode += 1;
      // The decoder installs the new table entry after consuming the next
      // code, so keep the wider code size for the following emission.
      if (nextCode > (1 << codeSize) && codeSize < 12) codeSize += 1;
    } else {
      emit(clearCode);
      reset();
    }
    prefix = value;
  }
  emit(prefix);
  emit(endCode);
  if (bitCount > 0) bytes.push(bitBuffer & 255);
  return Buffer.from(bytes);
}

function u16(value) { return Buffer.from([value & 255, (value >> 8) & 255]); }
function block(data) {
  const chunks = [];
  for (let i = 0; i < data.length; i += 255) {
    const part = data.subarray(i, Math.min(i + 255, data.length));
    chunks.push(Buffer.from([part.length]), part);
  }
  chunks.push(Buffer.from([0]));
  return Buffer.concat(chunks);
}

const chunks = [Buffer.from('GIF89a', 'ascii')];
const screen = Buffer.alloc(7);
screen.writeUInt16LE(WIDTH, 0);
screen.writeUInt16LE(HEIGHT, 2);
screen[4] = 0xf5; // global table, 8-bit color resolution, 64-color table
screen[5] = 0;
screen[6] = 0;
chunks.push(screen);
const table = Buffer.alloc(64 * 3);
palette.forEach((color, i) => {
  table[i * 3] = color[0];
  table[i * 3 + 1] = color[1];
  table[i * 3 + 2] = color[2];
});
chunks.push(table);
chunks.push(Buffer.from([0x21, 0xff, 0x0b]), Buffer.from('NETSCAPE2.0', 'ascii'), Buffer.from([0x03, 0x01, 0x00, 0x00, 0x00]));

for (let frame = 0; frame < FRAMES; frame += 1) {
  const phase = (frame / FRAMES) * Math.PI * 2;
  const pixels = frameBuffer(phase);
  chunks.push(Buffer.from([0x21, 0xf9, 0x04, 0x08]), u16(DELAY), Buffer.from([0x00, 0x00]));
  const descriptor = Buffer.alloc(10);
  descriptor[0] = 0x2c;
  descriptor.writeUInt16LE(0, 1);
  descriptor.writeUInt16LE(0, 3);
  descriptor.writeUInt16LE(WIDTH, 5);
  descriptor.writeUInt16LE(HEIGHT, 7);
  descriptor[9] = 0;
  chunks.push(descriptor);
  chunks.push(Buffer.from([6]), block(lzwEncode(pixels, 6)));
}
chunks.push(Buffer.from([0x3b]));
fs.writeFileSync(OUTPUT, Buffer.concat(chunks));
console.log(`Wrote ${OUTPUT} (${fs.statSync(OUTPUT).size} bytes)`);
