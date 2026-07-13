import { createRootRoute, Outlet } from '@tanstack/solid-router';
import '../app.css';

export const Route = createRootRoute({
	component: RootComponent
});

function RootComponent() {
	return (
		<div class="shell">
			<Outlet />
		</div>
	);
}
