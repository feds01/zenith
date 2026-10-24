// Finds the zenith binaries that could run for a workspace folder, in the
// order to try them: the one that the settings name, or else the project's
// own, from its npm package, the one that comes with the extension, and
// `zenith` on the `PATH`.

import { execFile } from "node:child_process";
import { statSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";

/** How long a binary has to say whether it has `zenith lsp`, in milliseconds. */
const PROBE_TIMEOUT = 10_000;

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
 * The binaries that could run for `search.folder`, in the order to try them.
 * The one that the setting names is the only one, even if it doesn't exist,
 * so that the caller can say that the setting is wrong, rather than run
 * another.
 */
export function findBinaries(search: Search): Binary[] {
  const executable = search.platform === "win32" ? "zenith.exe" : "zenith";
  if (search.setting !== "") {
    return [{ path: settingPath(search.setting, search.folder), source: "setting" }];
  }
  const project = projectBinary(search, executable);
  const bundled = join(search.bundled, executable);
  const onPath = search.path
    .split(delimiter)
    .filter((dir) => dir !== "")
    .map((dir) => join(dir, executable))
    .find(isFile);
  return [
    ...(project === undefined ? [] : [{ path: project, source: "project" as const }]),
    ...(isFile(bundled) ? [{ path: bundled, source: "bundled" as const }] : []),
    ...(onPath === undefined ? [] : [{ path: onPath, source: "path" as const }]),
  ];
}

/**
 * Whether the binary at `path` runs, and has `zenith lsp`, which a zenith
 * that predates it doesn't: a project may pin one.
 */
export function hasLsp(path: string): Promise<boolean> {
  return new Promise((resolve) => {
    execFile(path, ["lsp", "--help"], { timeout: PROBE_TIMEOUT }, (error) => resolve(error === null));
  });
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

/**
 * The binary of the project's npm package, as Node finds it from the folder:
 * that of the platform's package, which zenith's package depends on. It is
 * found from zenith's package, as package managers that don't hoist it, such
 * as pnpm, leave it there.
 */
function projectBinary(search: Search, executable: string): string | undefined {
  try {
    const main = createRequire(join(search.folder, "package.json")).resolve(`${NPM_PACKAGE}/package.json`);
    const platformPackage = `${NPM_PACKAGE}-${search.platform}-${search.arch}`;
    const manifest = createRequire(main).resolve(`${platformPackage}/package.json`);
    const binary = join(dirname(manifest), executable);
    return isFile(binary) ? binary : undefined;
  } catch {
    return undefined;
  }
}
