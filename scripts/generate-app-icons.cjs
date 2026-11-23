// Optional asset maintenance tool; ordinary Cargo builds use checked-in outputs.
// node scripts/generate-app-icons.cjs [path-to-sharp-package]
const fs = require('node:fs');
const path = require('node:path');
const sharp = require(process.argv[2] || 'sharp');
const root = path.resolve(__dirname, '../apps/explorer/assets/branding');
const sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024];
const smallMaxSize = 40;

(async () => {
  const frames = new Map();
  for (const size of sizes) {
    const source = size <= smallMaxSize ? 'skylis-small.svg' : 'skylis.svg';
    const png = await sharp(path.join(root, source), { density: 192 })
      .resize(size, size).png().toBuffer();
    frames.set(size, png);
    fs.writeFileSync(path.join(root, `skylis-${size}.png`), png);
  }
  // Modern Windows supports PNG frames in ICO; keep individual small sizes sharp.
  const icoSizes = sizes.filter(size => size <= 256);
  const directory = Buffer.alloc(6 + 16 * icoSizes.length);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(icoSizes.length, 4);
  let offset = directory.length;
  icoSizes.forEach((size, index) => {
    const frame = frames.get(size);
    const at = 6 + index * 16;
    directory[at] = directory[at + 1] = size === 256 ? 0 : size;
    directory.writeUInt16LE(1, at + 4);
    directory.writeUInt16LE(32, at + 6);
    directory.writeUInt32LE(frame.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += frame.length;
  });
  fs.writeFileSync(path.join(root, 'skylis.ico'), Buffer.concat([
    directory, ...icoSizes.map(size => frames.get(size)),
  ]));
  const types = [['icp4', 16], ['icp5', 32], ['icp6', 64], ['ic07', 128],
    ['ic08', 256], ['ic09', 512], ['ic10', 1024], ['ic11', 32],
    ['ic12', 64], ['ic13', 256], ['ic14', 512]];
  const chunks = types.map(([type, size]) => {
    const png = frames.get(size);
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, 'ascii');
    header.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([header, png]);
  });
  const header = Buffer.alloc(8);
  header.write('icns', 0, 4, 'ascii');
  header.writeUInt32BE(8 + chunks.reduce((sum, chunk) => sum + chunk.length, 0), 4);
  fs.writeFileSync(path.join(root, 'skylis.icns'), Buffer.concat([header, ...chunks]));
  console.log(`Generated ${sizes.length} PNGs (<=40px optical small, >=48px base), ${icoSizes.length}-frame ICO and ICNS.`);
})().catch(error => { console.error(error); process.exitCode = 1; });
