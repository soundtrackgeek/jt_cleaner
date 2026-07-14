import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
  plugins: [react()],
  server: {
    host: "0.0.0.0",
    port: 1420,
    strictPort: true,
    allowedHosts: ["terminal.local"],
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});

