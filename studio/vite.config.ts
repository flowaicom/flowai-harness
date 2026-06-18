import { reactRouter } from "@react-router/dev/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import tsconfigPaths from "vite-tsconfig-paths";

const backendTarget = process.env.FLOWAI_STUDIO_BACKEND_URL ?? "http://localhost:4111";

export default defineConfig({
  plugins: [tailwindcss(), reactRouter(), tsconfigPaths()],
  server: {
    port: 3000,
    proxy: {
      "/api": {
        target: backendTarget,
        changeOrigin: true,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        configure: (proxy: any) => {
          proxy.on("error", (err: any, _req: any, res: any) => {
            console.warn("[vite proxy] Backend not ready:", err.message);
            if ("writeHead" in res && typeof res.writeHead === "function") {
              res.writeHead(503, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ error: "Backend not ready" }));
            }
          });
        },
      },
      "/__flowai_config.js": {
        target: backendTarget,
        changeOrigin: true,
      },
    },
  },
  build: {
    target: "esnext",
    minify: "esbuild",
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          if (id.includes("react-dom") || id.includes("react-router") || id.match(/\/react\//)) {
            return "vendor-react";
          }
          if (id.includes("@radix-ui")) {
            return "vendor-radix";
          }
          if (id.includes("lucide-react")) {
            return "vendor-icons";
          }
          if (id.includes("zustand") || id.includes("immer")) {
            return "vendor-state";
          }
        },
      },
    },
  },
});
