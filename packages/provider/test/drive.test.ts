import { expect, it } from "vitest";
import { driveStream } from "../src/drive.ts";
import { drain, fixtureFetch } from "./support.ts";

// No shipped adapter throws from `onEvent`, so the `other` cause is pinned
// through the shared driver with a decoder that does.
it("finishes with cause other, keeping chunks queued before it, when the adapter's decoder throws", async () => {
  const { fetchImpl } = fixtureFetch("data: {}\n\n");
  const chunks = await drain(
    driveStream(
      { apiKey: "k", baseUrl: "https://example.test", fetchImpl },
      {
        path: "/v1",
        headers: {},
        json: {},
        permanentErrors: new Set(),
        onEvent(_event, driver) {
          driver.push({ type: "block_start", index: 0, blockType: "text" });
          throw new Error("decoder bug");
        },
      },
    ),
  );
  expect(chunks.at(0)).toEqual({
    type: "block_start",
    index: 0,
    blockType: "text",
  });
  expect(chunks.at(-1)).toEqual({
    type: "finish",
    kind: "error",
    error: { message: "decoder bug", cause: "other" },
  });
});
