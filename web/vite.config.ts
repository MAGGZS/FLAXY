import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Em dev, /api vai para o servidor Rust. Em produção o próprio servidor serve o build.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": { target: "http://127.0.0.1:8000", ws: true, changeOrigin: true },
    },
  },
});
