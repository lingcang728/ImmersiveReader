import { get, writable } from "svelte/store";
import { invokeCommand } from "../ipc";
import { detectDevice } from "./device";

/**
 * Backend-reported platform capabilities (`get_platform_capabilities`).
 *
 * Desktop builds ship the full surface (folder imports, task acquisition,
 * reader sessions, Credential Manager secrets…). Android/Tauri-mobile builds
 * lack several of those — SAF directory walking, the control-DB task engines
 * and native window management either do not exist or cannot work through the
 * sandbox, so the UI gates the corresponding affordances instead of offering
 * controls that fail at click time.
 *
 * `normalizePlatformCapabilities` is intentionally defensive about wire
 * naming (snake_case and camelCase) and about unknown payloads: any field the
 * backend does not report falls back to the device-derived default rather
 * than silently flipping a feature off.
 */
export interface PlatformCapabilities {
	/** Backend platform tag, e.g. "windows" / "android". */
	platform: string;
	/** True on phone/tablet shells where mobile UI is enabled. */
	isMobile: boolean;
	/** Formats the backend importer accepts, e.g. ["md","markdown","txt","epub","zip"]. */
	supportedFormats: string[];
	/** Whether filesystem directory picking/imports are possible. */
	directoryImport: boolean;
	/** Whether content:// URI (Android SAF) imports are possible. */
	uriImport: boolean;
	/** Whether "reveal in folder" style shell actions are possible. */
	revealDirectory: boolean;
	/** Whether 连读 reader sessions (`start_reader_session`) work. */
	readerSessions: boolean;
	/** Whether Zhihu/podcast task acquisition machinery is supported. */
	taskAcquisition: boolean;
	/** Whether a secrets store (Credential Manager / Keystore) exists. */
	secretStore: boolean;
	/** Whether the platform can intercept hardware volume keys. */
	volumeKeyBridge: boolean;
	/** Whether external open-with/pending-open intents are delivered. */
	openWith: boolean;
	/** Whether reading-bundle export/import is implemented. */
	exportBundle: boolean;
	/** Whether in-place markdown editing is supported. */
	markdownEditing: boolean;
}

/**
 * Device-derived defaults used until the backend answers and forever when the
 * command is unavailable on an older backend. Defaults are optimistic for
 * desktop features (they exist today) and conservative for mobile-only ones.
 */
export function fallbackPlatformCapabilities(): PlatformCapabilities {
	const device = detectDevice();
	const isMobile = device.isMobile;
	return {
		platform: device.isAndroid ? "android" : device.isiOS ? "ios" : "desktop",
		isMobile,
		supportedFormats: ["md", "markdown", "txt", "epub", "zip"],
		directoryImport: !isMobile,
		uriImport: device.isAndroid,
		revealDirectory: !isMobile,
		readerSessions: true,
		taskAcquisition: !isMobile,
		secretStore: true,
		volumeKeyBridge: device.isAndroid,
		// take_pending_open_uris is registered on every target (desktop
		// "open with" queues non-Markdown payloads there too) — draining it
		// is always safe, worst case it answers [].
		openWith: true,
		exportBundle: true,
		markdownEditing: true,
	};
}

type RawCaps = Record<string, unknown>;

function pick(raw: RawCaps, ...keys: string[]): unknown {
	for (const key of keys) {
		if (key in raw) return raw[key];
	}
	return undefined;
}

function coerceBool(raw: RawCaps, fallback: boolean, ...keys: string[]): boolean {
	const value = pick(raw, ...keys);
	if (typeof value === "boolean") return value;
	if (typeof value === "number") return value !== 0;
	if (typeof value === "string") {
		const lowered = value.toLowerCase();
		if (lowered === "true") return true;
		if (lowered === "false") return false;
	}
	return fallback;
}

