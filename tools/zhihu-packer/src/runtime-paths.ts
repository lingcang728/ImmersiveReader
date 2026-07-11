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

export function resolveProfileDir(input: RuntimePathInput): string {
  return resolveConfigured(input.cwd, input.environment.IMMERSIVE_ZHIHU_PROFILE, ".browser-profile");
}

export function resolveBrowserExecutable(
  environment: Readonly<Record<string, string | undefined>>,
): string | undefined {
  const configured = environment.IMMERSIVE_CHROMIUM_EXECUTABLE?.trim();
  return configured || undefined;
}
