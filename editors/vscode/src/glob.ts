// Matching paths against zenith's globs, as zenith_utils does, for the globs
// of `[workspace] members`: `*` matches within a segment, `**` as a whole
// segment matches any number of them, and one or more at the end, `?` matches
// a character, `{a,b}` either alternative, and `\` escapes the next character.

/**
 * Whether the glob `pattern` matches `path`, a POSIX path that isn't empty.
 * An invalid glob matches nothing.
 */
export function globMatches(pattern: string, path: string): boolean {
  const alternatives = expandBraces(pattern);
  return (
    path !== "" && alternatives !== undefined && alternatives.some((alternative) => toRegExp(alternative).test(path))
  );
}

/** The alternatives of `pattern`, with its `{a,b}` expanded, or `undefined` if a brace isn't closed. */
function expandBraces(pattern: string): string[] | undefined {
  // 1. The first brace, which isn't escaped, and the one that closes it.
  let open = -1;
  let depth = 0;
  const commas: number[] = [];
  for (let index = 0; index < pattern.length; index++) {
    const char = pattern[index];
    if (char === "\\") {
      index++;
    } else if (char === "{") {
      if (depth === 0) {
        open = index;
      }
      depth++;
    } else if (char === "," && depth === 1) {
      commas.push(index);
    } else if (char === "}" && depth > 0) {
      depth--;
      if (depth === 0) {
        // 2. Each alternative in its place, with the rest expanded too.
        const bounds = [open, ...commas, index];
        const rest = expandBraces(pattern.slice(index + 1));
        if (rest === undefined) {
          return undefined;
        }
        const results: string[] = [];
        for (let part = 0; part + 1 < bounds.length; part++) {
          const inner = expandBraces(pattern.slice(bounds[part]! + 1, bounds[part + 1]));
          if (inner === undefined) {
            return undefined;
          }
          for (const each of inner) {
            for (const after of rest) {
              results.push(pattern.slice(0, open) + each + after);
            }
          }
        }
        return results;
      }
    }
  }
  return depth === 0 ? [pattern] : undefined;
}

/** The regular expression of `pattern`, which has no braces. */
function toRegExp(pattern: string): RegExp {
  const segments = pattern.replace(/^(\.\/)+/, "").split("/");
  let source = "";
  segments.forEach((segment, index) => {
    const last = index === segments.length - 1;
    if (segment === "**") {
      // A trailing `/**` matches one or more segments, and another `**` any number.
      const separator = index === 0 ? "" : "/";
      source += last ? (index === 0 ? "[^/]+(?:/[^/]+)*" : "(?:/[^/]+)+") : `${separator}(?:[^/]+/)*`;
      return;
    }
    if (index > 0 && segments[index - 1] !== "**") {
      source += "/";
    }
    for (let char = 0; char < segment.length; char++) {
      const each = segment[char]!;
      if (each === "\\" && char + 1 < segment.length) {
        char++;
        source += escapeRegExp(segment[char]!);
      } else if (each === "*") {
        source += "[^/]*";
      } else if (each === "?") {
        source += "[^/]";
      } else {
        source += escapeRegExp(each);
      }
    }
  });
  return new RegExp(`^${source}$`);
}

/** `text` with the characters that regular expressions give a meaning to escaped. */
function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
}
