// Finds the zenith binary to run for a workspace folder: the one that the
// settings name, or else the project's own, from its npm package, unless it
// predates `zenith lsp`, or else the one that comes with the extension, or
// else `zenith` on the `PATH`.

import { readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";

/** The last release of zenith without `zenith lsp`. */
const LAST_WITHOUT_LSP = [0, 5, 0];

/** zenith's npm package, which depends on a package per platform with its binary. */
const NPM_PACKAGE = "@feds01/zenith";

/** Where a binary was found. */
export type Source = "setting" | "project" | "bundled" | "path";

/** A binary to run, and where it was found. */
export interface Binary {
  /** Its path. */
  path: string;

  /** Where it was found. */
  source: Source;
}

/** The binary to run, and the project's own zenith, if it was passed over. */
export interface Choice {
  /** The binary to run, if there is one. */
  binary: Binary | undefined;

  /** The version of the project's zenith, if it predates `zenith lsp`. */
  outdated: string | undefined;
}

/** Where to look for a binary. */
export interface Search {
  /** The workspace folder that the binary checks. */
  folder: string;

  /** The `zenith.path` setting, or `""` if it isn't set. */
  setting: string;

  /** The directory of the binary that comes with the extension. */
  bundled: string;

  /** The platform and the architecture to find a binary for, as Node names them. */
  platform: NodeJS.Platform;
  arch: string;

  /** The directories to look in last, separated as the platform's `PATH` is. */
  path: string;
}

/**
 * The binary to run for `search.folder`. The one that the setting names is
 * run even if it doesn't exist, so that the caller can say that the setting
 * is wrong, rather than run another.
 */
export function findBinary(search: Search): Choice {
  const executable = search.platform === "win32" ? "zenith.exe" : "zenith";
  if (search.setting !== "") {
    return { binary: { path: settingPath(search.setting, search.folder), source: "setting" }, outdated: undefined };
  }

  // 1. The project's own, unless it predates `zenith lsp`.
  const project = projectZenith(search, executable);
  const outdated = project === undefined || hasLsp(project.version) ? undefined : project.version;
  if (project?.binary !== undefined && outdated === undefined) {
    return { binary: { path: project.binary, source: "project" }, outdated };
  }

  // 2. The extension's own, which always has it.
  const bundled = join(search.bundled, executable);
  if (isFile(bundled)) {
    return { binary: { path: bundled, source: "bundled" }, outdated };
  }

  // 3. The first on the `PATH`.
  const onPath = search.path
    .split(delimiter)
    .filter((dir) => dir !== "")
    .map((dir) => join(dir, executable))
    .find(isFile);
  return { binary: onPath === undefined ? undefined : { path: onPath, source: "path" }, outdated };
}

/**
 * Whether zenith `version` has `zenith lsp`, as every release after 0.5.0
 * does. A version that names no release is taken to have it.
 */
export function hasLsp(version: string): boolean {
  const release = /^(\d+)\.(\d+)\.(\d+)/.exec(version)?.slice(1).map(Number);
  if (release === undefined) {
    return true;
  }
  const differs = release.findIndex((part, index) => part !== LAST_WITHOUT_LSP[index]);
  return differs !== -1 && (release[differs] ?? 0) > (LAST_WITHOUT_LSP[differs] ?? 0);
}

/** Whether a file is at `path`. */
export function isFile(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

/** The path that the setting `setting` names: absolute, under the home directory, or relative to `folder`. */
function settingPath(setting: string, folder: string): string {
  if (setting === "~" || setting.startsWith("~/") || setting.startsWith("~\\")) {
    return join(homedir(), setting.slice(1));
  }
  return isAbsolute(setting) ? setting : resolve(folder, setting);
}

/** The project's zenith, from its npm package. */
interface ProjectZenith {
  /** Its version. */
  version: string;

  /** Its binary for the platform, if the package manager installed it. */
  binary: string | undefined;
}

/**
 * The project's zenith, as Node finds its npm package from the folder, if
 * the project has it. Its binary is that of the platform's package, which
 * zenith's package depends on, and is found from zenith's package, as
 * package managers that don't hoist it, such as pnpm, leave it there.
 */
function projectZenith(search: Search, executable: string): ProjectZenith | undefined {
  let main: string;
  let version: string;
  try {
    main = createRequire(join(search.folder, "package.json")).resolve(`${NPM_PACKAGE}/package.json`);
    version = String((JSON.parse(readFileSync(main, "utf8")) as { version?: unknown }).version);
  } catch {
    return undefined;
  }
  try {
    const platformPackage = `${NPM_PACKAGE}-${search.platform}-${search.arch}`;
    const manifest = createRequire(main).resolve(`${platformPackage}/package.json`);
    const binary = join(dirname(manifest), executable);
    return { version, binary: isFile(binary) ? binary : undefined };
  } catch {
    return { version, binary: undefined };
  }
}
