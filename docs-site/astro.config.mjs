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
					items: [{ label: 'Welcome', slug: 'index' }],
				},
			],
		}),
	],
});
