import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const outputPath = path.resolve(here, '../src-tauri/icons/icon.ico');
const size = 256;
const pixels = Buffer.alloc(size * size * 4);

const insideRoundedSquare = (x, y, left, top, right, bottom, radius) => {
  const nearestX = Math.max(left + radius, Math.min(x, right - radius));
  const nearestY = Math.max(top + radius, Math.min(y, bottom - radius));
  return (x - nearestX) ** 2 + (y - nearestY) ** 2 <= radius ** 2;
};

const insidePolygon = (x, y, points) => {
  let hit = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const [xi, yi] = points[i];
    const [xj, yj] = points[j];
    const crosses = (yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi;
    if (crosses) hit = !hit;
  }
  return hit;
};

const star = [];
for (let i = 0; i < 16; i++) {
  const angle = -Math.PI / 2 + (i * Math.PI) / 8;
  const radius = i % 2 === 0 ? 19 : 7;
  star.push([184 + Math.cos(angle) * radius, 71 + Math.sin(angle) * radius]);
}

const leftLeg = [[72, 199], [110, 58], [139, 58], [184, 199], [151, 199], [125, 111], [103, 199]];
const crossbar = [[99, 145], [155, 145], [163, 169], [92, 169]];

for (let y = 0; y < size; y++) {
  for (let x = 0; x < size; x++) {
    const rounded = insideRoundedSquare(x + 0.5, y + 0.5, 22, 22, 234, 234, 52);
    const border = insideRoundedSquare(x + 0.5, y + 0.5, 19, 19, 237, 237, 55) && !rounded;
    const isGlyph = insidePolygon(x + 0.5, y + 0.5, leftLeg) || insidePolygon(x + 0.5, y + 0.5, crossbar) || insidePolygon(x + 0.5, y + 0.5, star);
    const offset = (y * size + x) * 4;
    if (isGlyph || border) {
      pixels[offset] = 201;
      pixels[offset + 1] = 250;
      pixels[offset + 2] = 100;
      pixels[offset + 3] = 255;
    } else if (rounded) {
      const glow = Math.max(0, 20 - Math.floor(Math.hypot(x - 82, y - 61) / 8));
      pixels[offset] = 18 + glow;
      pixels[offset + 1] = 26 + glow;
      pixels[offset + 2] = 35 + glow;
      pixels[offset + 3] = 255;
    }
  }
}

const stride = size * 4;
const xor = Buffer.alloc(stride * size);
for (let y = 0; y < size; y++) {
  const sourceY = size - 1 - y;
  for (let x = 0; x < size; x++) {
    const source = (sourceY * size + x) * 4;
    const target = y * stride + x * 4;
    xor[target] = pixels[source + 2];
    xor[target + 1] = pixels[source + 1];
    xor[target + 2] = pixels[source];
    xor[target + 3] = pixels[source + 3];
  }
}

const maskStride = Math.ceil(size / 32) * 4;
const andMask = Buffer.alloc(maskStride * size);
for (let y = 0; y < size; y++) {
  for (let x = 0; x < size; x++) {
    if (pixels[(y * size + x) * 4 + 3] === 0) {
      const offset = (size - 1 - y) * maskStride + Math.floor(x / 8);
      andMask[offset] |= 0x80 >> (x % 8);
    }
  }
}

const bitmap = Buffer.alloc(40);
bitmap.writeUInt32LE(40, 0);
bitmap.writeInt32LE(size, 4);
bitmap.writeInt32LE(size * 2, 8);
bitmap.writeUInt16LE(1, 12);
bitmap.writeUInt16LE(32, 14);
bitmap.writeUInt32LE(0, 16);
bitmap.writeUInt32LE(xor.length, 20);

const image = Buffer.concat([bitmap, xor, andMask]);
const header = Buffer.alloc(6 + 16);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(1, 4);
header.writeUInt8(0, 6);
header.writeUInt8(0, 7);
header.writeUInt8(0, 8);
header.writeUInt8(0, 9);
header.writeUInt16LE(1, 10);
header.writeUInt16LE(32, 12);
header.writeUInt32LE(image.length, 14);
header.writeUInt32LE(header.length, 18);

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, Buffer.concat([header, image]));
process.stdout.write(`Generated ${outputPath}\n`);
