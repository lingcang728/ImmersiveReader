export type FileSrcConverter = (path: string) => string;

const imageTagRegex = /<img\b[^>]*>/gi;
const srcAttributeRegex = /\ssrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i;

function decodeHtmlAttribute(value: string): string {
	return value.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (full, entity) => {
		if (entity[0] === '#') {
			const isHex = entity[1]?.toLowerCase() === 'x';
			const codePoint = Number.parseInt(entity.slice(isHex ? 2 : 1), isHex ? 16 : 10);
			if (!Number.isFinite(codePoint)) return full;
			try {
				return String.fromCodePoint(codePoint);
			} catch {
				return full;
			}
		}

		const entities: Record<string, string> = {
			amp: '&',
			apos: "'",
			gt: '>',
			lt: '<',
			nbsp: '\u00a0',
			quot: '"'
		};
		return entities[entity] ?? full;
	});
}

function escapeHtmlAttribute(value: string): string {
	return value
		.replace(/&/g, '&amp;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;');
}

function safeDecodeUriPath(value: string): string {
	try {
		return decodeURIComponent(value);
	} catch {
		return value;
	}
}

function splitPathSuffix(src: string): { path: string; suffix: string } {
	const queryIndex = src.indexOf('?');
	const hashIndex = src.indexOf('#');
	const suffixIndex =
		queryIndex === -1 ? hashIndex : hashIndex === -1 ? queryIndex : Math.min(queryIndex, hashIndex);

	if (suffixIndex === -1) return { path: src, suffix: '' };
	return {
		path: src.slice(0, suffixIndex),
		suffix: src.slice(suffixIndex)
	};
}

function isWindowsAbsolutePath(path: string): boolean {
	return /^[a-zA-Z]:[\\/]/.test(path);
}

function isAbsoluteNativePath(path: string): boolean {
	return isWindowsAbsolutePath(path) || path.startsWith('/') || path.startsWith('\\');
}

function hasExternalScheme(path: string): boolean {
	if (isWindowsAbsolutePath(path)) return false;
	return /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(path);
}

function directoryName(path: string): string {
	const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
	return index === -1 ? '' : path.slice(0, index);
}

function pathSeparator(path: string): string {
	return path.includes('\\') ? '\\' : '/';
}

function joinNativePath(baseDirectory: string, imagePath: string): string {
	if (!baseDirectory) return imagePath;
	const separator = pathSeparator(baseDirectory);
	if (baseDirectory.endsWith('/') || baseDirectory.endsWith('\\')) {
		return `${baseDirectory}${imagePath}`;
	}
	return `${baseDirectory}${separator}${imagePath}`;
}

function normalizeNativePath(path: string): string {
	const separator = pathSeparator(path);
	const normalizedInput = path.replace(/[\\/]+/g, separator);
	const windowsRoot = /^([a-zA-Z]:)([\\/]|$)/.exec(normalizedInput);
	const root = windowsRoot ? `${windowsRoot[1]}${separator}` : normalizedInput.startsWith(separator) ? separator : '';
	const body = root ? normalizedInput.slice(root.length) : normalizedInput;
	const segments: string[] = [];

	for (const segment of body.split(separator)) {
		if (!segment || segment === '.') continue;
		if (segment === '..') {
			if (segments.length > 0 && segments[segments.length - 1] !== '..') {
				segments.pop();
			} else if (!root) {
				segments.push(segment);
			}
			continue;
		}
		segments.push(segment);
	}

	return `${root}${segments.join(separator)}`;
}

function pathFromFileUrl(src: string): string | null {
	try {
		const url = new URL(src);
		if (url.protocol !== 'file:') return null;
		const decoded = safeDecodeUriPath(url.pathname);
		if (/^\/[a-zA-Z]:\//.test(decoded)) {
			return decoded.slice(1).replace(/\//g, '\\');
		}
		return decoded;
	} catch {
		return null;
	}
}

export function resolveMarkdownImageSrc(
	src: string,
	markdownPath: string,
	convertFileSrc: FileSrcConverter
): string {
	const decodedSrc = decodeHtmlAttribute(src).trim();
	if (!decodedSrc || decodedSrc.startsWith('#') || decodedSrc.startsWith('//')) return src;
	if (/^(https?|data|blob|asset):/i.test(decodedSrc)) return src;
	if (/^https?:\/\/asset\.localhost\//i.test(decodedSrc)) return src;

	const fileUrlPath = /^file:/i.test(decodedSrc) ? pathFromFileUrl(decodedSrc) : null;
	if (fileUrlPath) return convertFileSrc(fileUrlPath);
	if (hasExternalScheme(decodedSrc)) return src;

	const { path, suffix } = splitPathSuffix(decodedSrc);
	if (!path) return src;

	const decodedPath = safeDecodeUriPath(path);
	const nativePath = isAbsoluteNativePath(decodedPath)
		? decodedPath
		: joinNativePath(directoryName(markdownPath), decodedPath);

	return `${convertFileSrc(normalizeNativePath(nativePath))}${suffix}`;
}

export function resolveMarkdownImageSources(
	html: string,
	markdownPath: string,
	convertFileSrc: FileSrcConverter
): string {
	if (!markdownPath || !html.includes('<img')) return html;

	return html.replace(imageTagRegex, (tag) => {
		const match = srcAttributeRegex.exec(tag);
		if (!match) return tag;

		const rawValue = match[1] ?? match[2] ?? match[3] ?? '';
		const resolved = resolveMarkdownImageSrc(rawValue, markdownPath, convertFileSrc);
		if (resolved === rawValue) return tag;

		const quote = match[1] !== undefined ? '"' : match[2] !== undefined ? "'" : '"';
		const replacement = ` src=${quote}${escapeHtmlAttribute(resolved)}${quote}`;

		return `${tag.slice(0, match.index)}${replacement}${tag.slice(match.index + match[0].length)}`;
	});
}
