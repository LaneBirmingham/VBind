import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

// `npm run dev` proxies the API to a running VBind (default port 47821).
const target = `http://127.0.0.1:${process.env.VBIND_PORT ?? "47821"}`;

export default defineConfig({
  plugins: [preact()],
  build: { outDir: "dist", emptyOutDir: true, target: "es2022", chunkSizeWarningLimit: 900 },
  server: { proxy: { "/api": { target, changeOrigin: true } } },
});
