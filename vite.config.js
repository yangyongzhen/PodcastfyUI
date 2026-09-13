import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [sveltekit()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
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
    // 4. 预转换组件：style 虚拟模块的加载依赖「父模块 transform 时缓存的 CSS」
    //    （vite-plugin-svelte 的 load-compiled-css 从 meta.svelte.css 取）。
    //    首个请求若抢在父模块之前到达，加载钩子取不到 CSS 就返回 null，接着
    //    vite:css 拿 .svelte 原文去跑 postcss，报 "Unknown word"（每次 dev 启动
    //    必现、刷新即好）。预热父模块即可消除这个启动竞态。
    warmup: {
      clientFiles: ["./src/routes/+page.svelte", "./src/routes/**/*.svelte"],
      ssrFiles: ["./src/routes/+page.svelte", "./src/routes/**/*.svelte"],
    },
  },
}));
