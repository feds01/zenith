// Which project zenith checks for a workspace folder, if any: folders of the
// same project share a server, which checks it once for them all.

import { readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";

import type * as Toml from "smol-toml" with { "resolution-mode": "import" };

// smol-toml's types are of its ES module, and it has a CommonJS one too, which
// this CommonJS extension requires.
const { parse } = require("smol-toml") as typeof Toml;

import { isFile } from "./binary";
import { globMatches } from "./glob";

/** The name of zenith's configuration file, as zenith_config names it. */
export const CONFIG_FILE_NAME = "zenith.toml";

/**
 * The project that zenith checks for the workspace folder at `folder`, when
 * it runs there with `args`, by a key that is the same for the folders of
 * one project: the configuration file that it finds, as `zenith check`
 * would, in the folder or one above it. A folder whose arguments name its
 * project, with `--config` or `--root`, is a project of its own. `undefined`
 * if zenith checks nothing there.
 */
export function projectOf(folder: string, args: readonly string[]): string | undefined {
  const named = args.some((arg) =>
    ["--config", "--root"].some((option) => arg === option || arg.startsWith(`${option}=`)),
  );
  return named ? `folder:${folder}` : findConfig(folder);
}

/**
 * The configuration file of the project that `folder` is in, if there is
 * one, as zenith finds it (`Config::discover` in zenith_config): the nearest
 * in the folder or above it, or, if that is a member's, the root's that lists
 * it in `[workspace] members`.
 */
export function findConfig(folder: string): string | undefined {
  const nearest = nearestConfig(folder);
  if (nearest === undefined) {
    return undefined;
  }
  const dir = dirname(nearest);
  for (let ancestor = dir; dirname(ancestor) !== ancestor;) {
    ancestor = dirname(ancestor);
    const config = join(ancestor, CONFIG_FILE_NAME);
    if (isFile(config) && isMember(config, relative(ancestor, dir).split(sep).join("/"))) {
      return config;
    }
  }
  return nearest;
}

/** The configuration file in `folder` or the nearest folder above it, if there is one. */
function nearestConfig(folder: string): string | undefined {
  for (let dir = folder; ; dir = dirname(dir)) {
    const config = join(dir, CONFIG_FILE_NAME);
    if (isFile(config)) {
      return config;
    }
    if (dirname(dir) === dir) {
      return undefined;
    }
  }
}

/**
 * Whether the configuration file at `config` lists the directory `member`, a
 * POSIX path relative to its own, in `[workspace] members`. A file that can't
 * be read, or isn't valid TOML, lists none.
 */
function isMember(config: string, member: string): boolean {
  try {
    const workspace = parse(readFileSync(config, "utf8"))["workspace"];
    const members = typeof workspace === "object" && workspace !== null ? (workspace as Toml.TomlTable)["members"] : [];
    return Array.isArray(members) && members.some((glob) => typeof glob === "string" && globMatches(glob, member));
  } catch {
    return false;
  }
}
