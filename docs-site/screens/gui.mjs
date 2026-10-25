// The desktop app's screenshots: the PNGs `guishots` writes to
// screens/gui-captures/ (the real app's windows, drawn by GPUI), saved as
// src/assets/screens/gui-<name>-<os>.webp. No callouts: the windows are
// pictures, not terminal cells render.mjs could find text in.
//
//   node screens/gui.mjs            every capture
//   node screens/gui.mjs menu       the captures whose name contains "menu"

import { readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const CAPTURES = join(here, 'gui-captures');
const OUT = join(here, '..', 'src', 'assets', 'screens');

const filter = process.argv[2];
let files;
try {
	files = readdirSync(CAPTURES).filter((f) => f.endsWith('.png'));
} catch {
	console.error(`No captures in ${CAPTURES}. Run guishots first (see docs-site/README.md).`);
	process.exit(1);
}
for (const file of files) {
	const name = `gui-${file.replace(/\.png$/, '')}`;
	if (filter && !name.includes(filter)) continue;
	// Lossless: the windows are mostly flat colors and text.
	await sharp(join(CAPTURES, file)).webp({ lossless: true, effort: 6 }).toFile(join(OUT, `${name}.webp`));
	console.log(name);
}
