import { defineConfig } from "vite";
import preact from "@preact/preset-vite";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [preact(), tailwindcss()],
  build: {
    // The output is embedded in the Rust binary, so keep it lean and predictable.
    outDir: "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 700,
  },
  server: {
    port: 5173,
    proxy: {
      // `npm run dev` talks to a panel started with `--dev-cors`.
      "/api": { target: "http://127.0.0.1:8080", ws: true, changeOrigin: true },
    },
  },
});
