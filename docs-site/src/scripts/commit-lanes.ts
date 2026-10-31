// Commit lanes on a canvas, after the backdrop in the video: thin lanes with
// commit rings, and pulses that light each commit they pass and sometimes hop
// to the next lane, drawing a merge line as it goes.
//
// With reduced motion on, the lanes are drawn once and nothing moves.
//
// The code works along and across the lanes. Lanes run down the canvas with
// pulses climbing them, and `horizontal` rotates that a quarter turn so they
// run across it with pulses heading right.

export interface CommitLaneOptions {
	/** Run the lanes left to right instead of top to bottom. */
	horizontal?: boolean;
	/** The custom property for the color behind the canvas. */
	background: string;
	laneSpacing: number;
	minLanes: number;
	sway: [number, number];
	wavelength: [number, number];
	commitGap: [number, number];
	speed: [number, number];
	hopLength: number;
	trailLength: number;
	lineWidth: number;
	commitRadius: number;
	ringGrowth: number;
	glowRadius: number;
	/** How far the lanes move per pixel scrolled. */
	parallax: number;
	/** Keep the lanes behind the middle column quiet, for text on top of them. */
	quietMiddle: boolean;
}

interface Lane {
	position: number;
	sway: number;
	wavelength: number;
	phase: number;
	gap: number;
	offset: number;
	color: number;
	fade: number;
}

interface Hop {
	from: Lane;
	to: Lane;
	start: number;
	end: number;
	color: string;
	born: number;
}

interface Pulse {
	lane: Lane;
	along: number;
	speed: number;
	color: string;
	nextCommit: number;
	hop?: Hop;
	trail: { across: number; along: number }[];
}

interface Ring {
	across: number;
	along: number;
	color: string;
	born: number;
}

// The graph's lane colors. Light mode darkens them to hold up on white.
const THEMES = {
	dark: {
		lanes: ['#4bc0eb', '#7bd938', '#d16ba5', '#e5b567', '#929291', '#9d8cf2'],
		laneAlpha: 0.3,
	},
	light: {
		lanes: ['#006698', '#3f8f12', '#b03a7a', '#a8741a', '#85898f', '#6a4fc8'],
		laneAlpha: 0.24,
	},
};

const HOP_CHANCE = 0.3;
const HOP_LIFETIME = 4;
const RING_LIFETIME = 1.4;

const random = (min: number, max: number) => min + Math.random() * (max - min);
const smoothstep = (t: number) => t * t * (3 - 2 * t);
const theme = () => THEMES[document.documentElement.dataset.theme === 'light' ? 'light' : 'dark'];

