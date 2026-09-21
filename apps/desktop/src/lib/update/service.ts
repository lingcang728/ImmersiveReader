import { writable } from "svelte/store";
import { describeError } from "$lib/errors";

export type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "installing" | "failed" | "upToDate";

export interface UpdateViewState {
	status: UpdateStatus;
	currentVersion: string;
	version: string;
	date: string;
	notes: string;
	sizeBytes: number | null;
	downloadedBytes: number;
	totalBytes: number | null;
	error: string;
}

const AUTO_CHECK_KEY = "immersive-reader-updater-last-auto-check-v1";
const AUTO_CHECK_INTERVAL = 24 * 60 * 60 * 1_000;
const initialState: UpdateViewState = {
	status: "idle",
	currentVersion: "",
	version: "",
	date: "",
	notes: "",
	sizeBytes: null,
	downloadedBytes: 0,
	totalBytes: null,
	error: "",
};

export const updateState = writable<UpdateViewState>(initialState);
let currentState = initialState;
type TauriUpdate = Awaited<ReturnType<typeof import("@tauri-apps/plugin-updater")["check"]>>;
let pendingUpdate: TauriUpdate = null;

function isTauriRuntime(): boolean {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function patch(value: Partial<UpdateViewState>): void {
	currentState = { ...currentState, ...value };
	updateState.set(currentState);
}

function errorMessage(error: unknown): string {
	return describeError(error);
}

function positiveNumber(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function packageSize(raw: Record<string, unknown>): number | null {
	const direct = positiveNumber(raw.size);
	if (direct !== null) return direct;
	const platforms = raw.platforms;
	if (!platforms || typeof platforms !== "object") return null;
	for (const value of Object.values(platforms)) {
		if (value && typeof value === "object") {
			const size = positiveNumber((value as Record<string, unknown>).size);
			if (size !== null) return size;
		}
	}
	return null;
}

async function loadCurrentVersion(): Promise<void> {
	if (currentState.currentVersion || !isTauriRuntime()) return;
	const { getVersion } = await import("@tauri-apps/api/app");
	patch({ currentVersion: await getVersion() });
}

let checkInFlight: Promise<void> | null = null;

export function checkForDesktopUpdate(manual = false): Promise<void> {
	if (checkInFlight) return checkInFlight;
	checkInFlight = doCheckForDesktopUpdate(manual).finally(() => {
		checkInFlight = null;
	});
	return checkInFlight;
}

async function doCheckForDesktopUpdate(manual: boolean): Promise<void> {
	if (!isTauriRuntime()) return;
	patch({ status: "checking", error: "" });
	try {
		await loadCurrentVersion();
		const lastCheck = Number(localStorage.getItem(AUTO_CHECK_KEY) ?? 0);
		if (!manual && Date.now() - lastCheck < AUTO_CHECK_INTERVAL) {
			patch({ status: "upToDate" });
			return;
		}
		const { check } = await import("@tauri-apps/plugin-updater");
		pendingUpdate = await check({ timeout: 15_000 });
		if (!pendingUpdate) {
			localStorage.setItem(AUTO_CHECK_KEY, String(Date.now()));
			patch({ status: "upToDate", version: "", notes: "", sizeBytes: null });
			return;
		}
		localStorage.removeItem(AUTO_CHECK_KEY);
		const sizeBytes = packageSize(pendingUpdate.rawJson);
		patch({
			status: "available",
			currentVersion: pendingUpdate.currentVersion,
			version: pendingUpdate.version,
			date: pendingUpdate.date ?? "",
			notes: pendingUpdate.body ?? "",
			sizeBytes,
			downloadedBytes: 0,
			totalBytes: sizeBytes,
		});
	} catch (error) {
		pendingUpdate = null;
		patch({ status: "failed", error: errorMessage(error) });
	}
}

export async function downloadAndInstallDesktopUpdate(): Promise<void> {
	if (!pendingUpdate || currentState.status !== "available") {
		patch({ status: "failed", error: "没有可安装的更新，请重新检查。" });
		return;
	}
	try {
		patch({ status: "downloading", error: "", downloadedBytes: 0 });
		await pendingUpdate.downloadAndInstall((event) => {
			if (event.event === "Started") {
				patch({ totalBytes: event.data.contentLength ?? currentState.sizeBytes });
			} else if (event.event === "Progress") {
				patch({ downloadedBytes: currentState.downloadedBytes + event.data.chunkLength });
			} else {
				patch({ status: "installing" });
			}
		});
		patch({ status: "installing" });
		const { relaunch } = await import("@tauri-apps/plugin-process");
		await relaunch();
	} catch (error) {
		patch({ status: "failed", error: errorMessage(error) });
	}
}
