<script lang="ts">
	import '../app.css';
	import favicon from '$lib/assets/favicon.svg';

	let { children } = $props();
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

<div class="shell">
	{@render children()}
</div>

<style>
	.shell {
		max-width: 900px;
		margin: 0 auto;
		padding: var(--st-space-4);
	}
</style>
