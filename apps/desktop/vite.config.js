import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
const decodeNamedCharacterReference = decodeURIComponent(
  new URL("./node_modules/decode-named-character-reference/index.js", import.meta.url).pathname,
).replace(/^\/([A-Za-z]:)/, "$1");

// https://vite.dev/config/
export default defineConfig(async () => {
  /** @type {import("vite").UserConfig} */
  const config = {
    plugins: await sveltekit(),
    resolve: {
      alias: {
        // The package's browser export touches `document` at module load time.
        // Markdown rendering also runs in a Web Worker, where `document` does not exist.
        "decode-named-character-reference": decodeNamedCharacterReference,
      },
    },
    worker: {
      format: "es",
    },
    build: {
      rollupOptions: {
        output: {
          /**
           * @param {string} id
           * @returns {string | undefined}
           */
          manualChunks(id) {
            if (id.includes("node_modules/svelte/")) return "svelte";
          },
        },
      },
    },

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
  return config;
});
