import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: [
      "scripts/**/*.test.ts",
      "packages/**/test/**/*.test.ts",
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
