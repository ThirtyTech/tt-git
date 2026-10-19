// Draws the screen captures from `docshots` as 1920×1080 images with
// highlights and callouts, into src/assets/screens/.
//
//   pnpm screens            every shot in shots.mjs
//   pnpm screens graph      only shots whose name contains "graph"
//
// Captures come from `pnpm screens:capture` (the tt-git-testkit `docshots`
// example), which renders the real app against a mock repository. Chrome
// does the drawing, through playwright-core; set CHROME_PATH to use a
// particular browser.

import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { chromium } from 'playwright-core';
import sharp from 'sharp';
import { shots } from './shots.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const CAPTURES = join(here, 'captures');
const OUT = join(here, '..', 'src', 'assets', 'screens');

const W = 1920;
const H = 1080;
const CELL_W = 10;
const CELL_H = 20;
const TITLE_BAR = 36;

const font = async (weight) =>
	(
		await readFile(
			require.resolve(`@fontsource/jetbrains-mono/files/jetbrains-mono-latin-${weight}-normal.woff2`),
		)
	).toString('base64');

async function main() {
	const filter = process.argv[2];
	const todo = shots.filter((s) => !filter || s.name.includes(filter));
	if (!todo.length) throw new Error(`no shot matches ${filter}`);
	await mkdir(OUT, { recursive: true });

	const browser = await chromium.launch(
		process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: 'chrome' },
	);
	const page = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });
	const [regular, bold] = await Promise.all([font(400), font(700)]);
	await page.setContent(shell(regular, bold));
	await page.evaluate(() => document.fonts.ready);

	for (const shot of todo) {
		const path = join(CAPTURES, `${shot.capture}.json`);
		if (!existsSync(path)) throw new Error(`missing capture ${path}; run pnpm screens:capture`);
		const capture = JSON.parse(await readFile(path, 'utf8'));
		process.stdout.write(`${shot.name} `);
		const problems = await page.evaluate(
			([capture, shot, geo]) => window.draw(capture, shot, geo),
			[capture, shot, { W, H, CELL_W, CELL_H, TITLE_BAR }],
		);
		for (const p of problems) console.warn(`  ${shot.name}: ${p}`);
		const png = await page.screenshot({ type: 'png' });
		const file = join(OUT, `${shot.name}.webp`);
		await writeFile(file, await sharp(png).webp({ nearLossless: true, quality: 60, effort: 6 }).toBuffer());
		console.log('ok');
	}
	await browser.close();
}

// The page: one canvas for the terminal, one SVG on top for annotations.
// Everything else happens in `draw`, below, run inside the page.
function shell(regular, bold) {
	return `<!doctype html><html><head><meta charset="utf-8"><style>
@font-face { font-family: 'JB'; font-weight: 400; src: url(data:font/woff2;base64,${regular}) format('woff2'); }
@font-face { font-family: 'JB'; font-weight: 700; src: url(data:font/woff2;base64,${bold}) format('woff2'); }
html, body { margin: 0; width: ${W}px; height: ${H}px; overflow: hidden; background: #07090d; }
canvas, svg { position: absolute; left: 0; top: 0; }
</style></head><body>
<canvas id="c" width="${W}" height="${H}"></canvas>
<svg id="o" width="${W}" height="${H}" xmlns="http://www.w3.org/2000/svg"></svg>
<span style="font-family: JB; font-weight: 700">.</span><span style="font-family: JB">.</span>
<script>window.draw = ${draw.toString()};</script>
</body></html>`;
}

