/* @refresh reload */
import { render } from 'solid-js/web';
import { createRouter, RouterProvider } from '@tanstack/solid-router';
import { routeTree } from './routeTree.gen';

const router = createRouter({ routeTree });

declare module '@tanstack/solid-router' {
	interface Register {
		router: typeof router;
	}
}

const root = document.getElementById('root');

render(() => <RouterProvider router={router} />, root!);
