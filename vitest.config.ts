import { defineConfig } from "vitest/config";
import { resolve } from "path";

export default defineConfig({
  resolve: {
    alias: {
      $lib: resolve(__dirname, "src/lib"),
    },
  },
  test: {
    environment: "happy-dom",
    // Node 26 defines localStorage/sessionStorage as globals set to undefined,
    // preventing happy-dom's populateGlobal from installing them. This setup
    // file re-installs them with a simple in-memory shim so tests can use
    // localStorage directly. See src/test-setup.ts.
    setupFiles: ["src/test-setup.ts"],
    include: ["src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary", "html"],
      include: ["src/lib/**/*.ts"],
      exclude: ["src/lib/**/*.test.ts", "src/lib/**/*.d.ts"],
    },
  },
});
