// Which project zenith checks for a workspace folder, if any: folders of the
// same project share a server, which checks it once for them all.

import { dirname, join } from "node:path";

import { isFile } from "./binary";

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
 * The configuration file in `folder` or the nearest folder above it, if
 * there is one, as zenith finds it (`Config::discover` in zenith_config).
 */
export function findConfig(folder: string): string | undefined {
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