function coerceStringList(raw: RawCaps, fallback: string[], ...keys: string[]): string[] {
	const value = pick(raw, ...keys);
	if (Array.isArray(value)) {
		const list = value.filter((item): item is string => typeof item === "string");
		return list.length > 0 ? list : fallback;
	}
	if (typeof value === "string" && value.trim()) {
		return value
			.split(/[,\s]+/)
			.map((item) => item.trim())
			.filter(Boolean);
	}
	return fallback;
}

/**
 * Merge an untrusted backend payload over the device-derived fallback. Unknown
 * or missing keys keep the fallback value so a partial payload never hides a
 * shipped feature by accident.
 */
export function normalizePlatformCapabilities(
	raw: unknown,
	fallback: PlatformCapabilities = fallbackPlatformCapabilities(),
): PlatformCapabilities {
	if (typeof raw !== "object" || raw === null) return fallback;
	const bag = raw as RawCaps;
	const platformValue = pick(bag, "platform", "os");
	return {
		platform:
			typeof platformValue === "string" && platformValue
				? platformValue
				: fallback.platform,
		isMobile: coerceBool(bag, fallback.isMobile, "is_mobile", "isMobile", "mobile"),
		supportedFormats: coerceStringList(
			bag,
			fallback.supportedFormats,
			"supported_formats",
			"supportedFormats",
			"formats",
		),
		directoryImport: coerceBool(
			bag,
			fallback.directoryImport,
			"directory_import",
			"directoryImport",
			"supports_directory_import",
		),
		uriImport: coerceBool(
			bag,
			fallback.uriImport,
			"uri_import",
			"uriImport",
			"supports_uri_import",
			"content_uri_import",
			"contentUriImport",
		),
		revealDirectory: coerceBool(
			bag,
			fallback.revealDirectory,
			"reveal_directory",
			"revealDirectory",
			"can_reveal",
		),
		readerSessions: coerceBool(
			bag,
			fallback.readerSessions,
			"reader_sessions",
			"readerSessions",
			// Backend emits the singular `readerSession` (tiny_http 连读).
			"reader_session",
			"readerSession",
			"continuous_reading",
		),
		taskAcquisition: coerceBool(
			bag,
			fallback.taskAcquisition,
			"task_acquisition",
			"taskAcquisition",
			"supports_tasks",
		),
		secretStore: coerceBool(
			bag,
			fallback.secretStore,
			"secret_store",
			"secretStore",
			"keystore",
		),
		volumeKeyBridge: coerceBool(
			bag,
			fallback.volumeKeyBridge,
			"volume_key_bridge",
			"volumeKeyBridge",
			// Backend emits `volumeKeyCapture`.
			"volume_key_capture",
			"volumeKeyCapture",
			"volume_keys",
		),
		openWith: coerceBool(
			bag,
			fallback.openWith,
			"open_with",
			"openWith",
			"pending_open_uris",
		),
		exportBundle: coerceBool(
			bag,
			fallback.exportBundle,
			"export_bundle",
			"exportBundle",
			"reading_bundle",
			"readingBundle",
		),
		markdownEditing: coerceBool(
			bag,
			fallback.markdownEditing,
			"markdown_editing",
			"markdownEditing",
		),
	};
}

/**
 * Live capabilities store. Starts at the device-derived fallback so UI never
 * flashes gated controls while the backend answer is in flight.
 */
export const platformCapabilities = writable<PlatformCapabilities>(
	fallbackPlatformCapabilities(),
);

export const platformCapabilitiesError = writable<string | null>(null);

/**
 * Query `get_platform_capabilities` and refresh the store. Safe to call
 * repeatedly; failures keep the previous value and record the error.
 */
export async function refreshPlatformCapabilities(): Promise<PlatformCapabilities> {
	const fallback = get(platformCapabilities);
	try {
		const raw = await invokeCommand<unknown>("get_platform_capabilities");
		const normalized = normalizePlatformCapabilities(raw, fallback);
		platformCapabilities.set(normalized);
		platformCapabilitiesError.set(null);
		return normalized;
	} catch (error) {
		platformCapabilitiesError.set(
			error instanceof Error ? error.message : String(error),
		);
		return fallback;
	}
}
