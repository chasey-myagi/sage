/**
 * Workspace layout names shared by every layer above core.
 *
 * Guarantees: one definition of the runtime directory name, so tools,
 * plugins, and apps agree on where sage keeps its own files without
 * importing each other.
 *
 * Intentionally not in the Log: the directory name itself; it is layout,
 * not model input.
 */

/**
 * Sage's runtime directory under a workspace (and under the user's home):
 * session logs (`.sage/sessions`), spilled bash output (`.sage/tool-output`),
 * and subtask presets (`.sage/subtasks`). Its contents are never part of the
 * user's work; each consumer documents how it skips the directory.
 */
export const SAGE_DIR_NAME = ".sage";
