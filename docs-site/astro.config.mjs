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
						{ label: 'Merge, rebase and cherry-pick', slug: 'git/merge-rebase' },
						{ label: 'Worktrees', slug: 'git/worktrees' },
						{ label: 'Refreshing', slug: 'git/refreshing' },
					],
				},
				{
					label: 'Configuration',
					items: [
						{ label: 'Configuration file', slug: 'configuration' },
						{ label: 'Layout', slug: 'configuration/layout' },
						{ label: 'Key bindings', slug: 'configuration/keys' },
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
