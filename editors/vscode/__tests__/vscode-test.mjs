// The configuration of the extension's tests, which `bun run test` runs in a
// VS Code that it downloads, on a copy of the fixture: a workspace of
// projects, and a folder that zenith doesn't check. The extension runs the
// binary in bin/, as it would once packaged, which `just vscode::test` copies
// there.

import { cpSync, existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "@vscode/test-cli";

/** The directory of the tests, and of the extension above it. */
const here = dirname(fileURLToPath(import.meta.url));
const extension = dirname(here);

const binary = join(extension, "bin", process.platform === "win32" ? "zenith.exe" : "zenith");
if (!existsSync(binary)) {
  throw new Error(
    `there is no ${binary} for the extension to run: build zenith, and copy it there, as \`just vscode::test\` does`,
  );
}

// A copy, so that the tests can change its files. `realpathSync`, as the
// temporary directory is a symlink on macOS.
const workspace = realpathSync(mkdtempSync(join(tmpdir(), "zenith-vscode-")));
cpSync(join(here, "fixture"), workspace, { recursive: true });

// The project `pinned` depends on a zenith that predates `zenith lsp`: its
// binary, for this platform, fails as such a zenith would.
const modules = join(workspace, "pinned", "node_modules", "@feds01");
const platformPackage = join(modules, `zenith-${process.platform}-${process.arch}`);
mkdirSync(platformPackage, { recursive: true });
mkdirSync(join(modules, "zenith"), { recursive: true });
writeFileSync(join(modules, "zenith", "package.json"), `{ "name": "@feds01/zenith" }\n`);
writeFileSync(
  join(platformPackage, "package.json"),
  `{ "name": "@feds01/zenith-${process.platform}-${process.arch}" }\n`,
);
writeFileSync(join(platformPackage, basename(binary)), "#!/bin/sh\nexit 2\n", { mode: 0o755 });

// VS Code's data, in a directory with a short path: the path of its socket
// must be short enough for macOS, which the default, under this one, may not be.
const userData = realpathSync(mkdtempSync(join(tmpdir(), "zenith-vscode-data-")));

// Paths are relative to this file.
export default defineConfig({
  extensionDevelopmentPath: "..",
  files: "../out/__tests__/**/*.test.js",
  workspaceFolder: join(workspace, "workspace.code-workspace"),
  // The workspace is trusted, as the extension runs in no other. Other
  // extensions are left out.
  launchArgs: ["--disable-workspace-trust", "--disable-extensions", `--user-data-dir=${userData}`],
  mocha: { ui: "bdd", timeout: 60_000 },
});
