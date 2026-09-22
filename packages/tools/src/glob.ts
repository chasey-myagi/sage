/**
 * Small glob matcher for find and the grep fallback.
 *
 * Contract: supports `*`, `?`, and `**`. The pattern is matched against a
 * workspace-relative posix path.
 *
 * Intentionally not in the Log: compiled regular expressions.
 */

export function globToRegExp(glob: string): RegExp {
  let source = "^";
  for (let index = 0; index < glob.length; index += 1) {
    const char = glob[index] ?? "";
    const next = glob[index + 1];
    if (char === "*" && next === "*") {
      if (glob[index + 2] === "/") {
        source += "(?:.*/)?";
        index += 2;
        continue;
      }
      source += ".*";
      index += 1;
      continue;
    }
    if (char === "*") {
      source += "[^/]*";
      continue;
    }
    if (char === "?") {
      source += "[^/]";
      continue;
    }
    if ("\\^$+?.()|[]{}".includes(char)) {
      source += `\\${char}`;
      continue;
    }
    source += char;
  }
  source += "$";
  return new RegExp(source);
}
