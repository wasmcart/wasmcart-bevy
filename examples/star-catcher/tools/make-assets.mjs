#!/usr/bin/env node
// Draws star-catcher's art and sound from code, so every asset in the cart is
// original and reproducible: node tools/make-assets.mjs
import { writeFileSync, mkdirSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const out = join(dirname(fileURLToPath(import.meta.url)), '..', 'assets');
mkdirSync(out, { recursive: true });

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}
function png(w, h, pixel) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const [r, g, b, a] = pixel(x, y).map((v) => Math.max(0, Math.min(255, Math.round(v))));
      raw.set([r, g, b, a], y * (w * 4 + 1) + 1 + x * 4);
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0)),
  ]);
}
const smooth = (e0, e1, x) => { const t = Math.max(0, Math.min(1, (x - e0) / (e1 - e0))); return t * t * (3 - 2 * t); };

// Ship: a swept arrowhead with a cockpit and an engine glow, 64x64.
writeFileSync(join(out, 'ship.png'), png(64, 64, (x, y) => {
  const u = (x + 0.5 - 32) / 32, v = (y + 0.5 - 32) / 32; // v: -1 top (nose) .. 1 bottom
  const half = (v + 0.9) * 0.55 * (v < 0.55 ? 1 : 1 - (v - 0.55) * 2.2);
  const d = half - Math.abs(u);
  const body = smooth(-0.04, 0.03, d) * (v > -0.9 ? 1 : 0);
  if (body <= 0) {
    const g = Math.exp(-((u * u) / 0.02 + ((v - 0.78) ** 2) / 0.01));
    return [255, 170 * g + 60, 40, 255 * g];
  }
  const shade = 0.55 + 0.45 * (1 - Math.abs(u) / Math.max(half, 0.01));
  const cockpit = Math.exp(-((u * u) / 0.008 + ((v + 0.15) ** 2) / 0.03));
  const stripe = Math.abs(Math.abs(u) - 0.18) < 0.035 && v > 0.1 ? 1 : 0;
  return [
    (70 + 120 * shade) * (1 - cockpit) + 120 * cockpit + 140 * stripe,
    (110 + 110 * shade) * (1 - cockpit) + 230 * cockpit + 30 * stripe,
    (170 + 80 * shade) * (1 - cockpit) + 255 * cockpit,
    255 * body,
  ];
}));

// Star: a five-pointed gold star with a soft glow, 48x48.
writeFileSync(join(out, 'star.png'), png(48, 48, (x, y) => {
  const u = (x + 0.5 - 24) / 24, v = (y + 0.5 - 24) / 24;
  const r = Math.hypot(u, v), a = Math.atan2(v, u) + Math.PI / 2;
  const sector = 2 * Math.PI / 5;
  const m = ((a % sector) + sector) % sector;
  const t = 1 - Math.abs(m - sector / 2) / (sector / 2);
  const edge = 0.36 + 0.58 * Math.pow(1 - t, 2.2);
  const inside = smooth(0.03, -0.03, r - edge);
  const glow = Math.exp(-r * r * 5) * 0.6;
  const a8 = Math.min(1, inside + glow) * 255;
  const hot = 1 - r;
  return [255, 190 + 60 * hot, 60 + 150 * hot * inside, a8];
}));

// Pickup chime: two quick rising sine notes, 16-bit mono 22050 Hz.
{
  const rate = 22050, secs = 0.22, n = Math.floor(rate * secs);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const f = t < 0.08 ? 988 : 1319;
    const lt = t < 0.08 ? t : t - 0.08;
    const env = Math.min(1, lt / 0.004) * Math.exp(-lt * 18);
    const s = Math.sin(2 * Math.PI * f * t) * 0.6 + Math.sin(4 * Math.PI * f * t) * 0.15;
    data.writeInt16LE(Math.round(s * env * 0.7 * 32767), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8);
  h.write('fmt ', 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write('data', 36); h.writeUInt32LE(data.length, 40);
  writeFileSync(join(out, 'pickup.wav'), Buffer.concat([h, data]));
}
console.log('wrote', out);
