const textEncoder = new TextEncoder();

/**
 * Deterministic non-crypto digest for contexts where `crypto.subtle` is
 * missing (non-secure context): four independent FNV-1a-32 passes over the
 * input → 16 bytes. The result is a pure function of the input, so a retried
 * click still reproduces the same request id and the backend's idempotent
 * claim keeps deduplicating — the old `crypto.randomUUID()` fallback minted
 * a fresh id per call, silently disabling exactly that dedupe.
 */
function fallbackDigest(input: string): Uint8Array {
	const bytes = textEncoder.encode(input);
	const out = new Uint8Array(16);
	const seeds = [0x811c9dc5, 0x050c5d1f, 0x27d4eb2d, 0x9e3779b9];
	for (let s = 0; s < seeds.length; s++) {
		let hash = seeds[s] >>> 0;
		for (let i = 0; i < bytes.length; i++) {
			hash ^= bytes[i];
			hash = Math.imul(hash, 0x01000193) >>> 0;
		}
		out[s * 4] = hash & 0xff;
		out[s * 4 + 1] = (hash >>> 8) & 0xff;
		out[s * 4 + 2] = (hash >>> 16) & 0xff;
		out[s * 4 + 3] = (hash >>> 24) & 0xff;
	}
	return out;
}

async function digest16(input: string): Promise<Uint8Array> {
	try {
		const digest = await crypto.subtle.digest("SHA-256", textEncoder.encode(input));
		return new Uint8Array(digest).subarray(0, 16);
	} catch {
		return fallbackDigest(input);
	}
}

function toUuid(bytes: Uint8Array): string {
	bytes[6] = (bytes[6] & 0x0f) | 0x40; // UUID version 4 layout
	bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant bits
	const hex = Array.from(bytes.subarray(0, 16), (b) => b.toString(16).padStart(2, "0")).join("");
	return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/**
 * Derive a request id from the operation inputs instead of minting a fresh
 * UUID per click: retrying the same action reproduces the same id, so the
 * backend's idempotent command claim dedupes retries rather than spawning
 * duplicate tasks. SHA-256 of the parts folded into UUID layout; without
 * WebCrypto a deterministic FNV-1a digest keeps the retry-stability.
 */
export async function stableRequestId(
	...parts: readonly (string | number | null | undefined)[]
): Promise<string> {
	// Length-prefix each part so embedded separators/newlines can never merge
	// two different part lists into the same digest input.
	const input = parts.map((part) => {
		const value = String(part ?? "");
		return `${value.length}:${value}`;
	}).join(",");
	return toUuid(await digest16(input));
}
