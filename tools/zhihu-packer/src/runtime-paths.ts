import * as path from "node:path";

export type RuntimePathInput = {
  readonly cwd: string;
  readonly environment: Readonly<Record<string, string | undefined>>;
  readonly explicit?: string;
};

function resolveConfigured(cwd: string, configured: string | undefined, fallback: string): string {
  return path.resolve(cwd, configured?.trim() || fallback);
}

export function resolveArchiveOutputDir(input: RuntimePathInput): string {
  if (input.explicit?.trim()) {
    return path.resolve(input.cwd, input.explicit);
  }
  if (input.environment.IMMERSIVE_ZHIHU_OUTPUT?.trim()) {
    return path.resolve(input.cwd, input.environment.IMMERSIVE_ZHIHU_OUTPUT);
  }
  if (input.environment.IMMERSIVE_LIBRARY_ROOT?.trim()) {
    return path.resolve(input.cwd, input.environment.IMMERSIVE_LIBRARY_ROOT, "知乎");
  }
  return path.resolve(input.cwd, "output");
}

export function resolveDatabasePath(input: RuntimePathInput): string {
  return resolveConfigured(input.cwd, input.environment.IMMERSIVE_ZHIHU_DB, "zhihu-packer.db");
}

/**
 * 工具本地运行态目录名：由进程按需写入、内容随机器而异，
 * 绝不进入 runtime 拷贝/发布 bundle，也不应提交进版本库。
 * scripts/prepare-runtime.ps1 的 Copy-Tree -ExcludeDirectories 与
 * 各级 .gitignore 必须与这份名单保持一致（P2-30⑨：.browser-cache 曾被漏排除，
 * Chromium 磁盘缓存可被打进发布 bundle）。
 */
export const TOOL_LOCAL_STATE_DIRS: readonly string[] = [
  ".browser-profile",
  ".obscura-profile",
  ".browser-cache",
];

export function resolveProfileDir(input: RuntimePathInput): string {
  return resolveConfigured(input.cwd, input.environment.IMMERSIVE_ZHIHU_PROFILE, ".browser-profile");
}

export function resolveBrowserCacheDir(input: RuntimePathInput): string {
  return resolveConfigured(input.cwd, input.environment.IMMERSIVE_ZHIHU_BROWSER_CACHE, ".browser-cache");
}

export function resolveBrowserExecutable(
  environment: Readonly<Record<string, string | undefined>>,
): string | undefined {
  const configured = environment.IMMERSIVE_CHROMIUM_EXECUTABLE?.trim();
  return configured || undefined;
}