export function startCommitLanes(canvas: HTMLCanvasElement, options: CommitLaneOptions) {
	const context = canvas.getContext('2d')!;
	const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');

	let across = 0;
	let along = 0;
	let background = '';
	let lanes: Lane[] = [];
	let pulses: Pulse[] = [];
	let hops: Hop[] = [];
	let rings: Ring[] = [];
	let frame = 0;
	let lastFrame = 0;
	let time = 0;

	const scrollOffset = () => (reducedMotion.matches ? 0 : window.scrollY * options.parallax);
	const pulseCount = () => Math.ceil(lanes.length / 2) + 1;
	const readBackground = () =>
		getComputedStyle(document.documentElement).getPropertyValue(options.background);

	// The third harmonic flattens the wave's peaks, so a lane runs straight
	// for a while and then bends, like a lane in the graph.
	function lanePosition(lane: Lane, point: number) {
		const u = point / lane.wavelength + lane.phase + time * 0.03;
		return lane.position + lane.sway * (Math.sin(u) + Math.sin(3 * u) / 3);
	}

	function hopPosition(hop: Hop, point: number) {
		const t = (hop.start - point) / (hop.start - hop.end);
		const from = lanePosition(hop.from, point);
		return from + (lanePosition(hop.to, point) - from) * smoothstep(Math.min(Math.max(t, 0), 1));
	}

	function commitAhead(lane: Lane, point: number) {
		return Math.ceil((point - lane.offset) / lane.gap - 1) * lane.gap + lane.offset;
	}

	function spawn(pulse: Partial<Pulse> = {}): Pulse {
		const lane = lanes[Math.floor(random(0, lanes.length))];
		const start = scrollOffset() + along + random(0, along);
		return {
			lane,
			along: start,
			speed: random(...options.speed),
			color: theme().lanes[lane.color],
			nextCommit: commitAhead(lane, start),
			trail: [],
			...pulse,
		};
	}

	// Lanes keep their places when only their length changes, as it does when
	// a phone's address bar slides away.
	function resize() {
		const ratio = Math.min(window.devicePixelRatio, 2);
		const size = [canvas.clientWidth, canvas.clientHeight];
		const [newAcross, newAlong] = options.horizontal ? size.toReversed() : size;
		const acrossChanged = newAcross !== across;
		across = newAcross;
		along = newAlong;
		canvas.width = size[0] * ratio;
		canvas.height = size[1] * ratio;
		if (options.horizontal) context.setTransform(0, ratio, -ratio, 0, along * ratio, 0);
		else context.setTransform(ratio, 0, 0, ratio, 0, 0);
		if (!acrossChanged) return;

		const { laneSpacing, minLanes, quietMiddle } = options;
		const count = Math.max(Math.round(across / laneSpacing), minLanes);
		const column = Math.min(across / 2, 400);
		lanes = Array.from({ length: count }, (_, i) => {
			const position =
				((i + 0.5) / count) * across + random(-laneSpacing, laneSpacing) * 0.12;
			return {
				position,
				sway: random(...options.sway),
				wavelength: random(...options.wavelength),
				phase: random(0, Math.PI * 2),
				gap: random(...options.commitGap),
				offset: random(0, 300),
				color: i % THEMES.dark.lanes.length,
				fade: quietMiddle ? 0.45 + 0.55 * Math.min(Math.abs(position - across / 2) / column, 1) : 1,
			};
		});
		pulses = Array.from({ length: pulseCount() }, () => spawn());
		hops = [];
		rings = [];
	}

	function advance(pulse: Pulse, delta: number, offset: number) {
		pulse.along -= pulse.speed * delta;

		if (pulse.hop && pulse.along <= pulse.hop.end) {
			pulse.lane = pulse.hop.to;
			pulse.hop = undefined;
			pulse.nextCommit = commitAhead(pulse.lane, pulse.along);
			rings.push({
				across: lanePosition(pulse.lane, pulse.along),
				along: pulse.along,
				color: pulse.color,
				born: time,
			});
		}

		if (!pulse.hop && pulse.along <= pulse.nextCommit) {
			const commit = pulse.nextCommit;
			pulse.nextCommit -= pulse.lane.gap;
			rings.push({
				across: lanePosition(pulse.lane, commit),
				along: commit,
				color: pulse.color,
				born: time,
			});

			const index = lanes.indexOf(pulse.lane);
			const neighbor = lanes[index + (Math.random() < 0.5 ? -1 : 1)];
			if (neighbor && Math.random() < HOP_CHANCE) {
				const hop = {
					from: pulse.lane,
					to: neighbor,
					start: commit,
					end: commit - options.hopLength,
					color: pulse.color,
					born: time,
				};
				pulse.hop = hop;
				hops.push(hop);
			}
		}

		pulse.trail.push({
			across: pulse.hop ? hopPosition(pulse.hop, pulse.along) : lanePosition(pulse.lane, pulse.along),
			along: pulse.along,
		});
		while (pulse.trail[0].along - pulse.along > options.trailLength) pulse.trail.shift();

		// Past the end, or left far behind by a jump in the scroll position.
		const gone = pulse.along < offset - options.trailLength || pulse.along > offset + along * 3;
		return gone ? spawn({ along: offset + along + random(0, along * 0.75) }) : pulse;
	}

	function drawLanes(offset: number) {
		const { lanes: colors, laneAlpha } = theme();
		const { lineWidth, commitRadius } = options;
		context.lineWidth = lineWidth;
		context.lineCap = 'round';

		for (const lane of lanes) {
			context.strokeStyle = colors[lane.color];
			context.globalAlpha = laneAlpha * lane.fade;
			context.beginPath();
			for (let point = -12; point <= along + 12; point += 8) {
				context.lineTo(lanePosition(lane, point + offset), point);
			}
			context.stroke();

			// Commits: a small ring that breaks the line.
			const first = Math.ceil((offset - lane.offset) / lane.gap);
			for (let k = first; k * lane.gap + lane.offset - offset <= along + 6; k++) {
				const commit = k * lane.gap + lane.offset;
				context.beginPath();
				context.arc(lanePosition(lane, commit), commit - offset, commitRadius, 0, Math.PI * 2);
				context.globalAlpha = 1;
				context.fillStyle = background;
				context.fill();
				context.globalAlpha = Math.min(laneAlpha * 2.2, 1) * lane.fade;
				context.stroke();
			}
		}
	}

	function drawHops(offset: number) {
		hops = hops.filter((hop) => time - hop.born < HOP_LIFETIME);
		context.lineWidth = options.lineWidth;
		for (const hop of hops) {
			const age = (time - hop.born) / HOP_LIFETIME;
			context.strokeStyle = hop.color;
			context.globalAlpha = 0.5 * (1 - age);
			context.beginPath();
			for (let point = hop.start; point >= hop.end; point -= 6) {
				context.lineTo(hopPosition(hop, point), point - offset);
			}
			context.stroke();
		}
	}

	function drawRings(offset: number) {
		const { commitRadius, ringGrowth, lineWidth } = options;
		rings = rings.filter((ring) => time - ring.born < RING_LIFETIME);
		context.lineWidth = lineWidth * 1.2;
		for (const ring of rings) {
			const age = (time - ring.born) / RING_LIFETIME;
			const radius = commitRadius + ringGrowth * smoothstep(age);
			context.strokeStyle = ring.color;
			context.fillStyle = ring.color;
			context.globalAlpha = 0.8 * (1 - age);
			context.beginPath();
			context.arc(ring.across, ring.along - offset, radius, 0, Math.PI * 2);
			context.stroke();
			context.beginPath();
			context.arc(ring.across, ring.along - offset, commitRadius, 0, Math.PI * 2);
			context.fill();
		}
	}

	function drawPulses(offset: number) {
		const { glowRadius, lineWidth } = options;
		context.lineWidth = lineWidth * 1.6;
		for (const { trail, color, lane } of pulses) {
			const head = trail.at(-1);
			if (!head) continue;
			const tail = trail[0];
			const headAlong = head.along - offset;

			const fade = context.createLinearGradient(0, tail.along - offset, 0, headAlong);
			fade.addColorStop(0, `${color}00`);
			fade.addColorStop(1, color);
			context.strokeStyle = fade;
			context.globalAlpha = 0.5 + 0.5 * lane.fade;
			context.beginPath();
			for (const point of trail) context.lineTo(point.across, point.along - offset);
			context.stroke();

			const glow = context.createRadialGradient(
				head.across,
				headAlong,
				0,
				head.across,
				headAlong,
				glowRadius,
			);
			glow.addColorStop(0, `${color}cc`);
			glow.addColorStop(0.25, `${color}55`);
			glow.addColorStop(1, `${color}00`);
			context.fillStyle = glow;
			context.fillRect(
				head.across - glowRadius,
				headAlong - glowRadius,
				glowRadius * 2,
				glowRadius * 2,
			);
		}
	}

	function draw() {
		const offset = scrollOffset();
		context.clearRect(0, 0, across, along);
		drawLanes(offset);
		drawHops(offset);
		drawRings(offset);
		drawPulses(offset);
		context.globalAlpha = 1;
	}

	function tick(now: number) {
		// A long gap means the tab was busy. Pulses shouldn't jump across it.
		const delta = Math.min((now - lastFrame) / 1000, 0.05);
		lastFrame = now;
		time += delta;
		const offset = scrollOffset();
		pulses = pulses.map((pulse) => advance(pulse, delta, offset));
		draw();
		frame = requestAnimationFrame(tick);
	}

	function restart() {
		cancelAnimationFrame(frame);
		if (reducedMotion.matches) {
			pulses = [];
			hops = [];
			rings = [];
			draw();
		} else if (!document.hidden) {
			if (!pulses.length) pulses = Array.from({ length: pulseCount() }, () => spawn());
			lastFrame = performance.now();
			frame = requestAnimationFrame(tick);
		}
	}

	function recolor() {
		const { lanes: colors } = theme();
		background = readBackground();
		pulses = pulses.map((pulse) => ({ ...pulse, color: colors[pulse.lane.color] }));
		hops = [];
		rings = [];
		if (reducedMotion.matches) draw();
	}

	new ResizeObserver(() => {
		resize();
		restart();
	}).observe(canvas);
	document.addEventListener('visibilitychange', restart);
	reducedMotion.addEventListener('change', restart);
	new MutationObserver(recolor).observe(document.documentElement, { attributeFilter: ['data-theme'] });

	background = readBackground();
	resize();
	restart();
}
