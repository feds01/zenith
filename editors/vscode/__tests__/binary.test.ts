// The tests of finding the binary to run, and the folders that zenith checks,
// on directories made for each.

import * as assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";

import { findBinary, hasLsp, type Search } from "../src/binary";
import { findConfig, projectOf } from "../src/project";

/** A directory with `files`, by their paths in it, and their contents. */
function directory(files: Record<string, string>): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "zenith-vscode-")));
  for (const [path, contents] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), contents);
  }
  return root;
}

/** The files of zenith's npm packages at `version`, for Linux on x86-64, under `modules`, a `node_modules` directory. */
function npmPackages(modules: string, version = "0.6.0"): Record<string, string> {
  return {
    [`${modules}/@feds01/zenith/package.json`]: `{ "name": "@feds01/zenith", "version": "${version}" }`,
    [`${modules}/@feds01/zenith-linux-x64/package.json`]: `{ "name": "@feds01/zenith-linux-x64" }`,
    [`${modules}/@feds01/zenith-linux-x64/zenith`]: "",
  };
}

/** A search of `root`: of its `project` folder, for Linux on x86-64, with the extension's binary in `extension/bin`. */
function search(root: string, overrides: Partial<Search> = {}): Search {
  return {
    folder: join(root, "project"),
    setting: "",
    bundled: join(root, "extension/bin"),
    platform: "linux",
    arch: "x64",
    path: "",
    ...overrides,
  };
}

/** The project's binary in `root`, as `npmPackages` puts it under `project/node_modules`. */
const PROJECT = "project/node_modules/@feds01/zenith-linux-x64/zenith";

describe("findBinary", () => {
  it("runs the binary that the setting names, even one that isn't there", () => {
    const root = directory({ ...npmPackages("project/node_modules"), "extension/bin/zenith": "" });
    const found = (setting: string) => findBinary(search(root, { setting })).binary;
    assert.deepEqual(found("/opt/zenith"), { path: "/opt/zenith", source: "setting" });
    assert.deepEqual(found("tools/zenith"), { path: join(root, "project/tools/zenith"), source: "setting" });
    assert.deepEqual(found("~/bin/zenith"), { path: join(homedir(), "bin/zenith"), source: "setting" });
  });

  it("runs the project's binary, or else the extension's, or else the first on the PATH", () => {
    const files = { "first/other": "", "second/zenith": "", "third/zenith": "" };
    const root = directory({ ...npmPackages("project/node_modules"), "extension/bin/zenith": "", ...files });
    const path = ["", join(root, "first"), join(root, "second"), join(root, "third")].join(delimiter);
    const found = (overrides: Partial<Search>) => findBinary(search(root, { path, ...overrides })).binary;
    assert.deepEqual(found({}), { path: join(root, PROJECT), source: "project" });
    // Without a package for the platform, or without the extension's binary.
    const bundled = { path: join(root, "extension/bin/zenith"), source: "bundled" };
    assert.deepEqual(found({ arch: "arm64" }), bundled);
    const onPath = { path: join(root, "second/zenith"), source: "path" };
    assert.deepEqual(found({ arch: "arm64", bundled: join(root, "nowhere") }), onPath);
    assert.deepEqual(findBinary(search(directory({}))), { binary: undefined, outdated: undefined });
  });

  it("passes over the project's zenith if it predates `zenith lsp`", () => {
    const root = directory({ ...npmPackages("project/node_modules", "0.5.0"), "extension/bin/zenith": "" });
    const bundled = { path: join(root, "extension/bin/zenith"), source: "bundled" };
    assert.deepEqual(findBinary(search(root)), { binary: bundled, outdated: "0.5.0" });
    const nothing = findBinary(search(root, { bundled: join(root, "nowhere") }));
    assert.deepEqual(nothing, { binary: undefined, outdated: "0.5.0" });
  });

  it("finds the binary of the npm package of the monorepo that a folder is in", () => {
    const root = directory(npmPackages("project/node_modules"));
    const folder = join(root, "project/packages/web");
    mkdirSync(folder, { recursive: true });
    assert.deepEqual(findBinary(search(root, { folder })).binary, { path: join(root, PROJECT), source: "project" });
  });

  it("finds the project's binary where pnpm leaves it, next to zenith's package", () => {
    // pnpm links zenith's package into node_modules, but not its dependencies.
    const store = "project/node_modules/.pnpm/@feds01+zenith@0.6.0/node_modules";
    const root = directory(npmPackages(store));
    mkdirSync(join(root, "project/node_modules/@feds01"), { recursive: true });
    const link = join(root, "project/node_modules/@feds01/zenith");
    symlinkSync(join(root, store, "@feds01/zenith"), link, "junction");
    const binary = join(root, store, "@feds01/zenith-linux-x64/zenith");
    assert.deepEqual(findBinary(search(root)).binary, { path: binary, source: "project" });
  });

  it("finds `zenith.exe` on Windows", () => {
    const root = directory({ "extension/bin/zenith": "", "extension/bin/zenith.exe": "" });
    const bundled = { path: join(root, "extension/bin/zenith.exe"), source: "bundled" };
    assert.deepEqual(findBinary(search(root, { platform: "win32" })).binary, bundled);
  });
});

