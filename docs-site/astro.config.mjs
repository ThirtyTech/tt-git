// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Published on GitHub Pages as a project site, under /tt-git/.
const SITE = 'https://thirtytech.github.io';
const BASE = '/tt-git';

// Astro's `base` doesn't rewrite absolute links written in the pages, so
// `[Graph](/panes/graph/)` and `<LinkCard href="/panes/graph/" />` would
// point outside the site. Prefix them with BASE.
function rehypePrefixBase() {
	const rewrite = (value) =>
		typeof value === 'string' &&
		value.startsWith('/') &&
		!value.startsWith('//') &&
		value !== BASE &&
		!value.startsWith(`${BASE}/`)
			? `${BASE}${value}`
			: value;

	const visit = (node) => {
		if (node.type === 'element' && node.properties) {
			for (const attr of ['href', 'src']) node.properties[attr] = rewrite(node.properties[attr]);
		}
		// Components in MDX, like <LinkCard href="…" />.
		if ((node.type === 'mdxJsxFlowElement' || node.type === 'mdxJsxTextElement') && node.attributes) {
			for (const attr of node.attributes) {
				if (attr.type === 'mdxJsxAttribute' && attr.name === 'href') attr.value = rewrite(attr.value);
			}
		}
		node.children?.forEach(visit);
	};

	return (tree) => visit(tree);
}

// Link previews (Discord, Slack, X…) and icons. Starlight already writes the
// title, description, og:url, og:site_name and twitter:card for each page;
// these add the picture, the icons and the embed color. The images in
// public/ come from `pnpm screens:social`.
const OG_IMAGE = `${SITE}${BASE}/og.png`;
const OG_ALT = 'The tt-git logo next to its commit graph, status and branches panes.';
const head = [
	{ tag: 'meta', attrs: { property: 'og:image', content: OG_IMAGE } },
	{ tag: 'meta', attrs: { property: 'og:image:type', content: 'image/png' } },
	{ tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
	{ tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
	{ tag: 'meta', attrs: { property: 'og:image:alt', content: OG_ALT } },
	{ tag: 'meta', attrs: { name: 'twitter:image', content: OG_IMAGE } },
	{ tag: 'meta', attrs: { name: 'twitter:image:alt', content: OG_ALT } },
	// The color of the bar down the side of a Discord embed.
	{ tag: 'meta', attrs: { name: 'theme-color', content: '#4bc0eb' } },
	{ tag: 'link', attrs: { rel: 'icon', type: 'image/png', sizes: '192x192', href: `${BASE}/icon-192.png` } },
	{ tag: 'link', attrs: { rel: 'apple-touch-icon', sizes: '180x180', href: `${BASE}/apple-touch-icon.png` } },
];

// https://astro.build/config
export default defineConfig({
	site: SITE,
	base: BASE,
	markdown: {
		rehypePlugins: [rehypePrefixBase],
	},
	integrations: [
		starlight({
			title: 'tt-git',
			description:
				'A terminal git client with a GitLens-style commit graph, a lazygit-style status pane and herdr-style plugins.',
			logo: { src: './src/assets/logo.svg' },
			favicon: '/favicon.svg',
			head,
			customCss: ['./src/styles/theme.css'],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/thirtytech/tt-git' }],
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Welcome', slug: 'index' },
						{ label: 'Install', slug: 'getting-started/install' },
						{ label: 'A tour of the screen', slug: 'getting-started/tour' },
						{ label: 'Moving around', slug: 'getting-started/navigation' },
						{ label: 'Your first commit', slug: 'getting-started/first-commit' },
					],
				},
				{
					label: 'Panes',
					items: [
						{ label: 'Graph', slug: 'panes/graph' },
						{ label: 'Details', slug: 'panes/details' },
						{ label: 'Status', slug: 'panes/status' },
						{ label: 'Staging hunks and lines', slug: 'panes/staging' },
						{ label: 'Blame', slug: 'panes/blame' },
						{ label: 'Branches', slug: 'panes/branches' },
						{ label: 'Merged and unmerged', slug: 'panes/merged-unmerged' },
						{ label: 'Tags', slug: 'panes/tags' },
						{ label: 'Stash', slug: 'panes/stash' },
						{ label: 'Plugin panes', slug: 'panes/plugin-panes' },
					],
				},
				{
					label: 'Working with git',
					items: [
						{ label: 'Command palette', slug: 'git/palette' },
						{ label: 'Push, pull and fetch', slug: 'git/remotes' },
						{ label: 'Merge, rebase, cherry-pick and revert', slug: 'git/merge-rebase' },
						{ label: 'Reset', slug: 'git/reset' },
						{ label: 'Undo', slug: 'git/undo' },
						{ label: 'Worktrees', slug: 'git/worktrees' },
						{ label: 'Agent actions', slug: 'git/agents' },
						{ label: 'Refreshing', slug: 'git/refreshing' },
					],
				},
				{
					label: 'Configuration',
					items: [
						{ label: 'Configuration file', slug: 'configuration' },
						{ label: 'Layout', slug: 'configuration/layout' },
						{ label: 'Key bindings', slug: 'configuration/keys' },
						{ label: 'Themes', slug: 'configuration/themes' },
					],
				},
				{
					label: 'Plugins',
					items: [
						{ label: 'Overview', slug: 'plugins' },
						{ label: 'Writing a plugin', slug: 'plugins/manifest' },
						{ label: 'Environment and context', slug: 'plugins/environment' },
						{ label: 'Events', slug: 'plugins/events' },
						{ label: 'CLI and socket API', slug: 'plugins/api' },
						{ label: 'Example plugins', slug: 'plugins/examples' },
						{ label: 'GitHub', slug: 'plugins/github' },
						{ label: 'Azure DevOps', slug: 'plugins/azure-devops' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Keyboard shortcuts', slug: 'reference/keys' },
						{ label: 'Command line', slug: 'reference/cli' },
						{ label: 'How tt-git uses git', slug: 'reference/how-it-works' },
						{ label: 'Memory and speed', slug: 'reference/performance' },
					],
				},
			],
		}),
	],
});
