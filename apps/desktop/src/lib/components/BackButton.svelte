<script lang="ts">
	export let label = '返回';
	export let title: string | undefined = undefined;
	export let onClick: () => void;
</script>

<button
	type="button"
	class="back-btn"
	on:click={onClick}
	title={title ?? label}
	aria-label={label}
>
	<svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
		<path
			d="M15 6L9 12l6 6"
			stroke="currentColor"
			stroke-width="1.85"
			stroke-linecap="round"
			stroke-linejoin="round"
		/>
	</svg>
</button>

<style>
	.back-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 36px;
		height: 36px;
		padding: 0;
		border: 1px solid color-mix(in srgb, var(--line) 80%, transparent);
		border-radius: 10px;
		background: var(--bg-secondary);
		color: var(--text);
		cursor: pointer;
		flex-shrink: 0;
		transition:
			background 0.15s ease,
			border-color 0.15s ease,
			color 0.15s ease;
	}
	.back-btn:hover {
		color: var(--link);
		border-color: color-mix(in srgb, var(--link) 40%, var(--line));
		background: color-mix(in srgb, var(--link) 14%, var(--bg-secondary));
	}
	.back-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}
</style>
