<script lang="ts">
	export let flowActive = false;
	export let element: HTMLElement;
</script>

<main class="content" class:flow-active={flowActive} bind:this={element}>
	<slot />
</main>

<style>
	.content {
		position: relative;
		flex: 1;
		min-width: 0;
		min-height: 0;
		overflow-y: auto;
		overflow-x: hidden;
		scroll-behavior: smooth;
	}
	.content.flow-active {
		overflow: hidden;
	}
	:global(.app.focus-key-scroll-active) .content {
		scroll-behavior: auto;
	}
</style>
