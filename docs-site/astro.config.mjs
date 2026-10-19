// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	integrations: [
		starlight({
			title: 'tt-git',
			description:
				'A terminal git client with a GitLens-style commit graph, a lazygit-style status pane and herdr-style plugins.',
			logo: { src: './src/assets/logo.svg' },
			favicon: '/favicon.svg',
			customCss: ['./src/styles/theme.css'],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/jsheely/tt-git' }],
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
