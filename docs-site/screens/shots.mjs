// Every image in src/assets/screens/: which capture it draws, what part of
// the screen, and the highlights and callouts on top.
//
// view       omitted: the whole 1920×1080 screen in a terminal window.
//            { region, maxScale?, legend?: 'right' | 'bottom' }: zoom into
//            `region` (a target, below) on a plain backdrop. The callouts
//            go beside or below it, whichever lets the zoom be bigger, and
//            their targets get numbered markers.
// dim        darken everything except highlights and callout targets.
// legend     { x, y?, width? }: where callouts stack on a full-screen shot.
// callouts   { target, text, markerSide?, tone? }, or with `side`
//            ('right' | 'left' | 'above' | 'below', plus dx/dy/gap) to sit
//            beside the target with a leader line instead of in the legend.
//            text may use **bold**.
// highlights { target, label?, tone? }: an outline, optionally labelled.
//
// Targets, in cells of the 190×51 screen:
//   { pane: 'graph' | 'status' | 'branches' | 'details' }
//   { cells: [x, y, w, h] }
//   { find: 'text', in?: pane, nth?, row?, w?, h?, dx?, dy? }
//   { box: 'title', w?, h? }   the dialog whose top border starts with the title

// Over the empty middle of the graph, on full-screen shots.
const MID = { x: 770, width: 660 };
// Beside a dialog in the middle of the screen.
const BESIDE_DIALOG = { x: 1330, width: 560 };

