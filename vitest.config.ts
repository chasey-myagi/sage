import { defineConfig } from "vitest/config";

export default defineConfig({
  // Workspace packages export `dist/` by default; the `source` condition in
  // their `exports` points tests at `src/`, so tests never need a build.
  // Setting the list replaces Vite's server defaults, hence `node` is kept.
  ssr: { resolve: { conditions: ["source", "node"] } },
  test: {
    environment: "node",
    include: [
      "scripts/**/*.test.ts",
      "packages/**/test/**/*.test.ts",
      "apps/**/test/**/*.test.ts",
      // Regression files are named test/regressions/issue-<n>.test.ts.
      "packages/**/test/regressions/**/*.test.ts",
    ],
    setupFiles: ["packages/core/src/testing/setup.ts"],
    coverage: {
      provider: "v8",
      // Thresholds stay unset until the M2 compiler lands (M2 编译器落地后设).
      include: ["packages/core/src/view/**", "packages/core/src/session.ts"],
    },
  },
});
