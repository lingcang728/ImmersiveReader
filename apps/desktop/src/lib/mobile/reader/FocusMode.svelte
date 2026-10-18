<!--
	Mobile focus mode — sentence spotlight + progressive dim/blur + HUD.

	Locked algorithms are reused, not re-implemented:
	  - `splitSentences` ($lib/focus/segment) splits the chapter into sentences
	  - `getFocusScrollTarget` ($lib/focus/scroll) decides where the reading
	    line should land inside a scroll container

	Presentation is mobile-only: a fixed, pointer-events:none dim layer with a
	cut-out around the active sentence (progressive blur bands at the cut
	edges), and a bottom HUD. The overlay never mutates the article DOM, so a
	20k-word chapter costs one Range measurement per sentence step.

	Public instance API (used by ReaderScreen / volume keys):
	  next() / prev() / exit() / isActive()
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import {
		buildFocusModel,
		locateUnitAtLine,
		locateUnitAtPoint,
		rangeForUnit,
		scrollTargetForUnit,
		unitText,
		type FocusModel
	} from './sentence';

	interface Props {
		active: boolean;
		articleEl: HTMLElement | null;
		scrollerEl: HTMLElement | null;
		/** Changes whenever the mounted chapter/document changes. */
		contentKey: string;
		/** Bumped by the parent on font/line-height/width reflow. */
		reflowKey?: number;
		onExit: () => void;
		/** Asked when the reader steps past the first/last sentence. */
		onBoundary: (direction: -1 | 1) => void;
	}

	let {
		active,
		articleEl = null,
		scrollerEl = null,
		contentKey,
		reflowKey = 0,
		onExit,
		onBoundary
	}: Props = $props();

	let model = $state<FocusModel | null>(null);
	let activeIndex = $state(0);
	let box = $state<{ top: number; bottom: number; left: number; right: number } | null>(null);
	let ready = $state(false);
	let scrolling = $state(false);
	let hudText = $state('');
	let viewportHeight = $state(0);

	const NEAR_BAND_PX = 84;

	let bottomNearHeight = $derived(
		box ? Math.min(Math.max(0, viewportHeight - box.bottom), NEAR_BAND_PX) : 0
	);
	let bottomFarTop = $derived(box ? box.bottom + bottomNearHeight : 0);
	let bottomFarHeight = $derived(Math.max(0, viewportHeight - bottomFarTop));

	let generation = 0;
	let builtKey = '';
	let programmaticTarget: number | null = null;
	let programmaticTimer: ReturnType<typeof setTimeout> | null = null;
	let scrollFrame: number | null = null;
	let scrollIdleTimer: ReturnType<typeof setTimeout> | null = null;
	let layoutTimers: ReturnType<typeof setTimeout>[] = [];
	let suppressTapUntil = 0;

	function clearProgrammatic() {
		programmaticTarget = null;
		if (programmaticTimer) {
			clearTimeout(programmaticTimer);
			programmaticTimer = null;
		}
	}

	function measure() {
		if (!model || !scrollerEl) {
			box = null;
			return;
		}
		const range = rangeForUnit(model, activeIndex);
		if (!range) {
			box = null;
			return;
		}
		const rect = range.getBoundingClientRect();
		if (rect.height <= 0 && rect.width <= 0) {
			box = null;
			return;
		}
		box = { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
		viewportHeight = window.innerHeight;
		hudText = unitText(model, activeIndex).slice(0, 60);
	}

	function realign(behavior: ScrollBehavior) {
		if (!model || !scrollerEl) return;
		const target = scrollTargetForUnit(model, activeIndex, scrollerEl);
		if (target === null) return;
		if (Math.abs(target - scrollerEl.scrollTop) < 2) {
			measure();
			return;
		}
		clearProgrammatic();
		programmaticTarget = target;
		programmaticTimer = setTimeout(clearProgrammatic, 900);
		scrollerEl.scrollTo({ top: target, behavior });
	}

	function setIndex(index: number, behavior: ScrollBehavior = 'smooth') {
		if (!model) return;
		if (index < 0) {
			onBoundary(-1);
			return;
		}
		if (index >= model.total) {
			onBoundary(1);
			return;
		}
		activeIndex = index;
		measure();
		realign(behavior);
	}

	function relocateFromLine() {
		if (!active || !model || !articleEl || !scrollerEl) return;
		const located = locateUnitAtLine(model, articleEl, scrollerEl);
		if (located && located.globalIndex !== activeIndex) activeIndex = located.globalIndex;
		measure();
	}

	/** Build (or rebuild after a chapter swap) and align to the reading line. */
	function enterOrRebuild(forceBuild: boolean) {
		if (!active || !articleEl || !scrollerEl) return;
		if (forceBuild || builtKey !== contentKey || !model) {
			builtKey = contentKey;
			const next = buildFocusModel(articleEl);
			model = next;
			if (next.total === 0) {
				ready = false;
				box = null;
				clearProgrammatic();
				return;
			}
			ready = true;
			const located = locateUnitAtLine(next, articleEl, scrollerEl);
			activeIndex = located?.globalIndex ?? 0;
			measure();
			realign('auto');
			return;
		}
		relocateFromLine();
		realign('auto');
	}

	function scheduleLayoutSync(forceBuild: boolean) {
		for (const timer of layoutTimers) clearTimeout(timer);
		layoutTimers = [];
		const gen = generation;
		// The parent restores the chapter's scroll position a frame or two
		// after the new DOM commits; re-run the line sync until it settles.
		// Only the first pass rebuilds the sentence model — later passes just
		// re-locate on the reading line (rebuilding a long chapter is costly).
		const delays = [0, 90, 260, 520];
		for (const [index, delay] of delays.entries()) {
			layoutTimers.push(
				setTimeout(() => {
					if (gen !== generation) return;
					untrack(() => enterOrRebuild(forceBuild && index === 0));
				}, delay)
			);
		}
	}

	function handleScroll() {
		if (!active || scrollFrame !== null) return;
		scrolling = true;
		if (scrollIdleTimer) clearTimeout(scrollIdleTimer);
		scrollIdleTimer = setTimeout(() => {
			scrolling = false;
			scrollIdleTimer = null;
		}, 200);
		scrollFrame = requestAnimationFrame(() => {
			scrollFrame = null;
			if (!active || !model || !articleEl || !scrollerEl) return;
			if (programmaticTarget !== null) {
				if (Math.abs(scrollerEl.scrollTop - programmaticTarget) < 3) {
					clearProgrammatic();
				} else {
					measure();
					return;
				}
			}
			const located = locateUnitAtLine(model, articleEl, scrollerEl);
			if (located && located.globalIndex !== activeIndex) activeIndex = located.globalIndex;
			measure();
		});
	}

	function handleTap(event: MouseEvent) {
		if (!active || !model || !articleEl || !scrollerEl) return;
		if (performance.now() < suppressTapUntil) return;
		const target = event.target instanceof Element ? event.target : null;
		if (target?.closest('a[href]')) {
			event.preventDefault();
			event.stopPropagation();
		}
		const located = locateUnitAtPoint(model, articleEl, event.clientX, event.clientY);
		if (!located || located.globalIndex === activeIndex) {
			measure();
			return;
		}
		suppressTapUntil = performance.now() + 120;
		setIndex(located.globalIndex, 'smooth');
	}

	function handleResize() {
		if (!active) return;
		measure();
	}

	function exitFocus() {
		// Keep the reader where the sentence currently sits: realign before the
		// overlay disappears so leaving focus never moves the reading position.
		realign('auto');
		onExit();
	}

	export function next() {
		setIndex(activeIndex + 1);
	}

	export function prev() {
		setIndex(activeIndex - 1);
	}

	export function exit() {
		exitFocus();
	}

	export function isActive(): boolean {
		return active && ready;
	}

	// Attach/detach when focus is entered, the document changes, or the refs
	// swap. Reads are captured up-front; the body runs untracked so state
	// writes inside the sync cannot re-trigger this effect.
	$effect(() => {
		const isActiveNow = active;
		const article = articleEl;
		const scroller = scrollerEl;
		// Read the key so a chapter swap re-runs this effect; the value itself
		// is only consumed through `builtKey` inside the untracked body.
		void contentKey;
		const gen = ++generation;

		untrack(() => {
			builtKey = '';
			model = null;
			box = null;
			ready = false;
			clearProgrammatic();
			if (!isActiveNow || !article || !scroller) return;
			scroller.addEventListener('scroll', handleScroll, { passive: true });
			scroller.addEventListener('click', handleTap, true);
			window.addEventListener('resize', handleResize);
			window.visualViewport?.addEventListener('resize', handleResize);
			scheduleLayoutSync(true);
		});

		return () => {
			if (gen === generation) generation += 1;
			untrack(() => {
				if (article && scroller) {
					scroller.removeEventListener('scroll', handleScroll);
					scroller.removeEventListener('click', handleTap, true);
				}
				window.removeEventListener('resize', handleResize);
				window.visualViewport?.removeEventListener('resize', handleResize);
				for (const timer of layoutTimers) clearTimeout(timer);
				layoutTimers = [];
				if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
				scrollFrame = null;
				clearProgrammatic();
				if (scrollIdleTimer) clearTimeout(scrollIdleTimer);
				scrollIdleTimer = null;
				scrolling = false;
			});
		};
	});

	// Font/line-height/width reflow: re-measure and pull the active sentence
	// back to the reading line (viewport-anchor behavior).
	$effect(() => {
		reflowKey;
		untrack(() => {
			if (!active || !ready) return;
			requestAnimationFrame(() => {
				if (!active) return;
				measure();
				realign('auto');
			});
		});
	});
</script>

{#if active && ready}
	<div class="focus-layer" class:scrolling data-testid="focus-mode">
		{#if box}
			<div
				class="focus-dim focus-dim-far top"
				style:height="{Math.max(0, box.top - NEAR_BAND_PX)}px"
			></div>
			<div
				class="focus-dim focus-dim-near top"
				style:top="{Math.max(0, box.top - NEAR_BAND_PX)}px"
				style:height="{Math.min(box.top, NEAR_BAND_PX)}px"
			></div>
			<div
				class="focus-dim focus-dim-near bottom"
				style:top="{box.bottom}px"
				style:height="{bottomNearHeight}px"
			></div>
			<div
				class="focus-dim focus-dim-far bottom"
				style:top="{bottomFarTop}px"
				style:height="{bottomFarHeight}px"
			></div>
			<div class="focus-spotlight" style:top="{box.top}px" style:height="{box.bottom - box.top}px"></div>
		{/if}

		<div class="focus-hud" data-testid="focus-hud" role="toolbar" aria-label="专注模式">
			<button type="button" class="focus-hud-btn" onclick={prev} aria-label="上一句">上一句</button>
			<span class="focus-hud-count" title={hudText}>
				第 {activeIndex + 1} / {model?.total ?? 0} 句
			</span>
			<button type="button" class="focus-hud-btn" onclick={next} aria-label="下一句">下一句</button>
			<button
				type="button"
				class="focus-hud-exit"
				onclick={exitFocus}
				aria-label="退出专注模式"
			>退出</button>
		</div>
	</div>
{/if}

<style>
	.focus-layer {
		position: fixed;
		inset: 0;
		z-index: 44;
		pointer-events: none;
		/* Overlay only: no layout work happens here beyond transform/opacity. */
	}

	.focus-dim {
		position: fixed;
		left: 0;
		right: 0;
		pointer-events: none;
		background: var(--bg);
		will-change: height, top;
	}

	.focus-dim-far {
		opacity: 0.9;
	}

	.focus-dim-near {
		opacity: 0.72;
		backdrop-filter: blur(2px);
		-webkit-backdrop-filter: blur(2px);
	}

	.focus-dim-far.top {
		top: 0;
	}

	.focus-dim-far.bottom {
		opacity: 0.9;
	}

	.focus-dim-near.top {
		background: linear-gradient(to bottom, var(--bg) 0%, transparent 100%);
		mask-image: linear-gradient(to bottom, black 30%, transparent 100%);
		-webkit-mask-image: linear-gradient(to bottom, black 30%, transparent 100%);
	}

	.focus-dim-near.bottom {
		background: linear-gradient(to top, var(--bg) 0%, transparent 100%);
		mask-image: linear-gradient(to top, black 30%, transparent 100%);
		-webkit-mask-image: linear-gradient(to top, black 30%, transparent 100%);
	}

	/* While finger-scrolling, backdrop blur is dropped: it is the most
	   expensive part of the overlay and is invisible during motion anyway. */
	.focus-layer.scrolling .focus-dim-near {
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		opacity: 0.55;
	}

	.focus-spotlight {
		position: fixed;
		left: 8px;
		right: 8px;
		pointer-events: none;
		border-radius: 10px;
		box-shadow:
			0 0 0 1px color-mix(in srgb, var(--spotlight-color) 90%, transparent),
			0 0 28px 10px var(--spotlight-color);
		transition: top 140ms ease, height 140ms ease;
	}

	.focus-hud {
		position: fixed;
		left: 50%;
		transform: translateX(-50%);
		bottom: calc(max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)) + 24px);
		display: flex;
		align-items: center;
		gap: 2px;
		padding: 4px;
		border-radius: 999px;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		box-shadow: 0 10px 32px rgba(0, 0, 0, 0.28);
		pointer-events: auto;
		z-index: 46;
		max-width: calc(100vw - 24px);
	}

	.focus-hud-btn,
	.focus-hud-exit {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: 44px;
		height: 44px;
		padding: 0 10px;
		border: 0;
		border-radius: 999px;
		background: transparent;
		color: var(--text);
		font-size: 14px;
		cursor: pointer;
		-webkit-tap-highlight-color: transparent;
		white-space: nowrap;
	}

	.focus-hud-btn:active,
	.focus-hud-exit:active {
		background: var(--bg);
	}

	.focus-hud-count {
		font-size: 12.5px;
		color: var(--text-secondary);
		font-variant-numeric: tabular-nums;
		padding: 0 6px;
		white-space: nowrap;
		max-width: 40vw;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.focus-hud-exit {
		color: var(--link);
	}

	@media (prefers-reduced-motion: reduce) {
		.focus-spotlight {
			transition: none;
		}
	}
</style>