// Runs in the browser. Draws `capture` per `shot` and returns a list of
// problems (annotation targets it couldn't find).
function draw(capture, shot, geo) {
	const { W, H, CELL_W, CELL_H, TITLE_BAR } = geo;
	const problems = [];
	const canvas = document.getElementById('c');
	const svg = document.getElementById('o');
	svg.innerHTML = '';
	const ctx = canvas.getContext('2d');
	ctx.setTransform(1, 0, 0, 1, 0, 0);
	ctx.clearRect(0, 0, W, H);

	// --- colors -----------------------------------------------------------
	const BG = '#0f1319';
	const FG = '#c9d1d9';
	const NAMED = {
		black: '#0d1117',
		red: '#f47067',
		green: '#8ddb4c',
		yellow: '#e3b341',
		blue: '#539bf5',
		magenta: '#d6a8ff',
		cyan: '#4bc0eb',
		gray: '#c5cbd3',
		darkgray: '#5d6573',
		lightred: '#ff938a',
		lightgreen: '#a5e075',
		lightyellow: '#f2cc60',
		lightblue: '#6cb6ff',
		lightmagenta: '#e2c5ff',
		lightcyan: '#96d0ff',
		white: '#f0f3f6',
	};
	const ANSI = Object.values(NAMED);
	const color = (c, fallback) => {
		if (c === 'reset') return fallback;
		if (c.startsWith('#')) return c;
		if (c.startsWith('i')) {
			const i = +c.slice(1);
			if (i < 16) return ANSI[i];
			if (i >= 232) {
				const v = 8 + (i - 232) * 10;
				return `rgb(${v},${v},${v})`;
			}
			const n = i - 16;
			const lv = (x) => (x ? 55 + x * 40 : 0);
			return `rgb(${lv(Math.floor(n / 36))},${lv(Math.floor(n / 6) % 6)},${lv(n % 6)})`;
		}
		return NAMED[c] ?? fallback;
	};

	// --- where the grid and the callouts go ---------------------------------
	const cols = capture.width;
	const rows = capture.height;
	const view = shot.view ?? { full: true };
	// A cell rect for the part of the screen being shown.
	const region = view.full ? { x: 0, y: 0, w: cols, h: rows } : resolve(view.region);
	if (!region) throw new Error(`${shot.name}: view region not found: ${JSON.stringify(view.region)}`);
	const margin = view.margin ?? 56;
	const FONT = 30;
	const LEGEND_W = 640;
	const GAP = 18;
	const PAD = 10;
	const sum = (xs) => xs.reduce((a, b) => a + b, 0);
	const measure = (c, width) => {
		const size = c.fontSize ?? FONT;
		const lines = wrap(c.text, width - 92, size);
		const lh = size * 1.3;
		return { width, size, lines, lh, bh: Math.max(72, lines.length * lh + 36) };
	};
	let scale = 1;
	let ox = 0;
	let oy = 0;
	// Callouts without `at` or `side` go in a legend, keyed by index.
	const inLegend = (shot.callouts ?? []).map((c, i) => [c, i]).filter(([c]) => !c.at && !c.side);
	const legendAt = [];
	const stack = (x, width, top) => {
		const hs = inLegend.map(([c]) => measure(c, width).bh);
		let y = top ?? (H - sum(hs) - GAP * (hs.length - 1)) / 2;
		inLegend.forEach(([, i], k) => {
			legendAt[i] = { x, y, width };
			y += hs[k] + GAP;
		});
	};

	if (view.full) {
		ox = (W - cols * CELL_W) / 2;
		oy = TITLE_BAR + (H - TITLE_BAR - rows * CELL_H) / 2;
		if (shot.legend) stack(shot.legend.x, shot.legend.width ?? LEGEND_W, shot.legend.y);
	} else {
		const rw = region.w * CELL_W;
		const rh = region.h * CELL_H;
		const between = 56;
		const maxScale = view.maxScale ?? 3;
		const rightWidth = view.legendWidth ?? LEGEND_W;
		const rightScale = Math.min(
			(W - 2 * margin - 2 * PAD - between - rightWidth) / rw,
			(H - 2 * margin - 2 * PAD) / rh,
			maxScale,
		);
		const n = inLegend.length;
		const ncol = view.legendColumns ?? Math.min(n, 3);
		const gridWidth = (W - 2 * margin - (ncol - 1) * GAP) / ncol;
		const gridRows = [];
		inLegend.forEach(([c], k) => {
			const r = Math.floor(k / ncol);
			gridRows[r] = Math.max(gridRows[r] ?? 0, measure(c, gridWidth).bh);
		});
		const gridH = sum(gridRows) + GAP * Math.max(0, gridRows.length - 1);
		const bottomScale = Math.min(
			(W - 2 * margin - 2 * PAD) / rw,
			(H - 2 * margin - 2 * PAD - between - gridH) / rh,
			maxScale,
		);
		// Put the callouts wherever the zoom can be bigger.
		const side = !n ? 'none' : (view.legend ?? (bottomScale > rightScale * 1.05 ? 'bottom' : 'right'));
		if (side === 'right') {
			const width = rightWidth;
			scale = rightScale;
			const left = (W - (rw * scale + 2 * PAD + between + width)) / 2 + PAD;
			ox = left - region.x * CELL_W * scale;
			oy = (H - rh * scale) / 2 - region.y * CELL_H * scale;
			stack(left + rw * scale + PAD + between, width);
		} else if (side === 'bottom') {
			const width = gridWidth;
			const rowH = gridRows;
			scale = bottomScale;
			const top = (H - (rh * scale + 2 * PAD + between + gridH)) / 2 + PAD;
			ox = (W - rw * scale) / 2 - region.x * CELL_W * scale;
			oy = top - region.y * CELL_H * scale;
			let gy = top + rh * scale + PAD + between;
			rowH.forEach((h, r) => {
				const items = inLegend.slice(r * ncol, r * ncol + ncol);
				let gx = (W - (items.length * width + (items.length - 1) * GAP)) / 2;
				for (const [, i] of items) {
					legendAt[i] = { x: gx, y: gy, width };
					gx += width + GAP;
				}
				gy += h + GAP;
			});
		} else {
			scale = Math.min((W - 2 * margin - 2 * PAD) / rw, (H - 2 * margin - 2 * PAD) / rh, maxScale);
			ox = (W - rw * scale) / 2 - region.x * CELL_W * scale;
			oy = (H - rh * scale) / 2 - region.y * CELL_H * scale;
		}
	}
	// Cell (x, y) → page pixels.
	const px = (x) => ox + x * CELL_W * scale;
	const py = (y) => oy + y * CELL_H * scale;

	// --- backdrop -------------------------------------------------------------
	if (view.full) {
		// A terminal window filling the frame.
		ctx.fillStyle = BG;
		ctx.fillRect(0, 0, W, H);
		ctx.fillStyle = '#161b24';
		ctx.fillRect(0, 0, W, TITLE_BAR);
		ctx.fillStyle = '#0f1319';
		roundRect(ctx, 12, 6, 300, TITLE_BAR - 6, 8, true);
		ctx.fillRect(12, 20, 300, TITLE_BAR - 20);
		ctx.fillStyle = '#4bc0eb';
		ctx.fillRect(12, 6, 300, 2);
		ctx.font = '13px "Segoe UI", system-ui, sans-serif';
		ctx.fillStyle = '#e6e8ec';
		ctx.textBaseline = 'middle';
		ctx.fillText(shot.tab ?? 'tt-git — lumen-web', 28, 6 + (TITLE_BAR - 6) / 2 + 1);
		ctx.fillStyle = '#8b919c';
		ctx.fillText('+', 330, TITLE_BAR / 2 + 3);
		// Window buttons.
		ctx.strokeStyle = '#8b919c';
		ctx.lineWidth = 1;
		const by = TITLE_BAR / 2;
		ctx.beginPath();
		ctx.moveTo(W - 136, by);
		ctx.lineTo(W - 126, by);
		ctx.stroke();
		ctx.strokeRect(W - 86.5, by - 5.5, 10, 10);
		ctx.beginPath();
		ctx.moveTo(W - 36, by - 5);
		ctx.lineTo(W - 26, by + 5);
		ctx.moveTo(W - 26, by - 5);
		ctx.lineTo(W - 36, by + 5);
		ctx.stroke();
	} else {
		const g = ctx.createLinearGradient(0, 0, W, H);
		g.addColorStop(0, '#0d1520');
		g.addColorStop(1, '#06080c');
		ctx.fillStyle = g;
		ctx.fillRect(0, 0, W, H);
		// Faint dot grid, like the logo's tile.
		ctx.fillStyle = 'rgba(75,192,235,0.07)';
		for (let y = 24; y < H; y += 32) for (let x = 24; x < W; x += 32) ctx.fillRect(x, y, 2, 2);
		// The zoomed piece as a card with a shadow.
		const pad = 10;
		const cx = px(region.x) - pad;
		const cy = py(region.y) - pad;
		const cw = region.w * CELL_W * scale + 2 * pad;
		const ch = region.h * CELL_H * scale + 2 * pad;
		ctx.save();
		ctx.shadowColor = 'rgba(75,192,235,0.25)';
		ctx.shadowBlur = 48;
		ctx.fillStyle = BG;
		roundRect(ctx, cx, cy, cw, ch, 14, true);
		ctx.restore();
		ctx.strokeStyle = 'rgba(75,192,235,0.35)';
		ctx.lineWidth = 1.5;
		roundRect(ctx, cx, cy, cw, ch, 14, false);
		ctx.save();
		ctx.beginPath();
		ctx.rect(px(region.x), py(region.y), region.w * CELL_W * scale, region.h * CELL_H * scale);
		ctx.clip();
	}

	// --- cells ----------------------------------------------------------------
	ctx.save();
	ctx.translate(ox, oy);
	ctx.scale(scale, scale);
	ctx.textBaseline = 'alphabetic';
	const fontFor = (bold) => `${bold ? 700 : 400} 16.6px JB, "Cascadia Mono", Consolas, "Segoe UI Symbol", monospace`;
	const y0 = view.full ? 0 : region.y;
	const y1 = view.full ? rows : region.y + region.h;
	const x0 = view.full ? 0 : region.x;
	const x1 = view.full ? cols : region.x + region.w;
	// Backgrounds first, so wide glyphs and line joins aren't painted over.
	for (let y = y0; y < y1; y++) {
		for (let x = x0; x < x1; x++) {
			const [, si] = capture.rows[y][x];
			const [fg, bg, mods] = capture.styles[si];
			const back = mods.includes('r') ? color(fg, FG) : color(bg, BG);
			if (back !== BG) {
				ctx.fillStyle = back;
				ctx.fillRect(x * CELL_W, y * CELL_H, CELL_W + 0.5, CELL_H + 0.5);
			}
		}
	}
	for (let y = y0; y < y1; y++) {
		for (let x = x0; x < x1; x++) {
			const [sym, si] = capture.rows[y][x];
			if (!sym || sym === ' ') continue;
			const [fg, bg, mods] = capture.styles[si];
			let fore = mods.includes('r') ? color(bg, BG) : color(fg, FG);
			ctx.globalAlpha = mods.includes('d') ? 0.6 : 1;
			ctx.fillStyle = fore;
			ctx.strokeStyle = fore;
			const cx = x * CELL_W;
			const cy = y * CELL_H;
			if (!glyph(ctx, sym, cx, cy)) {
				ctx.font = fontFor(mods.includes('b'));
				ctx.fillText(sym, cx, cy + 15);
			}
			if (mods.includes('u')) ctx.fillRect(cx, cy + CELL_H - 3, CELL_W, 1);
		}
	}
	ctx.globalAlpha = 1;
	ctx.restore();
	if (!view.full) ctx.restore();

	// --- annotations ------------------------------------------------------------
	const NS = 'http://www.w3.org/2000/svg';
	const el = (name, attrs, parent = svg) => {
		const e = document.createElementNS(NS, name);
		for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
		parent.appendChild(e);
		return e;
	};
	const defs = el('defs', {});
	defs.innerHTML = `
		<filter id="glow" x="-20%" y="-20%" width="140%" height="140%">
			<feGaussianBlur stdDeviation="4" result="b"/>
			<feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
		</filter>
		<filter id="shadow" x="-10%" y="-10%" width="120%" height="140%">
			<feDropShadow dx="0" dy="6" stdDeviation="10" flood-color="#000" flood-opacity="0.55"/>
		</filter>`;

	const ACCENT = { cyan: '#4bc0eb', green: '#7bd938', amber: '#f2b84b', pink: '#ff7ab6' };
	const hl = (shot.highlights ?? []).map((h) => ({ ...h, rect: toPx(h.target, h.pad ?? 4) }));
	const callouts = (shot.callouts ?? []).map((c, i) => ({
		...c,
		n: c.n ?? i + 1,
		rect: c.target ? toPx(c.target, c.pad ?? 4) : null,
	}));

	// Spotlight: dim everything but the highlights.
	const holes = [...hl, ...callouts.filter((c) => c.rect && c.box !== false)]
		.map((h) => h.rect)
		.filter(Boolean);
	if (shot.dim && holes.length) {
		const mask = el('mask', { id: 'spot' }, defs);
		el('rect', { x: 0, y: 0, width: W, height: H, fill: 'white' }, mask);
		for (const r of holes) el('rect', { x: r.x, y: r.y, width: r.w, height: r.h, rx: 8, fill: 'black' }, mask);
		el('rect', { x: 0, y: 0, width: W, height: H, fill: 'rgba(4,6,10,0.62)', mask: 'url(#spot)' });
	}

	const box = (r, tone, label) => {
		const c = ACCENT[tone] ?? ACCENT.cyan;
		el('rect', {
			x: r.x,
			y: r.y,
			width: r.w,
			height: r.h,
			rx: 7,
			fill: c,
			'fill-opacity': 0.07,
			stroke: c,
			'stroke-width': 4,
			filter: 'url(#glow)',
		});
		if (label) {
			// A pill inside the top-right corner.
			const t = el('text', {
				'font-family': 'Segoe UI, system-ui, sans-serif',
				'font-size': 28,
				'font-weight': 700,
				fill: '#06121c',
			});
			t.textContent = label;
			const tw = t.getComputedTextLength();
			const pw = tw + 32;
			const x = r.x + r.w - pw - 16;
			const y = r.y + 16;
			svg.insertBefore(el('rect', { x, y, width: pw, height: 44, rx: 22, fill: c }), t);
			t.setAttribute('x', x + 16);
			t.setAttribute('y', y + 31);
			svg.appendChild(t);
		}
	};
	for (const h of hl) if (h.rect) box(h.rect, h.tone, h.label);

	const sized = callouts.map((c, i) => {
		const spot = legendAt[i];
		const m = measure(c, c.width ?? spot?.width ?? 520);
		return spot ? { ...c, ...m, at: [spot.x, spot.y], line: c.line ?? false, marker: c.marker ?? true } : { ...c, ...m };
	});

	for (const c of sized) {
		const tone = ACCENT[c.tone] ?? ACCENT.cyan;
		if (c.rect && c.box !== false) box(c.rect, c.tone);
		const { width, lines, lh, bh } = c;
		const text = el('text', {
			'font-family': 'Segoe UI, system-ui, sans-serif',
			'font-size': c.size,
			fill: '#e9eef4',
		});
		if (c.rect && c.marker) marker(c.rect, c.markerSide ?? 'left', c.n, tone);
		let bx;
		let by;
		if (c.at) {
			[bx, by] = c.at;
		} else if (c.rect) {
			const gap = c.gap ?? 48;
			const side = c.side ?? 'right';
			const r = c.rect;
			if (side === 'right') [bx, by] = [r.x + r.w + gap, r.y + r.h / 2 - bh / 2];
			if (side === 'left') [bx, by] = [r.x - gap - width, r.y + r.h / 2 - bh / 2];
			if (side === 'below') [bx, by] = [r.x + r.w / 2 - width / 2, r.y + r.h + gap];
			if (side === 'above') [bx, by] = [r.x + r.w / 2 - width / 2, r.y - gap - bh];
			bx += c.dx ?? 0;
			by += c.dy ?? 0;
		} else {
			problems.push(`callout ${c.n} has neither at nor target`);
			continue;
		}
		bx = Math.max(12, Math.min(W - width - 12, bx));
		by = Math.max(12, Math.min(H - bh - 12, by));

		// Leader line from the box to the nearest point on the target.
		if (c.rect && c.line !== false) {
			const r = c.rect;
			const bcx = bx + width / 2;
			const bcy = by + bh / 2;
			const tx = Math.max(r.x, Math.min(r.x + r.w, bcx));
			const ty = Math.max(r.y, Math.min(r.y + r.h, bcy));
			const sx = Math.max(bx, Math.min(bx + width, tx));
			const sy = Math.max(by, Math.min(by + bh, ty));
			el('line', { x1: sx, y1: sy, x2: tx, y2: ty, stroke: tone, 'stroke-width': 3.5, 'stroke-linecap': 'round' });
			el('circle', { cx: tx, cy: ty, r: 7, fill: tone });
		}
		el('rect', {
			x: bx,
			y: by,
			width,
			height: bh,
			rx: 12,
			fill: '#0b1724',
			'fill-opacity': 0.97,
			stroke: tone,
			'stroke-width': 2,
			filter: 'url(#shadow)',
		});
		el('circle', { cx: bx + 38, cy: by + 36, r: 20, fill: tone });
		const num = el('text', {
			x: bx + 38,
			y: by + 44.5,
			'text-anchor': 'middle',
			'font-family': 'Segoe UI, system-ui, sans-serif',
			'font-size': 24,
			'font-weight': 700,
			fill: '#06121c',
		});
		num.textContent = String(c.n);
		lines.forEach((line, i) => {
			const t = el('tspan', { x: bx + 74, y: by + 18 + lh * (i + 0.5) + c.size * 0.35 }, text);
			// **bold** spans.
			line.split(/(\*\*[^*]+\*\*)/).forEach((part) => {
				if (!part) return;
				const bold = part.startsWith('**');
				const s = el('tspan', bold ? { 'font-weight': 700, fill: tone } : {}, t);
				s.textContent = bold ? part.slice(2, -2) : part;
			});
		});
		svg.appendChild(text);
	}

	return problems;

	// A numbered badge just outside a highlight.
	function marker(r, side, n, tone) {
		const R = 20;
		const g = 8;
		if (side === 'left' && r.x - 2 * R - g < 4) side = 'right';
		if (side === 'right' && r.x + r.w + 2 * R + g > W - 4) side = 'left';
		const at = {
			left: [r.x - R - g, r.y + r.h / 2],
			right: [r.x + r.w + R + g, r.y + r.h / 2],
			above: [r.x + r.w / 2, r.y - R - g],
			below: [r.x + r.w / 2, r.y + r.h + R + g],
			'top-left': [r.x, r.y],
			'top-right': [r.x + r.w, r.y],
		}[side];
		el('circle', { cx: at[0], cy: at[1], r: R + 2.5, fill: '#06121c' });
		el('circle', { cx: at[0], cy: at[1], r: R, fill: tone });
		const t = el('text', {
			x: at[0],
			y: at[1] + 8.5,
			'text-anchor': 'middle',
			'font-family': 'Segoe UI, system-ui, sans-serif',
			'font-size': 24,
			'font-weight': 700,
			fill: '#06121c',
		});
		t.textContent = String(n);
	}

	// --- helpers ----------------------------------------------------------------

	// A target → a cell rect {x, y, w, h}.
	//   { pane: 'status' }                  the pane's box
	//   { cells: [x, y, w, h] }             explicit
	//   { find: 'text', in?: pane, nth?, row?: true, w?, h?, dx?, dy? }
	//   { box: 'title text' }               the rounded box whose top border holds the text
	function resolve(t) {
		if (!t) return null;
		if (t.pane) {
			const p = capture.panes[t.pane];
			if (!p) {
				problems.push(`no pane ${t.pane}`);
				return null;
			}
			return { x: p.x, y: p.y, w: p.w, h: p.h };
		}
		if (t.cells) {
			const [x, y, w, h] = t.cells;
			return { x, y, w, h };
		}
		if (t.find) {
			const area = t.in ? capture.panes[t.in] : { x: 0, y: 0, w: cols, h: rows };
			let seen = 0;
			for (let y = area.y; y < area.y + area.h; y++) {
				const line = capture.text[y];
				let from = area.x;
				for (;;) {
					const i = line.indexOf(t.find, from);
					if (i < 0 || i >= area.x + area.w) break;
					if (seen++ === (t.nth ?? 0)) {
						let r = { x: i, y, w: [...t.find].length, h: 1 };
						if (t.row) r = { x: area.x + 1, y, w: area.w - 2, h: 1 };
						if (t.w) r.w = t.w;
						if (t.h) r.h = t.h;
						r.x += t.dx ?? 0;
						r.y += t.dy ?? 0;
						return r;
					}
					from = i + 1;
				}
			}
			problems.push(`not found: ${JSON.stringify(t.find)}`);
			return null;
		}
		if (t.box) {
			for (let y = 0; y < rows; y++) {
				const line = capture.text[y];
				// The title follows the corner, maybe after a space or a dash.
				let i = line.indexOf(t.box);
				while (i >= 0 && !/[╭┌]$/.test(line.slice(Math.max(0, i - 3), i).replace(/[ ─]/g, ''))) {
					i = line.indexOf(t.box, i + 1);
				}
				if (i < 0) continue;
				const left = i - 3 + line.slice(i - 3, i).search(/[╭┌]/);
				let right = left + 1;
				while (right < cols && !'╮┐'.includes(line[right])) right++;
				let bottom = y + 1;
				while (bottom < rows && !'╰└'.includes(capture.text[bottom][left])) bottom++;
				return { x: left, y, w: t.w ?? right - left + 1, h: t.h ?? bottom - y + 1 };
			}
			problems.push(`no box titled ${JSON.stringify(t.box)}`);
			return null;
		}
		problems.push(`bad target ${JSON.stringify(t)}`);
		return null;
	}

	function toPx(target, pad) {
		const r = resolve(target);
		if (!r) return null;
		return {
			x: px(r.x) - pad,
			y: py(r.y) - pad,
			w: r.w * CELL_W * scale + 2 * pad,
			h: r.h * CELL_H * scale + 2 * pad,
		};
	}

	function wrap(s, maxW, size) {
		const m = document.createElement('canvas').getContext('2d');
		m.font = `${size}px "Segoe UI", system-ui, sans-serif`;
		const out = [];
		for (const para of s.split('\n')) {
			let line = '';
			for (const word of para.split(' ')) {
				const next = line ? `${line} ${word}` : word;
				if (m.measureText(next.replaceAll('**', '')).width > maxW && line) {
					out.push(line);
					line = word;
				} else line = next;
			}
			out.push(line);
		}
		// Keep **bold** spans from breaking across lines.
		return out.map((l) => ((l.match(/\*\*/g) ?? []).length % 2 ? `${l}**` : l));
	}

	function roundRect(c, x, y, w, h, r, fill) {
		c.beginPath();
		c.roundRect(x, y, w, h, r);
		if (fill) c.fill();
		else c.stroke();
	}

	// Box-drawing characters and commit dots, drawn as shapes so lines meet
	// exactly. Returns false for anything else.
	function glyph(c, sym, x, y) {
		const cx = x + CELL_W / 2;
		const cy = y + CELL_H / 2;
		const LINES = {
			'─': 'lr', '━': 'lr', '│': 'ud', '┃': 'ud', '┼': 'lrud', '┴': 'lru', '┬': 'lrd',
			'├': 'udr', '┤': 'udl', '┌': 'rd', '┐': 'ld', '└': 'ur', '┘': 'ul',
		};
		const ROUND = { '╭': 'rd', '╮': 'ld', '╰': 'ur', '╯': 'ul' };
		c.lineWidth = 1.4;
		c.lineCap = 'butt';
		if (LINES[sym]) {
			const s = LINES[sym];
			c.beginPath();
			if (s.includes('l')) { c.moveTo(x, cy); c.lineTo(cx, cy); }
			if (s.includes('r')) { c.moveTo(cx, cy); c.lineTo(x + CELL_W, cy); }
			if (s.includes('u')) { c.moveTo(cx, y); c.lineTo(cx, cy); }
			if (s.includes('d')) { c.moveTo(cx, cy); c.lineTo(cx, y + CELL_H); }
			if (s.length > 1) {
				// Join the half-lines cleanly at the centre.
				c.moveTo(cx, cy);
			}
			c.stroke();
			return true;
		}
		if (ROUND[sym]) {
			const s = ROUND[sym];
			const r = CELL_W / 2;
			const ex = s.includes('r') ? x + CELL_W : x;
			const ey = s.includes('d') ? y + CELL_H : y;
			c.beginPath();
			c.moveTo(ex, cy);
			c.arcTo(cx, cy, cx, ey, r);
			c.lineTo(cx, ey);
			c.stroke();
			return true;
		}
		if (sym === '●') {
			c.beginPath();
			c.arc(cx, cy, 4.2, 0, Math.PI * 2);
			c.fill();
			return true;
		}
		if (sym === '◉') {
			c.lineWidth = 1.6;
			c.beginPath();
			c.arc(cx, cy, 4.6, 0, Math.PI * 2);
			c.stroke();
			c.beginPath();
			c.arc(cx, cy, 2.2, 0, Math.PI * 2);
			c.fill();
			return true;
		}
		return false;
	}
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
