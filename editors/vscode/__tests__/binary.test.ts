// The tests of finding the binary to run, and the folders that zenith checks,
// on directories made for each.

import * as assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";

import { findBinaries, hasLsp, type Search } from "../src/binary";
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

/** The files of zenith's npm packages for Linux on x86-64, under `modules`, a `node_modules` directory. */
function npmPackages(modules: string): Record<string, string> {
  return {
    [`${modules}/@feds01/zenith/package.json`]: `{ "name": "@feds01/zenith" }`,
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

describe("findBinaries", () => {
  it("runs only the binary that the setting names, even one that isn't there", () => {
    const root = directory({ ...npmPackages("project/node_modules"), "extension/bin/zenith": "" });
    const found = (setting: string) => findBinaries(search(root, { setting }));
    assert.deepEqual(found("/opt/zenith"), [{ path: "/opt/zenith", source: "setting" }]);
    assert.deepEqual(found("tools/zenith"), [{ path: join(root, "project/tools/zenith"), source: "setting" }]);
    assert.deepEqual(found("~/bin/zenith"), [{ path: join(homedir(), "bin/zenith"), source: "setting" }]);
  });

  it("tries the project's binary, then the extension's, then the first on the PATH", () => {
    const root = directory({
      ...npmPackages("project/node_modules"),
      "extension/bin/zenith": "",
      "first/other": "",
      "second/zenith": "",
      "third/zenith": "",
    });
    const path = ["", join(root, "first"), join(root, "second"), join(root, "third")].join(delimiter);
    assert.deepEqual(findBinaries(search(root, { path })), [
      { path: join(root, "project/node_modules/@feds01/zenith-linux-x64/zenith"), source: "project" },
      { path: join(root, "extension/bin/zenith"), source: "bundled" },
      { path: join(root, "second/zenith"), source: "path" },
    ]);
    assert.deepEqual(findBinaries(search(directory({}))), []);
  });

  it("finds the binary of the npm package of the monorepo that a folder is in", () => {
    const root = directory(npmPackages("project/node_modules"));
    const folder = join(root, "project/packages/web");
    mkdirSync(folder, { recursive: true });
    const binary = join(root, "project/node_modules/@feds01/zenith-linux-x64/zenith");
    assert.deepEqual(findBinaries(search(root, { folder })), [{ path: binary, source: "project" }]);
  });

  it("finds the project's binary where pnpm leaves it, next to zenith's package", () => {
    // pnpm links zenith's package into node_modules, but not its dependencies.
    const store = "project/node_modules/.pnpm/@feds01+zenith@0.5.0/node_modules";
    const root = directory(npmPackages(store));
    mkdirSync(join(root, "project/node_modules/@feds01"), { recursive: true });
    const link = join(root, "project/node_modules/@feds01/zenith");
    symlinkSync(join(root, store, "@feds01/zenith"), link, "junction");
    const binary = join(root, store, "@feds01/zenith-linux-x64/zenith");
    assert.deepEqual(findBinaries(search(root)), [{ path: binary, source: "project" }]);
  });

  it("finds no project binary when its package has none for the platform", () => {
    const root = directory({ ...npmPackages("project/node_modules"), "extension/bin/zenith": "" });
    const bundled = { path: join(root, "extension/bin/zenith"), source: "bundled" };
    assert.deepEqual(findBinaries(search(root, { arch: "arm64" })), [bundled]);
  });

  it("finds `zenith.exe` on Windows", () => {
    const root = directory({ "extension/bin/zenith": "", "extension/bin/zenith.exe": "" });
    const bundled = { path: join(root, "extension/bin/zenith.exe"), source: "bundled" };
    assert.deepEqual(findBinaries(search(root, { platform: "win32" })), [bundled]);
  });
});

describe("hasLsp", () => {
  it("tells a zenith with `zenith lsp` from one that predates it", async function () {
    if (process.platform === "win32") {
      this.skip();
    }
    const root = directory({
      "new/zenith": '#!/bin/sh\n[ "$1 $2" = "lsp --help" ]\n',
      "old/zenith": "#!/bin/sh\necho \"error: unrecognized subcommand 'lsp'\" >&2\nexit 2\n",
    });
    chmodSync(join(root, "new/zenith"), 0o755);
    chmodSync(join(root, "old/zenith"), 0o755);
    assert.equal(await hasLsp(join(root, "new/zenith")), true);
    assert.equal(await hasLsp(join(root, "old/zenith")), false);
    assert.equal(await hasLsp(join(root, "missing/zenith")), false);
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

  it("is the folder itself when the arguments name its project", () => {
    const root = directory({ "other/a.ts": "" });
    const folder = join(root, "other");
    assert.equal(projectOf(folder, ["--config", "../zenith.toml"]), `folder:${folder}`);
    assert.equal(projectOf(folder, ["--root=.."]), `folder:${folder}`);
    assert.equal(projectOf(folder, ["--no-cache"]), undefined);
  });
});
