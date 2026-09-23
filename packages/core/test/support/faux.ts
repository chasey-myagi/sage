// Old import path for the faux provider, kept only because apps/sage and
// plugins compiler tests still import it. The one definition is
// `@sage/core/testing`; delete this file once those imports move there.
export type { StreamChunk as FauxChunk } from "@sage/provider/contract";
export {
  createFaux,
  FAUX_MODEL as MODEL,
  FAUX_SYSTEM as SYSTEM,
  FAUX_USAGE as USAGE,
  stopScript,
  toolScript,
} from "../../src/testing/index.ts";
