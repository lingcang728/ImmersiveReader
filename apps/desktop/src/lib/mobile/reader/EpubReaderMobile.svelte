<!--
	Embedded EPUB reader wrapper for the mobile shell.

	`$lib/epub/EpubReader.svelte` is the single reading kernel (chapter loading,
	locator capture/restore, links, resources, sanitization, bookmarks, search).
	It is imported *dynamically* here so the mobile first-screen bundle never
	pulls the EPUB/DOMPurify path, as required by docs/mobile-redesign.md.
	The wrapper only forwards the imperative calls ReaderScreen chrome needs.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import type { Component } from 'svelte';
	import type { Publication, ReaderLocator, SearchHit } from '$lib/epub/types';
	import type { EpubMeta } from './types';

	interface Props {
		bookId: string;
		bookDir: string;
		publication: Publication;
		initialLocator: ReaderLocator | null;
		onExit: () => void;
		onLocator: (locator: ReaderLocator) => void;
		onMeta: (meta: EpubMeta) => void;
	}

	let { bookId, bookDir, publication, initialLocator, onExit, onLocator, onMeta }: Props = $props();

	// The kernel is a legacy-mode component with exported instance functions;
	// the dynamic component reference is intentionally loosely typed.
	let EpubReaderComponent = $state<Component<any> | null>(null);
	let readerRef = $state<any>(null);
	let ready = $state(false);

	onMount(() => {
		void import('$lib/epub/EpubReader.svelte')
			.then((module) => {
				EpubReaderComponent = module.default as unknown as Component<any>;
			})
			.catch((error) => {
				console.error('[mobile-reader] EPUB kernel failed to load', error);
			});
	});

	function call<T>(name: string, ...args: unknown[]): T | undefined {
		const fn = readerRef?.[name];
		if (typeof fn !== 'function') return undefined;
		return (fn as (...callArgs: unknown[]) => T)(...args);
	}

	export function navigateChapterBy(direction: 1 | -1): void {
		call('navigateChapterBy', direction);
	}

	export function scrollPage(direction: 1 | -1): void {
		call('scrollPage', direction);
	}

	export function goToChapterId(chapterId: string): void {
		call('goToChapterId', chapterId);
	}

	export function goToSearchHit(hit: SearchHit): void {
		call('goToSearchHit', hit);
	}

	export function goToLocator(locator: ReaderLocator): void {
		call('goToLocator', locator);
	}

	export function bookmarkCurrent(): Promise<void> {
		return Promise.resolve(call<void>('bookmarkCurrent'));
	}

	export function flushLocator(): void {
		call('flushLocator');
	}

	export function handleBack(): boolean {
		return call<boolean>('handleBack') ?? false;
	}

	export function getFocusContext(): {
		articleEl: HTMLElement;
		scrollerEl: HTMLElement;
		contentKey: string;
	} | null {
		return call('getFocusContext') ?? null;
	}

	/** True once the kernel mounted (its first chapter may still be loading). */
	export function isReady(): boolean {
		ready = readerRef !== null;
		return ready;
	}
</script>

{#if EpubReaderComponent}
	<EpubReaderComponent
		bind:this={readerRef}
		{bookId}
		{bookDir}
		{publication}
		isMobile
		embedded
		{initialLocator}
		{onExit}
		{onLocator}
		{onMeta}
	/>
{:else}
	<div class="epub-loading" data-testid="epub-loading" aria-hidden="true">
		<div class="epub-skel"></div>
		<div class="epub-skel"></div>
		<div class="epub-skel short"></div>
	</div>
{/if}

<style>
	:global(.epub-reader.embedded) {
		height: 100%;
	}

	.epub-loading {
		padding: 64px 20px 0;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.epub-skel {
		height: 15px;
		border-radius: 6px;
		background: var(--bg-secondary);
	}

	.epub-skel.short {
		width: 68%;
	}
</style>
