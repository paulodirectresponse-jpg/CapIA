import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Porta fixa: tauri.conf.json aponta devUrl para ela.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", target: "es2023" },
});
