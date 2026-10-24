#!/usr/bin/env node
// Packages the extension for a release: a VSIX for each VS Code platform that
// zenith has a binary for, with that binary, and a universal VSIX without one,
// for the other platforms, which runs the project's zenith or the one on the
// `PATH`.
//
//     node editors/vscode/package.mjs <version> <npm-packages> <out>
//
// The binaries are taken from the platform packages that `npm/assemble.mjs`
// assembles in `npm-packages`, e.g. `zenith-darwin-arm64/zenith`: Node names
// platforms as VS Code does. Build the extension first, with `npm run build`.

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** A version as the Marketplace takes it, which has no pre-releases, e.g. `0.1.0`. */
const VERSION = /^\d+\.\d+\.\d+$/;

/** The files of the extension that a VSIX has, besides the binary. */
const FILES = ["dist/extension.js", "README.md", "icon.png"];

const here = dirname(fileURLToPath(import.meta.url));
const vsce = join(here, "node_modules", ".bin", process.platform === "win32" ? "vsce.cmd" : "vsce");

/** Print `message` to stderr, and exit with an error. */
function fail(message) {
  process.stderr.write(`error: ${message}\n`);
  process.exit(1);
}

/** The VS Code platforms that the binary of the npm package for `os` and `cpu` runs on. */
function platforms(os, cpu) {
  // The Linux binaries are static, so they run on Alpine too.
  return os === "linux" ? [`linux-${cpu}`, `alpine-${cpu}`] : [`${os}-${cpu}`];
}

/**
 * Package the extension at `version` into `out`, for `platform`, with the
 * binary at `binary`, or as the universal VSIX if `platform` is undefined.
 */
function packageFor(version, out, platform, binary) {
  const stage = mkdtempSync(join(tmpdir(), "zenith-vscode-"));
  try {
    // 1. Its files, the licence and the binary.
    const files = [...FILES, "LICENSE"];
    for (const file of FILES) {
      mkdirSync(dirname(join(stage, file)), { recursive: true });
      copyFileSync(join(here, file), join(stage, file));
    }
    copyFileSync(join(here, "..", "..", "LICENSE"), join(stage, "LICENSE"));
    if (binary !== undefined) {
      const to = join("bin", binary.endsWith(".exe") ? "zenith.exe" : "zenith");
      mkdirSync(join(stage, "bin"), { recursive: true });
      copyFileSync(binary, join(stage, to));
      chmodSync(join(stage, to), 0o755);
      files.push(to);
    }

    // 2. The manifest, at the release's version, with just those files, and
    //    without what only building the extension needs: it is bundled, with
    //    its dependencies.
    const source = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));
    const { scripts, dependencies, devDependencies, overrides, ...manifest } = source;
    const packaged = { ...manifest, version, files: files.map((file) => file.replaceAll("\\", "/")) };
    writeFileSync(join(stage, "package.json"), `${JSON.stringify(packaged, null, 2)}\n`);

    // 3. The VSIX.
    const vsix = join(out, `zenith-${platform ?? "universal"}-${version}.vsix`);
    const target = platform === undefined ? [] : ["--target", platform];
    execFileSync(vsce, ["package", "--no-dependencies", ...target, "--out", vsix], {
      cwd: stage,
      stdio: ["ignore", "ignore", "inherit"],
      // npm's `.cmd` shims run only in a shell on Windows.
      shell: process.platform === "win32",
    });
    console.log(vsix);
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

// 1. Read the arguments.
const [version, npmPackages, out] = process.argv.slice(2);
if (!version || !npmPackages || !out) {
  fail("usage: node editors/vscode/package.mjs <version> <npm-packages> <out>");
}
if (!VERSION.test(version)) {
  fail(`\`${version}\` isn't a version that the Marketplace takes, like 0.1.0, without a pre-release`);
}
if (!existsSync(join(here, "dist", "extension.js"))) {
  fail("the extension isn't built: run `npm run build` in editors/vscode");
}
mkdirSync(resolve(out), { recursive: true });

// 2. A VSIX for each platform of each platform package.
const packages = readdirSync(npmPackages, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && /^zenith-[a-z0-9]+-[a-z0-9]+$/.test(entry.name))
  .map((entry) => entry.name);
if (packages.length === 0) {
  fail(`${npmPackages} has no platform packages, such as \`zenith-linux-x64\``);
}
for (const name of packages) {
  const [, os, cpu] = name.split("-");
  const binary = ["zenith", "zenith.exe"].map((file) => join(npmPackages, name, file)).find((path) => existsSync(path));
  if (binary === undefined) {
    fail(`${join(npmPackages, name)} has no binary`);
  }
  for (const platform of platforms(os, cpu)) {
    packageFor(version, resolve(out), platform, binary);
  }
}

// 3. And the universal one, without a binary.
packageFor(version, resolve(out), undefined, undefined);
