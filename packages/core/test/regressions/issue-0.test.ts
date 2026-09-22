import { it } from "vitest";

// Regression tests are named packages/<name>/test/regressions/issue-<n>.test.ts.
// When an issue fixes a bug, put the reproducing case in issue-<n>.test.ts and
// assert view, manifest, or log. This file is the naming example. It
// intentionally does not assert a behavior.
it("issue-0 reserves the regression filename", () => {});
