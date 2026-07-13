import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { tanstackRouter } from '@tanstack/router-plugin/vite';

export default defineConfig({
	plugins: [
		// Must run before vite-plugin-solid: generates src/routeTree.gen.ts
		// from the file-based routes in src/routes/*.
		tanstackRouter({ target: 'solid', autoCodeSplitting: true }),
		solid()
	],
	server: {
		proxy: {
			'/api': 'http://127.0.0.1:3000',
			'/health': 'http://127.0.0.1:3000'
		}
	}
});
