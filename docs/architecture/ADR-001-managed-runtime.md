# ADR-001: Managed runtime is a verified release input

状态：Accepted

## Context

Immersive Reader depends on Node/Chromium for the Zhihu sidecar and on Python, FFmpeg, and a local Whisper model for the Podcast sidecar. These assets are machine-specific, large, and intentionally excluded from Git. A release that only builds the Tauri executable can therefore produce an installer which starts but cannot start either sidecar.

## Decision

`scripts/prepare-runtime.ps1` is the single provisioning path for local builds and release refreshes. It copies the source applications into the managed `runtime/` tree and writes `runtime/manifest.json` schema v2. The manifest covers all application and contracts files, all model files, and the critical executable/library/resource extensions under the managed Node, Chromium, Python, and FFmpeg trees.

`scripts/verify-runtime.ps1` is the single integrity gate. It rejects unsafe or duplicate paths and verifies every manifest entry's existence, byte length, and SHA-256. The local ship script runs it before packaging and again against the installed runtime. Release CI must download a versioned runtime bundle, verify it before use, refresh application code, and verify it again before publishing the installer.

The runtime bundle is not committed to Git. CI first uses `IMMERSIVE_READER_RUNTIME_BUNDLE_URL` when it is configured; otherwise it downloads ordered `runtime-bundle.zip.*` volumes from the draft Release for the current Tag and reconstructs the archive byte-for-byte. A missing URL and missing Release volumes fail before any installer is published. This prevents a clean checkout from silently producing an incomplete release while allowing the immutable, multi-gigabyte runtime to remain attached to its versioned Release.

## Consequences

- Clean release machines do not need a repository-local model or browser profile.
- A tampered critical runtime file blocks local shipping and CI packaging.
- The external bundle store or versioned Release volumes must retain the exact bundle used by a release and provide HTTPS transport integrity in addition to the manifest check.
- Runtime preparation can be slower because the critical manifest hashes thousands of application/model/runtime files, but it is bounded to the release input and provides an auditable acceptance gate.