describe("hasLsp", () => {
  it("is every release after 0.5.0, and any version that names no release", () => {
    for (const version of ["0.5.1", "0.6.0", "0.6.0-beta.1", "1.0.0", "10.0.0"]) {
      assert.equal(hasLsp(version), true, version);
    }
    for (const version of ["0.5.0", "0.4.9", "0.1.0", "0.0.0-dev"]) {
      assert.equal(hasLsp(version), false, version);
    }
    assert.equal(hasLsp("workspace:*"), true);
  });
});

describe("projectOf", () => {
  it("is the zenith.toml in a folder or above it, the same for the folders of a project", () => {
    const root = directory({ "project/zenith.toml": "", "project/packages/web/a.ts": "", "other/a.ts": "" });
    const config = join(root, "project/zenith.toml");
    assert.equal(projectOf(join(root, "project"), []), config);
    assert.equal(projectOf(join(root, "project/packages/web"), []), config);
    assert.equal(findConfig(join(root, "project/packages/web")), config);
    assert.equal(projectOf(join(root, "other"), []), undefined);
  });

  it("is the root's zenith.toml in a member, and a project's own in one that the root doesn't list", () => {
    const root = directory({
      "zenith.toml": '[workspace]\nmembers = ["apps/*", "services/**"]\n',
      "apps/web/zenith.toml": "",
      "apps/web/src/a.ts": "",
      "services/core/api/zenith.toml": "",
      "tools/zenith.toml": "",
    });
    const config = join(root, "zenith.toml");
    assert.equal(projectOf(join(root, "apps/web"), []), config);
    assert.equal(projectOf(join(root, "apps/web/src"), []), config);
    assert.equal(projectOf(join(root, "services/core/api"), []), config);
    assert.equal(projectOf(join(root, "tools"), []), join(root, "tools/zenith.toml"));
  });

  it("is a member's own zenith.toml when the root's isn't valid TOML", () => {
    const root = directory({ "zenith.toml": "[workspace\n", "apps/web/zenith.toml": "" });
    assert.equal(projectOf(join(root, "apps/web"), []), join(root, "apps/web/zenith.toml"));
  });

  it("is the folder itself when the arguments name its project", () => {
    const root = directory({ "other/a.ts": "" });
    const folder = join(root, "other");
    assert.equal(projectOf(folder, ["--config", "../zenith.toml"]), `folder:${folder}`);
    assert.equal(projectOf(folder, ["--root=.."]), `folder:${folder}`);
    assert.equal(projectOf(folder, ["--no-cache"]), undefined);
  });
});
