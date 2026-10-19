// Generates the site icons from public/hushush-mark.png:
//   src/app/favicon.ico     16, 32, 48 px (browser tabs)
//   src/app/icon.png        512 px (modern browsers, Android)
//   src/app/apple-icon.png  180 px (iOS home screen; square, iOS rounds it)
// The mark is dark on transparent, so it sits on a cream tile that reads on
// both light and dark tabs. Next.js picks these files up by name.
//
// Usage (from web/): node scripts/make-icons.mjs
import { writeFileSync } from "node:fs";
import path from "node:path";
import sharp from "sharp";

const ROOT = path.resolve(import.meta.dirname, "..");
const MARK = path.join(ROOT, "public/hushush-mark.png");
const OUT = path.join(ROOT, "src/app");
const CREAM = "#f8f7f5";

/** The mark centred on a cream tile of `size` px; `rounded` = rounded corners (else full square). */
async function tile(size, { rounded = true, fill = 0.68 } = {}) {
  const markH = Math.round(size * fill);
  const mark = await sharp(MARK).resize({ height: markH, fit: "inside", kernel: "lanczos3" }).toBuffer();
  const r = rounded ? Math.round(size * 0.22) : 0;
  const bg = Buffer.from(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}"><rect width="${size}" height="${size}" rx="${r}" ry="${r}" fill="${CREAM}"/></svg>`,
  );
  return sharp(bg).composite([{ input: mark, gravity: "center" }]).png().toBuffer();
}

/** An .ico holding PNG images (supported by every current browser). */
function ico(pngs) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // icon
  header.writeUInt16LE(pngs.length, 4);
  const dir = Buffer.alloc(16 * pngs.length);
  let offset = 6 + dir.length;
  pngs.forEach(({ size, data }, i) => {
    const e = i * 16;
    dir.writeUInt8(size >= 256 ? 0 : size, e);
    dir.writeUInt8(size >= 256 ? 0 : size, e + 1);
    dir.writeUInt16LE(1, e + 4); // colour planes
    dir.writeUInt16LE(32, e + 6); // bits per pixel
    dir.writeUInt32LE(data.length, e + 8);
    dir.writeUInt32LE(offset, e + 12);
    offset += data.length;
  });
  return Buffer.concat([header, dir, ...pngs.map((p) => p.data)]);
}

// Tiny sizes: a bit more mark, so it stays legible at 16 px.
const small = await Promise.all([16, 32, 48].map(async (size) => ({ size, data: await tile(size, { fill: 0.78 }) })));
writeFileSync(path.join(OUT, "favicon.ico"), ico(small));
writeFileSync(path.join(OUT, "icon.png"), await tile(512));
writeFileSync(path.join(OUT, "apple-icon.png"), await tile(180, { rounded: false }));
console.log("wrote src/app/favicon.ico, icon.png, apple-icon.png");