export const shots = [
	// ---- landing page and tour ------------------------------------------------
	{ name: 'hero', capture: 'overview' },
	{
		name: 'tour-overview',
		capture: 'overview',
		highlights: [
			{ target: { pane: 'graph' } },
			{ target: { pane: 'status' }, tone: 'green' },
			{ target: { pane: 'branches' }, tone: 'amber' },
			{ target: { cells: [0, 50, 190, 1] }, tone: 'pink' },
		],
		legend: { ...MID, y: 150 },
		callouts: [
			{ target: { find: '[1] Graph' }, text: '**Graph**: every branch\'s history, newest first', markerSide: 'above' },
			{ target: { find: '[2] Status' }, text: '**Status**: your changes, and staging them', tone: 'green', markerSide: 'above' },
			{ target: { find: '[3] Branches' }, text: '**Branches**: branches, tags and stashes', tone: 'amber', markerSide: 'above' },
			{ target: { cells: [95, 35, 1, 1] }, box: false, marker: false, text: '**Details** opens in place of this bottom row' },
			{ target: { find: 'enter files + diff' }, text: '**Help bar**: keys for the pane you are in', tone: 'pink', markerSide: 'above' },
		],
	},
	{
		name: 'tour-graph',
		capture: 'overview',
		view: { region: { cells: [0, 0, 64, 5] } },
		callouts: [
			{ target: { find: '[1] Graph [all] 1/47' }, text: '**Title**: all branches; commit 1 of 47', markerSide: 'right' },
			{ target: { find: 'feature/notifications ', nth: 0, dx: -1, w: 23 }, text: '**Checked out**: on green', markerSide: 'top-right' },
			{ target: { find: '[main]' }, text: '**[local]** branch', markerSide: 'below' },
			{ target: { find: '[origin/main]' }, text: '**[origin/…]** remote branch', markerSide: 'below' },
			{ target: { find: '<v1.5.0-beta.1>' }, text: '**<tag>**', markerSide: 'below' },
		],
	},
	{
		name: 'status-pane',
		capture: 'overview',
		view: { region: { cells: [0, 35, 95, 8] } },
		callouts: [
			{ target: { find: '→ origin/feature/notifications ↑1', in: 'status' }, text: '**→ upstream**, and ↑ ahead or ↓ behind it', markerSide: 'above' },
			{ target: { find: 'R  ', in: 'status', w: 1 }, text: '**R** staged rename, old → new', markerSide: 'left' },
			{ target: { find: ' M package', in: 'status', dx: 1, w: 1 }, text: '**M** on the right: changed, not staged', markerSide: 'left' },
			{ target: { find: 'M  src/api', in: 'status', w: 1 }, text: '**M** on the left: staged', markerSide: 'left' },
			{ target: { find: 'MM', in: 'status' }, text: '**MM** staged, then changed again', markerSide: 'left' },
			{ target: { find: '??', in: 'status' }, text: '**??** untracked', markerSide: 'left' },
		],
	},
	{
		name: 'branches-pane',
		capture: 'branches',
		view: { region: { pane: 'branches' } },
		callouts: [
			{ target: { find: 'Branches 15', in: 'branches' }, text: '**Tabs**, each with a count; ] and [ switch', markerSide: 'above' },
			{ target: { find: '* feature/notifications', in: 'branches', w: 1 }, text: '**Checked out**: marked with *', markerSide: 'left' },
			{ target: { cells: [172, 36, 8, 13] }, text: '**Status column**: ✓ ↑ ↓ local origin gone…', markerSide: 'left' },
			{ target: { cells: [183, 36, 6, 13] }, text: '**Last activity**, newest first', markerSide: 'right' },
		],
	},
	{
		name: 'help-bar',
		capture: 'after-commit',
		view: { region: { cells: [96, 48, 94, 3] } },
		callouts: [
			{ target: { find: 'tab next pane', w: 29 }, text: '**Keys** for the focused pane, then global ones', markerSide: 'above' },
			{ target: { find: '✓ commit' }, text: '**Notices**: what just happened (✗ for errors)', tone: 'green', markerSide: 'above' },
		],
	},

	// ---- graph -----------------------------------------------------------------
	{
		name: 'graph-overview',
		capture: 'overview',
		dim: true,
		highlights: [{ target: { pane: 'graph' } }],
		legend: { ...MID, y: 110 },
		callouts: [
			{ target: { find: '[1] Graph [all] 1/47' }, text: '**Title**: what is shown, and where you are', markerSide: 'right' },
			{ target: { cells: [1, 1, 9, 33] }, text: '**Lanes**: one column per line of history', markerSide: 'below' },
			{ target: { find: '[origin/feature/notifications]' }, text: '**Badges**: branches and tags', markerSide: 'left' },
			{ target: { find: 'Poll for new notifications' }, text: '**Subject**: the message\'s first line', markerSide: 'right' },
			{ target: { cells: [158, 1, 31, 33] }, text: '**Author, age, short hash**', markerSide: 'left' },
			{ target: { find: 'Mark all as read' }, text: '**Selected**: space for details, enter for files', markerSide: 'right' },
		],
	},
	{
		name: 'graph-lanes',
		capture: 'overview',
		view: { region: { cells: [0, 8, 44, 13] } },
		callouts: [
			{ target: { find: '●', nth: 8 }, text: '**●** a commit', markerSide: 'right' },
			{ target: { find: '◉', nth: 0 }, text: '**◉** a merge commit', markerSide: 'left' },
			{ target: { find: '─┼─╯', dx: 2, w: 2 }, text: '**─╯** a lane ends: it joins here', markerSide: 'above' },
			{ target: { find: '◉─┴─╮', dx: 3, w: 2 }, text: '**─╮** a merge opens a lane', markerSide: 'below' },
			{ target: { find: '[fix/search-debounce]' }, text: 'Each tip gets the **next color**', markerSide: 'right' },
		],
	},
	{
		name: 'graph-badges',
		capture: 'overview',
		view: { region: { cells: [0, 0, 56, 5] } },
		callouts: [
			{ target: { find: 'feature/notifications ', nth: 0, dx: -1, w: 23 }, text: '**Checked out**: green, no brackets', markerSide: 'top-right' },
			{ target: { find: '[main]' }, text: '**[local]** branch', markerSide: 'below' },
			{ target: { find: '[origin/main]' }, text: '**[origin/…]** remote branch', markerSide: 'below' },
			{ target: { find: '<v1.5.0-beta.1>' }, text: '**<tag>**', markerSide: 'below' },
		],
	},
	{
		name: 'graph-filter',
		capture: 'graph-filter',
		dim: true,
		highlights: [{ target: { box: 'Filter graph' } }],
		legend: { ...BESIDE_DIALOG, y: 150 },
		callouts: [
			{ target: { find: 'Author: any · File: any' }, text: '**What is set**; tab switches Author / File', markerSide: 'left' },
			{ target: { find: '5/5' }, text: '**Type to narrow**: matches / total', markerSide: 'left' },
			{ target: { cells: [125, 11, 2, 5] }, text: '**Commits** by each author', markerSide: 'left' },
			{ target: { find: 'enter filter · tab author/file' }, text: '**enter** filters, esc cancels', markerSide: 'left' },
		],
	},
	{
		name: 'graph-filter-message',
		capture: 'graph-filter-message',
		view: { region: { box: 'Filter commits by message' } },
		callouts: [
			{ target: { find: 'Search in: Subject' }, text: '**ctrl+t**: subject, whole message or diff', markerSide: 'right' },
			{ target: { find: '^feat · ^fix · ^docs' }, text: '**tab** fills in the next preset', markerSide: 'right' },
			{ target: { find: '^fix█' }, text: 'A **regex** on the subject; edit it before enter', markerSide: 'left' },
			{ target: { find: 'regex on the subject · ignores case' }, text: 'Whether it ignores case, or what is wrong', markerSide: 'left' },
			{ target: { find: 'enter filter · tab presets' }, text: '**enter** filters, esc cancels', markerSide: 'left' },
		],
	},
	{
		name: 'graph-filter-diff',
		capture: 'graph-filter-diff',
		view: { region: { cells: [0, 0, 120, 6] } },
		callouts: [
			{ target: { find: 'diff /notification] 1/4' }, text: 'Commits whose **diff** adds or removes a match', markerSide: 'right' },
		],
	},
	{
		name: 'graph-filter-folder',
		capture: 'graph-filter-folder',
		view: { region: { cells: [0, 0, 70, 12] } },
		callouts: [
			{ target: { find: 'folder src/pages/', dx: -2, w: 20 }, text: 'The **filter**, in the title; esc clears it', markerSide: 'above' },
			{ target: { cells: [1, 1, 5, 10] }, text: 'Lanes still **join up** across matches', markerSide: 'right' },
		],
	},
	{
		name: 'graph-new-branch',
		capture: 'graph-new-branch',
		view: { region: { box: 'New branch at' } },
		callouts: [
			{ target: { find: 'New branch at', w: 21 }, text: 'Starts at the **selected commit**', markerSide: 'above' },
			{ target: { find: 'Branch · Worktree' }, text: '**tab**: here, or in a worktree', markerSide: 'left' },
			{ target: { find: 'fix/bell-a11y█' }, text: 'The new branch\'s **name**', markerSide: 'left' },
		],
	},
	{
		name: 'graph-zoom',
		capture: 'graph-zoom-details',
		highlights: [{ target: { pane: 'graph' } }, { target: { pane: 'details' }, tone: 'green' }],
		legend: { ...MID, y: 200 },
		callouts: [
			{ target: { find: '[1] Graph' }, box: false, text: '**+** puts the graph fullscreen', markerSide: 'right' },
			{ target: { find: '[4] Commit' }, box: false, text: '**space** opens details below it, full width', markerSide: 'right', tone: 'green' },
		],
	},
	{
		name: 'status-zoom',
		capture: 'status-zoom',
		highlights: [{ target: { pane: 'status' } }, { target: { pane: 'graph' }, tone: 'green' }, { target: { pane: 'branches' }, tone: 'green' }],
		legend: { ...MID, y: 200 },
		callouts: [
			{ target: { find: '[2] Status' }, box: false, text: '**+** on status: a column down the left', markerSide: 'right' },
			{ target: { find: '[1] Graph' }, box: false, text: 'Graph and branches move beside it', markerSide: 'right', tone: 'green' },
		],
	},
	{
		name: 'graph-merge',
		capture: 'graph-merge-confirm',
		view: { region: { box: 'Merge' } },
		callouts: [
			{ target: { find: 'feature/profile-page into', w: 20 }, text: 'The **branch** at the selected commit (or its hash)', markerSide: 'above' },
			{ target: { find: 'y confirm · n/esc cancel' }, text: '**y** merges; n or esc backs out', markerSide: 'left' },
		],
	},
	{
		name: 'graph-reset',
		capture: 'graph-reset',
		view: { region: { box: 'Reset feature' } },
		callouts: [
			{ target: { find: 'Soft: keep the changes, staged', w: 30 }, text: '**Soft** only moves the branch', markerSide: 'left' },
			{ target: { find: 'Mixed: keep the changes, unstaged', w: 33 }, text: "**Mixed**, git's default, unstages them", markerSide: 'left' },
			{ target: { find: 'Hard: throw the changes away', w: 28 }, text: '**Hard** asks again first', markerSide: 'left' },
		],
	},
	{
		name: 'graph-reset-hard',
		capture: 'graph-reset-hard',
		view: { region: { box: 'Hard reset' } },
		callouts: [
			{ target: { find: 'lost for good' }, text: "What **undo can't** bring back", markerSide: 'above' },
			{ target: { find: '3 commits leave' }, text: 'What **z** can', markerSide: 'left' },
			{ target: { find: 'y confirm · n/esc cancel' }, text: '**y** resets; n or esc backs out', markerSide: 'left' },
		],
	},
	{
		name: 'graph-cherry-pick-range',
		capture: 'graph-cherry-pick-range',
		view: { region: { cells: [0, 0, 190, 28] } },
		callouts: [
			{ target: { find: '· 3 selected' }, text: 'How many commits the **range** covers', markerSide: 'right' },
			{ target: { cells: [1, 6, 60, 3] }, text: '**v**, then j: every commit in the range is highlighted', markerSide: 'right' },
			{ target: { box: 'Cherry-pick range' }, text: '**C** applies them oldest first; y confirms', markerSide: 'right' },
		],
	},

	// ---- details -----------------------------------------------------------------
	{
		name: 'details-commit',
		capture: 'graph-details',
		dim: true,
		highlights: [{ target: { pane: 'details' } }],
		legend: { x: 1200, y: 726, width: 680, gap: 10 },
		callouts: [
			{ target: { find: '[4] Commit', w: 18 }, text: '**Title**: the commit\'s short hash', markerSide: 'right' },
			{ target: { find: 'Parents:', w: 50, h: 4 }, text: '**Parents, author, committer**', markerSide: 'right' },
			{ target: { find: '    Poll for new notifications', dx: 4, w: 26 }, text: '**The full message**', markerSide: 'right' },
			{ target: { find: 'src/api/notifications.ts |', w: 60, h: 2 }, text: '**Diffstat**, then the patch', markerSide: 'right' },
		],
	},
	{
		name: 'details-files',
		capture: 'details-files',
		dim: true,
		highlights: [{ target: { pane: 'details' } }],
		legend: { x: 1200, y: 130, width: 680 },
		callouts: [
			{ target: { find: '[4]', w: 30 }, text: '**}** lists the changed files under the diff', markerSide: 'right' },
			{ target: { find: 'Files', in: 'details' }, text: '**Files**: j and k pick one; its diff is above', markerSide: 'right' },
		],
	},
	{
		name: 'details-fullscreen',
		capture: 'graph-files',
		legend: { x: 1200, y: 620, width: 680 },
		callouts: [
			{ target: { find: '[4]', w: 34 }, text: '**enter** on a commit: files and diff, fullscreen', markerSide: 'right' },
			{ target: { find: 'Files', nth: 0, w: 30 }, text: '**Changed files**: j and k pick one', markerSide: 'right' },
			{ target: { find: 'diff --git', w: 60 }, text: '**That file\'s diff**: pgup and pgdn scroll', markerSide: 'left' },
		],
	},
	{
		name: 'details-search',
		capture: 'details-search',
		legend: { x: 1200, y: 620, width: 680 },
		callouts: [
			{ target: { find: '/items 2/5' }, text: '**/** searches the diff: match 2 of 5', markerSide: 'right' },
			{ target: { find: 'const items', w: 11 }, text: '**n** and **N** step through the matches; the current one is picked out', markerSide: 'left' },
		],
	},
	{
		name: 'details-link',
		capture: 'details-link',
		view: { region: { cells: [0, 41, 74, 6] } },
		callouts: [
			{ target: { find: 'https://github.com/lumen-dev/lumen-web/issues/42' }, text: '**o** or ctrl+click: open the link with a plugin that handles it', markerSide: 'below' },
		],
	},

	// ---- status ------------------------------------------------------------------
	{
		name: 'status-commit',
		capture: 'commit',
		dim: true,
		highlights: [{ target: { box: 'Commit message' } }],
		legend: { ...BESIDE_DIALOG, y: 240 },
		callouts: [
			{ target: { find: 'Show the unread count on the bell' }, text: '**The message**: one line', markerSide: 'left' },
			{ target: { find: 'enter confirm · esc cancel' }, text: '**enter** commits what is staged', markerSide: 'left' },
			{ target: { cells: [1, 36, 40, 5] }, text: '**Staged** files (green codes) go in it', tone: 'green', markerSide: 'right' },
		],
	},
	{
		name: 'agent-dialog',
		capture: 'agent-dialog',
		view: { region: { box: 'Ask an agent' } },
		callouts: [
			{ target: { find: 'Explain branch' }, text: '**tab** steps through ready-made actions', markerSide: 'above' },
			{ target: { find: 'start from the bell icon█' }, text: 'Extra instructions, optional', markerSide: 'left' },
			{ target: { find: 'how the feature on this branch works' }, text: 'What the action does', markerSide: 'left' },
		],
	},
	{
		name: 'agent-hunk-staging',
		capture: 'agent-hunk-staging',
		view: { region: { cells: [0, 0, 132, 30] } },
		callouts: [
			{ target: { cells: [1, 9, 64, 5] }, text: 'The lines picked with **v**', markerSide: 'right', tone: 'amber' },
			{ target: { find: 'Ask an agent about src/components/NotificationBell.tsx lines 7-9' }, text: 'The **file and lines** it asks about', markerSide: 'above' },
			{ target: { find: 'Explain change' }, text: '**tab**: actions for a hunk', markerSide: 'left' },
		],
	},
	{
		name: 'agent-hunk-details',
		capture: 'agent-hunk-details',
		view: { region: { cells: [0, 0, 132, 40] } },
		callouts: [
			{ target: { find: '@@ -0,0 +1,28 @@' }, text: 'The **first hunk** from the top of the view', markerSide: 'right', tone: 'amber' },
			{ target: { find: 'Ask an agent about src/api/notifications.ts lines 1-28' }, text: 'The **file and lines** it asks about', markerSide: 'above' },
			{ target: { find: 'why poll every 30 seconds?█' }, text: 'Your question, sent with the hunk', markerSide: 'left' },
		],
	},
	{
		name: 'status-stash',
		capture: 'stash-prompt',
		view: { region: { box: 'Stash' } },
		callouts: [
			{ target: { find: 'Everything · Staged · Unstaged · Keep staged · notifications.md' }, text: '**tab** picks what to stash; last is the selected file', markerSide: 'above' },
			{ target: { find: 'try a pulse animation█' }, text: 'A **message**, optional', markerSide: 'left' },
			{ target: { find: 'unstaged and untracked changes; staged ones stay' }, text: 'What will be stashed', markerSide: 'left' },
			{ target: { find: 'enter stash · tab what to stash' }, text: '**enter** stashes', markerSide: 'left' },
		],
	},
	{
		name: 'status-staged',
		capture: 'status-staged',
		view: { region: { cells: [0, 35, 95, 8] } },
		callouts: [
			{ target: { find: 'M  package.json', in: 'status', w: 1 }, text: '**space** staged it: the M moved left and turned green', tone: 'green', markerSide: 'left' },
		],
	},
	{
		name: 'status-conflict',
		capture: 'conflict',
		view: { region: { cells: [0, 35, 95, 5] } },
		callouts: [
			{ target: { find: 'MERGING →', in: 'status', w: 7 }, text: '**MERGING** in the title', markerSide: 'above' },
			{ target: { find: 'MERGING: stage fixes · C continue · X abort' }, text: '**Banner**: what to do next', markerSide: 'right' },
			{ target: { find: 'M  src/styles', in: 'status', w: 1 }, text: 'Resolved and **staged**', tone: 'green', markerSide: 'left' },
			{ target: { find: 'UU', in: 'status' }, text: '**UU** conflicted: fix it, then space', tone: 'pink', markerSide: 'left' },
		],
	},
	{
		name: 'status-discard',
		capture: 'discard-confirm',
		view: { region: { box: 'Discard' } },
		callouts: [{ target: { find: 'y confirm', w: 24 }, text: '**y** discards; it can\'t be undone', markerSide: 'left', tone: 'pink' }],
	},
	{
		name: 'status-discard-all',
		capture: 'discard-all',
		view: { region: { box: 'Discard all' } },
		callouts: [
			{ target: { find: 'Everything · Unstaged' }, text: '**tab** picks what to discard', markerSide: 'left' },
			{ target: { find: 'Lost:', w: 5 }, text: '**Lost** and **Kept**: counted for you', markerSide: 'left' },
			{ target: { find: 'y discard', w: 9 }, text: '**y** discards; it can\'t be undone', markerSide: 'left', tone: 'pink' },
		],
	},
	{
		name: 'status-tree',
		capture: 'status-tree',
		view: { region: { cells: [0, 35, 95, 13] } },
		callouts: [
			{ target: { find: '▾ src/', in: 'status', w: 6 }, text: '**▾** an open folder; **enter** folds it', markerSide: 'right' },
			{ target: { find: 'M  notifications.ts', in: 'status' }, text: 'Each file by its **name**, under its folder', markerSide: 'right' },
			{ target: { find: '▾ styles/', in: 'status', w: 9 }, text: '**space**, **d**, **i** on a folder act on all of it', markerSide: 'right' },
		],
	},
	{
		name: 'status-tree-discard',
		capture: 'status-tree-discard',
		view: { region: { box: 'Discard all in src/styles/' } },
		callouts: [
			{ target: { find: 'Discard all in src/styles/' }, text: '**d** on a folder: discard only the changes under it', markerSide: 'above' },
			{ target: { find: 'everything outside src/styles/.' }, text: 'The rest of the tree is **kept**', markerSide: 'left' },
		],
	},
	{
		name: 'status-ignore',
		capture: 'ignore-picker',
		view: { region: { box: 'Ignore' } },
		callouts: [
			{ target: { find: 'Add /src/styles/bell.css to .gitignore', w: 38 }, text: '**The file**, anchored to the root', markerSide: 'left' },
			{ target: { find: 'Add /src/styles/ to .gitignore', w: 30 }, text: 'Or **the folder** it is in', markerSide: 'left' },
			{ target: { find: 'this clone only' }, text: '**.git/info/exclude** stays in this clone', markerSide: 'right' },
		],
	},

	// ---- staging -------------------------------------------------------------------
	{
		name: 'staging',
		capture: 'staging',
		legend: { x: 1200, y: 330, width: 680 },
		callouts: [
			{ target: { find: '[4] Stage', w: 52 }, text: '**Title**: the file; hunks or lines', markerSide: 'right' },
			{ target: { find: '▌Unstaged  2 hunks' }, text: '**▌** the side you are on; tab switches', markerSide: 'right' },
			{ target: { cells: [1, 2, 92, 18] }, text: '**Selected hunk**: space stages it', markerSide: 'right' },
			{ target: { find: '@@ -16,9 +17,9 @@' }, text: '**@@** the next hunk; j and k move', markerSide: 'right' },
		],
	},
	{
		name: 'staging-lines',
		capture: 'staging-lines',
		view: { region: { cells: [0, 0, 72, 21] } },
		callouts: [
			{ target: { find: '· lines', w: 7 }, text: '**a** switched to lines', markerSide: 'right' },
			{ target: { cells: [1, 9, 64, 6] }, text: '**v**, then j: a **range**; space stages it', markerSide: 'right' },
			{ target: { find: '+import { useNotifications }', w: 62 }, text: 'Lines you didn\'t pick **stay unstaged**', markerSide: 'right', tone: 'amber' },
		],
	},
	{
		name: 'staging-both',
		capture: 'staging-staged',
		legend: { x: 600, y: 620, width: 720 },
		callouts: [
			{ target: { find: 'Unstaged  1 hunk', dx: -1, w: 17 }, text: '**Unstaged** changes on the left', markerSide: 'right' },
			{ target: { find: '▌Staged  1 hunk' }, text: '**Staged** on the right: space there unstages', markerSide: 'right', tone: 'green' },
		],
	},
	// ---- blame ----------------------------------------------------------------
	{
		name: 'blame-gutter',
		capture: 'blame',
		view: { region: { cells: [0, 0, 120, 14] } },
		callouts: [
			{ target: { find: '▌', nth: 0, h: 13 }, text: '**When**: hot is recent, cold is old', markerSide: 'below', tone: 'amber' },
			{ target: { find: ' 1w ▌', nth: 0, dx: 2, w: 2 }, text: '**When**: its age, or with t the date', markerSide: 'above', tone: 'amber' },
			{ target: { find: 'Mia Chen', nth: 0 }, text: '**Who**: on every line, a color each', markerSide: 'above', tone: 'green' },
			{ target: { find: '31a324f  Refresh', w: 39 }, text: '**Why**: the commit, where its lines start', markerSide: 'above' },
		],
	},
	{ name: 'blame-overview', capture: 'blame' },
	{ name: 'blame-details', capture: 'blame-details' },
	{
		name: 'blame-commit',
		capture: 'blame-commit',
		legend: { x: 1200, y: 700, width: 680 },
		callouts: [
			{ target: { find: '‹ Blame src/api/client.ts' , w: 52 }, text: '**esc** goes back here, to line 33', markerSide: 'below' },
			{ target: { find: 'Files (2)', w: 26, h: 3 }, text: '**Everything** the commit changed', markerSide: 'right' },
			{ target: { find: '+      signal: opts.signal,', w: 159 }, text: '**The blamed line**, picked out', markerSide: 'left', tone: 'green' },
		],
	},
	{ name: 'blame-step-back', capture: 'blame-step-back' },
	{ name: 'blame-search', capture: 'blame-search' },
	{
		name: 'staging-discard',
		capture: 'staging-discard',
		view: { region: { box: 'Discard changes' } },
		callouts: [{ target: { find: 'Discard the selected hunk of', w: 56, h: 3 }, text: 'Discards just the **selected hunk** from your files', markerSide: 'left', tone: 'pink' }],
	},

	// ---- branches ------------------------------------------------------------------
	{
		name: 'branches-overview',
		capture: 'branches',
		dim: true,
		highlights: [{ target: { pane: 'branches' } }],
		legend: { ...MID, y: 150 },
		callouts: [
			{ target: { find: '* feature/notifications', in: 'branches', w: 30, h: 13 }, text: '**One row per branch**: main and origin/main are one row', markerSide: 'left' },
			{ target: { cells: [172, 36, 10, 13] }, text: '**Status**: how each branch relates to its remote', markerSide: 'left' },
			{ target: { cells: [183, 36, 6, 13] }, text: '**Last activity**: commit, fetch or checkout; newest first', markerSide: 'right' },
		],
	},
	{
		name: 'branches-states',
		capture: 'branches',
		view: { region: { cells: [157, 35, 33, 15] } },
		callouts: [
			{ target: { find: '↑1', in: 'branches' }, text: '**↑1** ahead: push it', markerSide: 'left' },
			{ target: { find: '✓', in: 'branches' }, text: '**✓** in sync with its upstream', markerSide: 'left' },
			{ target: { find: 'origin  ', in: 'branches', w: 6 }, text: '**origin** only on the remote', markerSide: 'left' },
			{ target: { find: 'gone', in: 'branches' }, text: '**gone** deleted on the remote', markerSide: 'left', tone: 'pink' },
			{ target: { find: '=origin', in: 'branches' }, text: '**=origin** same commit, not tracking', markerSide: 'left' },
			{ target: { find: 'local', in: 'branches' }, text: '**local** never pushed', markerSide: 'left' },
			{ target: { find: '≠origin', in: 'branches' }, text: '**≠origin** different, not tracking', markerSide: 'left' },
			{ target: { find: '↓1', in: 'branches' }, text: '**↓1** behind: pull it', markerSide: 'left' },
		],
	},
	{
		name: 'branches-new',
		capture: 'branches-new',
		view: { region: { box: 'New branch from' } },
		callouts: [
			{ target: { find: 'Branch · Worktree' }, text: '**tab** picked Worktree (underlined)', markerSide: 'left' },
			{ target: { find: 'fix/footer-links█' }, text: 'The branch **name**', markerSide: 'left' },
			{ target: { find: 'checked out in .worktrees/fix/footer-links' }, text: 'Where the **worktree** goes', markerSide: 'left' },
		],
	},
	{
		name: 'branches-delete',
		capture: 'branches-delete',
		view: { region: { box: 'Delete branch' } },
		callouts: [
			{ target: { find: '(If it is not fully merged', w: 54, h: 2 }, text: 'Unmerged? It asks **again** before force-deleting', markerSide: 'left' },
			{ target: { find: 'y confirm · n/esc cancel' }, text: '**y** deletes', markerSide: 'left', tone: 'pink' },
		],
	},
	{
		name: 'branches-prune-gone',
		capture: 'branches-prune-gone',
		view: { region: { box: 'Delete gone branches' } },
		callouts: [
			{ target: { find: '(If any are not fully merged', w: 54, h: 2 }, text: 'Unmerged? It asks **again** before force-deleting', markerSide: 'left' },
			{ target: { find: 'y confirm · n/esc cancel' }, text: '**y** deletes them', markerSide: 'left', tone: 'pink' },
		],
	},
	{
		name: 'branches-filter',
		capture: 'branches-filter',
		view: { region: { cells: [95, 35, 95, 3] } },
		callouts: [
			{ target: { find: '/fpp', in: 'branches' }, text: '**/** filters as you type: fpp finds feature/profile-page', markerSide: 'above' },
		],
	},

	// ---- merged and unmerged -----------------------------------------------------
	{
		name: 'unmerged',
		capture: 'unmerged',
		legend: { ...MID, y: 190 },
		callouts: [
			{ target: { find: '[1] Graph [origin/feature/profile-page]', w: 44 }, text: 'The graph shows **only the selected branch**', markerSide: 'right' },
			{ target: { find: '(branched off origin/main)' }, text: 'Where it **branched off** the base', markerSide: 'right' },
			{ target: { find: '[main] [origin/main]', dx: -2, w: 24 }, text: 'The **base\'s tip** today', markerSide: 'right' },
			{ target: { find: 'Unmerged 6', in: 'branches' }, text: '**Unmerged** tab, against origin/main', markerSide: 'above' },
			{ target: { find: '  feature/profile-page', in: 'branches', row: true }, text: 'The **selected** branch', markerSide: 'left' },
		],
	},
	{
		name: 'merged-graph',
		capture: 'merged',
		view: { region: { cells: [0, 0, 70, 7] } },
		callouts: [
			{ target: { cells: [1, 5, 68, 1] }, text: 'Where it **branched off**', markerSide: 'right' },
			{ target: { cells: [1, 2, 68, 3] }, text: 'The branch\'s **own commits**', markerSide: 'right' },
			{ target: { cells: [1, 1, 68, 1] }, text: 'The merge that **brought it in**', markerSide: 'right' },
		],
	},
	{
		name: 'merged-tab',
		capture: 'merged',
		view: { region: { cells: [95, 35, 95, 7] } },
		callouts: [
			{ target: { find: '(origin/main)', in: 'branches' }, text: 'The **base branch** both tabs compare with', markerSide: 'above' },
			{ target: { cells: [97, 36, 18, 5] }, text: '**Branches** merged into it', markerSide: 'left' },
			{ target: { cells: [116, 36, 30, 5] }, text: 'Each tip\'s **subject**', markerSide: 'below' },
			{ target: { cells: [168, 36, 21, 5] }, text: '**Author** and **date**, newest first', markerSide: 'below' },
		],
	},
	{
		name: 'base-picker',
		capture: 'base-picker',
		view: { region: { box: 'Base branch' } },
		callouts: [
			{ target: { find: 'Detect automatically', w: 68 }, text: '**Detect automatically**, and what it finds', markerSide: 'left' },
			{ target: { cells: [61, 14, 68, 1] }, text: 'The base **now**', markerSide: 'left' },
		],
	},

	// ---- tags ------------------------------------------------------------------------
	{
		name: 'tags-overview',
		capture: 'tags',
		legend: { ...MID, y: 170 },
		callouts: [
			{ target: { find: 'Tags 7', in: 'branches' }, text: 'The **Tags** tab', markerSide: 'above' },
			{ target: { find: '  v1.4.0', in: 'branches', row: true }, text: 'The **selected** tag…', markerSide: 'left' },
			{ target: { find: '<v1.4.0>', in: 'graph' }, text: '…and its commit, in the **graph**', markerSide: 'right' },
		],
	},
	{
		name: 'tags-distance',
		capture: 'tags',
		view: { region: { cells: [95, 35, 46, 9] } },
		callouts: [
			{ target: { find: '✓', in: 'branches' }, text: '**✓** the base\'s tip: nothing unreleased', markerSide: 'right' },
			{ target: { find: '↓11', in: 'branches' }, text: '**↓11** commits on the base since', markerSide: 'left' },
			{ target: { find: '↑1↓16', in: 'branches' }, text: '**↑1** tagged on a release branch', markerSide: 'left' },
			{ target: { cells: [111, 41, 5, 2] }, text: 'Older tags: **not compared**', markerSide: 'left' },
		],
	},

	{
		name: 'tags-new',
		capture: 'tags-new',
		view: { region: { box: 'New tag at' } },
		callouts: [
			{ target: { find: 'Lightweight · Annotated' }, text: '**tab** picked Annotated (underlined)', markerSide: 'left' },
			{ target: { find: 'v1.6.0█' }, text: 'The tag **name**', markerSide: 'left' },
			{ target: { find: 'asks for a message next' }, text: '**enter** asks for the message', markerSide: 'left' },
		],
	},
	{
		name: 'tags-delete',
		capture: 'tags-delete',
		view: { region: { box: 'Delete tag' } },
		callouts: [
			{ target: { find: 'y confirm' }, text: '**y** deletes it here only', markerSide: 'left' },
			{ target: { find: 'r here and on origin' }, text: '**r** deletes it on the remote too', markerSide: 'right' },
		],
	},

	// ---- stash -----------------------------------------------------------------------
	{
		name: 'stash-tab',
		capture: 'stash-tab',
		view: { region: { cells: [95, 35, 95, 4] } },
		callouts: [
			{ target: { find: 'Stash 2', in: 'branches' }, text: 'The **Stash** tab, once there is a stash', markerSide: 'above' },
			{ target: { find: 'stash@{0}', in: 'branches' }, text: '**stash@{0}** is the newest', markerSide: 'left' },
			{ target: { find: 'On feature/notifications: try a websocket instead of polling', in: 'branches' }, text: 'Its branch and **message**', markerSide: 'right' },
		],
	},
	{
		name: 'worktrees-tab',
		capture: 'worktrees-tab',
		view: { region: { cells: [95, 35, 95, 15] } },
		callouts: [
			{ target: { find: 'Worktrees 3', in: 'branches' }, text: 'The **Worktrees** tab, once there is a linked worktree', markerSide: 'above' },
			{ target: { find: '* .', in: 'branches' }, text: 'The worktree tt-git is **running in**; paths are from here', markerSide: 'left' },
			{ target: { find: '(detached f33b691)', in: 'branches' }, text: 'Its **branch**, or the commit it has detached', markerSide: 'below' },
			{ target: { find: '↑1 · 1 changed', in: 'branches' }, text: '**Commits** the base lacks, and **changed files**', markerSide: 'left' },
			{ target: { find: ' KB', in: 'branches', dx: -4, w: 7 }, text: 'The **disk space** each takes', markerSide: 'right' },
			{ target: { find: 'total', in: 'branches', w: 13 }, text: 'All of them **together**', markerSide: 'below' },
		],
	},
	{
		name: 'worktree-remove',
		capture: 'worktree-remove',
		view: { region: { box: 'Remove worktree' } },
		callouts: [
			{ target: { find: 'Branch fix/bell-a11y stays.' }, text: 'Only the folder goes; the **branch stays**', markerSide: 'above' },
			{ target: { find: '(If it has uncommitted' }, text: 'With changes, it **asks again** before --force', markerSide: 'left' },
		],
	},
	{
		name: 'worktree-switch',
		capture: 'worktree-switch',
		view: { region: { cells: [95, 35, 95, 16] } },
		callouts: [
			{ target: { find: 'fix/bell-a11y', in: 'branches' }, text: '**enter** on a branch another worktree has', markerSide: 'left' },
			{ target: { find: 'fix/bell-a11y is checked out', w: 69 }, text: 'tt-git **opens that worktree** instead of failing', tone: 'green', markerSide: 'above' },
		],
	},
	{
		name: 'stash-show',
		capture: 'stash-show',
		legend: { x: 1200, y: 560, width: 680 },
		callouts: [
			{ target: { find: 'stash@{0}', nth: 0, w: 70 }, text: '**enter** on a stash: what it holds', markerSide: 'right' },
			{ target: { find: 'untracked', nth: 0, w: 20 }, text: '**Untracked files** saved with it', markerSide: 'right' },
		],
	},

	// ---- palette and help --------------------------------------------------------------
	{
		name: 'palette',
		capture: 'palette',
		dim: true,
		highlights: [{ target: { box: 'Commands' } }],
		legend: { ...BESIDE_DIALOG, y: 220 },
		callouts: [
			{ target: { find: '> █' }, text: '**Type** to search by name', markerSide: 'left' },
			{ target: { cells: [61, 11, 50, 4] }, text: 'The **focused pane\'s** commands first', markerSide: 'left' },
			{ target: { cells: [118, 11, 11, 12] }, text: 'The **key** for each, if any', markerSide: 'right' },
		],
	},
	{
		name: 'palette-search',
		capture: 'palette-search',
		view: { region: { box: 'Commands' } },
		callouts: [{ target: { find: '> sta█' }, text: '**sta**: in the name, or as letters in order', markerSide: 'left' }],
	},
	{
		name: 'theme-picker',
		capture: 'theme-picker',
		highlights: [{ target: { box: 'Theme' } }],
		legend: { ...BESIDE_DIALOG, y: 220 },
		callouts: [
			{ target: { find: 'catppuccin-mocha' }, text: 'The highlighted one **shows**', markerSide: 'left' },
			{ target: { find: 'terminal colors' }, text: 'Where each one **comes from**', markerSide: 'right' },
		],
	},
	{
		name: 'theme-light',
		capture: 'theme-light',
	},
	{
		name: 'help',
		capture: 'help',
		dim: true,
		highlights: [{ target: { box: 'Keybindings' } }],
		legend: { ...BESIDE_DIALOG, y: 220 },
		callouts: [
			{ target: { find: '> type to search' }, text: '**?** lists the focused pane\'s keys; type to search', markerSide: 'left' },
		],
	},
	{
		name: 'help-search',
		capture: 'help-search',
		view: { region: { box: 'Keybindings', h: 4 } },
		callouts: [{ target: { find: '> merge' }, text: 'Typing **narrows** the list', markerSide: 'left' }],
	},
	{
		name: 'help-about',
		capture: 'help-about',
		view: { region: { box: 'Keybindings', h: 15 } },
		callouts: [{ target: { find: 'Keys · About' }, text: '**tab** switches to About', markerSide: 'left' }],
	},
	{
		name: 'about',
		capture: 'about',
		view: { region: { box: 'About tt-git' } },
		callouts: [
			{ target: { find: 'Source: https://github.com/thirtytech/tt-git' }, text: 'Plain text: **most terminals** open links on click', markerSide: 'left' },
			{ target: { find: 'Prefer a window? Install the desktop app:' }, text: 'Where to get the **desktop app**', markerSide: 'left' },
		],
	},

	// ---- configuration ---------------------------------------------------------------
	{
		name: 'layout-default',
		capture: 'overview',
		highlights: [
			{ target: { pane: 'graph' }, label: 'graph · size = 70' },
			{ target: { cells: [0, 35, 190, 15] }, label: 'columns · details = true', tone: 'amber' },
		],
	},
	{
		name: 'layout-four-pane',
		capture: 'layout-four-pane',
		highlights: [
			{ target: { pane: 'status' }, label: 'status · 40', tone: 'green' },
			{ target: { pane: 'branches' }, label: 'branches', tone: 'amber' },
			{ target: { pane: 'graph' }, label: 'graph · 65' },
			{ target: { pane: 'details' }, label: 'details', tone: 'pink' },
		],
	},

	// ---- remotes, worktrees -------------------------------------------------------------
	{
		name: 'remotes-pushing',
		capture: 'pushing',
		view: { region: { cells: [96, 48, 94, 3] } },
		callouts: [
			{ target: { find: '⠏ pushing' }, text: '**Running**: what git is doing, and for how long', markerSide: 'above' },
		],
	},
	{
		name: 'remotes-title',
		capture: 'overview',
		view: { region: { cells: [0, 35, 95, 3] } },
		callouts: [
			{ target: { find: 'origin/feature/notifications', in: 'status' }, text: '**→ origin/…**: where push and pull go', markerSide: 'below' },
			{ target: { find: '↑1', in: 'status' }, text: '**↑1**: one commit to push', tone: 'green', markerSide: 'below' },
		],
	},
	{
		name: 'worktree-dialog',
		capture: 'graph-new-worktree',
		view: { region: { box: 'New branch at' } },
		callouts: [
			{ target: { find: 'Branch · Worktree' }, text: '**tab** picked Worktree (underlined)', markerSide: 'left' },
			{ target: { find: 'checked out in .worktrees/fix/bell-a11y' }, text: 'Its folder, under **.worktrees/**', markerSide: 'left' },
		],
	},
];
