/**
 * Resolves TypeScript source imports written with .js specifiers.
 *
 * The packages export their .ts entry, and those files import siblings as .js.
 * Node's type stripper does not rewrite that. This hook tries the .js file,
 * then the .ts file.
 *
 * Intentionally not in the Log: module resolution.
 */

export async function resolve(specifier, context, nextResolve) {
  const relative = specifier.startsWith("./") || specifier.startsWith("../");
  if (!relative || !specifier.endsWith(".js")) {
    return nextResolve(specifier, context);
  }
  try {
    return await nextResolve(specifier, context);
  } catch (error) {
    if (!isMissing(error)) throw error;
    return nextResolve(`${specifier.slice(0, -3)}.ts`, context);
  }
}

function isMissing(error) {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === "ERR_MODULE_NOT_FOUND"
  );
}
