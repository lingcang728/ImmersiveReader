// Gradle rustBuild* tasks invoke `node tauri android android-studio-script` with
// src-tauri as the working directory. Node resolves `tauri` as a file path, so
// this shim forwards to the real @tauri-apps/cli JS entry.
import '../node_modules/@tauri-apps/cli/tauri.js';
