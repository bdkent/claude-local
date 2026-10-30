import { defineConfig } from "vite";

// Tauri expects a fixed dev port and the built files in /dist.
export default defineConfig({
  root: "ui",
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "../dist", emptyOutDir: true, target: "es2021" },
});
