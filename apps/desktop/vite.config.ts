import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri dev server port must match tauri.conf.json devUrl and devCsp
// (1430: 1420 belongs to another project on the dev machine, and whatever
// serves devUrl is trusted as the app in `tauri dev`). Checked by the
// desktop crate's config tests.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
  },
  build: {
    outDir: "dist",
  },
});
