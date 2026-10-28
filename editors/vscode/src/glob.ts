// Matching paths against zenith's globs, as zenith_utils does, for the globs
// of `[workspace] members`: `*` matches within a segment, `**` as a whole
// segment matches any number of them, and one or more at the end, `?` matches
// a character, `{a,b}` either alternative, and `\` escapes the next character.
// Every other character is literal.

import picomatch from "picomatch/posix";

/** picomatch's options for zenith's dialect, in which brackets, `!` and extglobs are literal. */
const OPTIONS: picomatch.PicomatchOptions = {
  dot: true,
  literalBrackets: true,
  nobracket: true,
  noextglob: true,
  nonegate: true,
};

/** Whether the glob `pattern` matches `path`, a POSIX path that isn't empty. */
export function globMatches(pattern: string, path: string): boolean {
  // picomatch's trailing `/**` also matches the directory itself, which zenith's doesn't.
  const parent = pattern.endsWith("/**") && picomatch.isMatch(path, pattern.slice(0, -3), OPTIONS);
  return path !== "" && !parent && picomatch.isMatch(path, pattern, OPTIONS);
}
