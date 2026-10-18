<script lang="ts">
	import { detectDevice } from "$lib/platform/device";

	// 启动优化（task-1）：路由入口只做平台分发。桌面端与移动端各自的应用外壳
	// 都在这里动态加载，首屏不再解析另一端的全部 UI 代码；两端实现互相独立
	// （$lib/desktop/DesktopApp.svelte / $lib/mobile/MobileApp.svelte）。
	const isMobile = detectDevice().isMobile;

	// 只触发当前平台需要的那一个动态 import。等待/失败状态直接由这个 Promise
	// 驱动：加载期间显示与 app.html 相同主题底色的占位，失败时显示可读错误。
	const appModule = isMobile
		? import("$lib/mobile/MobileApp.svelte")
		: import("$lib/desktop/DesktopApp.svelte");

	function describeLoadError(error: unknown): string {
		if (error instanceof Error && error.message) return error.message;
		if (typeof error === "string" && error) return error;
		try {
			return JSON.stringify(error);
		} catch {
			return String(error);
		}
	}

	function retryLoad() {
		if (typeof window !== "undefined") window.location.reload();
	}
</script>

{#await appModule}
	<!-- 与 app.html 相同的主题底色（--bg），避免模块加载期间白屏。 -->
	<div class="boot-placeholder" role="status" aria-live="polite">
		<span>正在加载…</span>
	</div>
{:then app}
	<svelte:component this={app.default} />
{:catch error}
	<div class="boot-error" role="alert">
		<p>应用加载失败，请重新打开沉浸阅读。</p>
		<pre>{describeLoadError(error)}</pre>
		<button type="button" on:click={retryLoad}>重新加载</button>
	</div>
{/await}

<style>
	.boot-placeholder,
	.boot-error {
		display: flex;
		min-height: 100vh;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		padding: 24px;
		box-sizing: border-box;
		background: var(--bg, #faf8f5);
		color: var(--text-secondary, #5a5a5a);
		font-family: ui-sans-serif, system-ui, "Segoe UI", "Microsoft YaHei", sans-serif;
		font-size: 13px;
		line-height: 1.6;
		letter-spacing: 0.04em;
		text-align: center;
	}

	.boot-error {
		color: var(--danger, #a03528);
	}

	.boot-error p {
		margin: 0;
	}

	.boot-error pre {
		max-width: min(560px, 90vw);
		max-height: 30vh;
		overflow: auto;
		margin: 0;
		color: var(--text-secondary, #5a5a5a);
		font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
		font-size: 11.5px;
		white-space: pre-wrap;
		word-break: break-word;
	}

	.boot-error button {
		padding: 4px 14px;
		border: 1px solid var(--hr, #d0cbc1);
		border-radius: 4px;
		background: var(--bg-secondary, #f0ebe3);
		color: var(--text, #222222);
		font: inherit;
		cursor: pointer;
	}
</style>
