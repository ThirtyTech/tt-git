// Draws the link-preview card and the PNG icons into public/:
//
//   og.png                1200×630, shown by Discord, Slack, X and others
//                         when someone pastes a link to the site
//   apple-touch-icon.png  180×180, for iOS home screens
//   icon-192.png          192×192, for browsers that don't take the SVG favicon
//
//   pnpm screens:social
//
// The card is the logo, the name and the hero screenshot
// (src/assets/screens/hero.webp), so run this after `pnpm screens hero`.
// Chrome does the drawing, through playwright-core, as in render.mjs; set
// CHROME_PATH to use a particular browser.

import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { chromium } from 'playwright-core';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const ROOT = join(here, '..');
const PUBLIC = join(ROOT, 'public');

const W = 1200;
const H = 630;

const base64 = async (path) => (await readFile(path)).toString('base64');
const font = (weight) =>
	base64(require.resolve(`@fontsource/jetbrains-mono/files/jetbrains-mono-latin-${weight}-normal.woff2`));

async function main() {
	const logo = await readFile(join(ROOT, 'src', 'assets', 'logo.svg'), 'utf8');
	const [regular, bold, hero] = await Promise.all([
		font(400),
		font(700),
		base64(join(ROOT, 'src', 'assets', 'screens', 'hero.webp')),
	]);

	const browser = await chromium.launch(
		process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: 'chrome' },
	);
	const page = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });
	await page.setContent(card(logo, regular, bold, hero));
	await page.evaluate(() => document.fonts.ready);
	const png = await page.screenshot({ type: 'png' });
	await writeFile(join(PUBLIC, 'og.png'), await sharp(png).png({ compressionLevel: 9, palette: false }).toBuffer());
	console.log('og.png ok');
	await browser.close();

	const svg = await readFile(join(PUBLIC, 'favicon.svg'));
	for (const [name, size] of [
		['apple-touch-icon.png', 180],
		['icon-192.png', 192],
	]) {
		await sharp(svg, { density: 384 }).resize(size, size).png({ compressionLevel: 9 }).toFile(join(PUBLIC, name));
		console.log(`${name} ok`);
	}
}

function card(logo, regular, bold, hero) {
	return `<!doctype html><html><head><meta charset="utf-8"><style>
@font-face { font-family: 'JB'; font-weight: 400; src: url(data:font/woff2;base64,${regular}) format('woff2'); }
@font-face { font-family: 'JB'; font-weight: 700; src: url(data:font/woff2;base64,${bold}) format('woff2'); }
html, body { margin: 0; width: ${W}px; height: ${H}px; overflow: hidden; }
body {
	position: relative;
	background:
		radial-gradient(circle at 0% 0%, rgba(75, 192, 235, 0.16), transparent 55%),
		radial-gradient(circle at 100% 100%, rgba(123, 217, 56, 0.10), transparent 50%),
		linear-gradient(135deg, #11151c, #07090d);
	font-family: 'Segoe UI', system-ui, sans-serif;
	color: #e6e8ec;
}
.text { position: absolute; left: 64px; top: 64px; width: 470px; }
.logo { width: 140px; height: 140px; margin-left: -10px; }
h1 { font: 700 92px/1 'JB', monospace; margin: 22px 0 0; letter-spacing: -2px; color: #fff; }
h1 span { color: #4bc0eb; }
p { font-size: 29px; line-height: 1.32; margin: 24px 0 0; color: #c1c5cd; }
.url { position: absolute; left: 64px; bottom: 52px; font: 400 22px 'JB', monospace; color: #4bc0eb; }
.shot {
	position: absolute; left: 590px; top: 92px; width: 1000px;
	border-radius: 14px; border: 2px solid #2b313d;
	box-shadow: 0 30px 80px rgba(0, 0, 0, 0.65), 0 0 0 1px rgba(75, 192, 235, 0.12);
}
</style></head><body>
<img class="shot" src="data:image/webp;base64,${hero}">
<div class="text">
	<div class="logo">${logo}</div>
	<h1>tt<span>-</span>git</h1>
	<p>A terminal git client with a commit graph, a status pane and plugins.</p>
</div>
<div class="url">thirtytech.github.io/tt-git</div>
</body></html>`;
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
