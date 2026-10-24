// The tests of the extension in VS Code, on the workspace of test/fixture: the
// projects `web-api`, `app-db`, `nested`, whose folders are its packages, and
// `pinned`, which each break a contract, and `plain`, which zenith doesn't
// check.

import * as assert from "node:assert/strict";
import { rmSync, writeFileSync } from "node:fs";

import * as vscode from "vscode";

import type { Api } from "../src/extension";

/** How long to wait for the extension to do something. */
const TIMEOUT = 30_000;

/** The workspace folder named `name`. */
function folder(name: string): vscode.WorkspaceFolder {
  const found = vscode.workspace.workspaceFolders?.find((workspaceFolder) => workspaceFolder.name === name);
  assert.ok(found, `the workspace has no folder ${name}`);
  return found;
}

/** The URI of the file at `path` in the workspace folder named `name`. */
function file(name: string, path: string): vscode.Uri {
  return vscode.Uri.joinPath(folder(name).uri, path);
}

/** Wait until `done` returns something, and return it: on each of `changes`, if given, and every so often. */
async function waitFor<T>(what: string, done: () => T | undefined, changes?: vscode.Event<unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    const check = () => {
      const result = done();
      if (result !== undefined) {
        finish();
        resolve(result);
      }
    };
    const subscription = changes?.(check);
    const poll = setInterval(check, 100);
    const timer = setTimeout(() => {
      finish();
      reject(new Error(`timed out waiting for ${what}`));
    }, TIMEOUT);
    const finish = () => {
      subscription?.dispose();
      clearInterval(poll);
      clearTimeout(timer);
    };
    check();
  });
}

/** Wait until zenith's diagnostics of `uri` number `count`, and return them. */
function diagnostics(uri: vscode.Uri, count: number): Promise<vscode.Diagnostic[]> {
  const current = () => vscode.languages.getDiagnostics(uri).filter((diagnostic) => diagnostic.source === "zenith");
  return waitFor(
    `${count} diagnostics of ${vscode.workspace.asRelativePath(uri)}`,
    () => {
      const found = current();
      return found.length === count ? found : undefined;
    },
    vscode.languages.onDidChangeDiagnostics,
  );
}

/**
 * Change the workspace's folders with `change`, and wait until VS Code has:
 * it refuses another change until then.
 */
async function changeFolders(change: () => boolean): Promise<void> {
  const changed = new Promise<void>((resolve) => {
    const subscription = vscode.workspace.onDidChangeWorkspaceFolders(() => {
      subscription.dispose();
      resolve();
    });
  });
  assert.ok(change(), "VS Code takes the change of the workspace's folders");
  await changed;
}

/** Replace the text of the file at `uri` with `text` on disk, as another program would, without saving it in an editor. */
async function writeFile(uri: vscode.Uri, text: string): Promise<void> {
  await vscode.workspace.fs.writeFile(uri, new TextEncoder().encode(text));
}

/** Replace the text of the file at `uri` with `text` in an editor, and save it. */
async function write(uri: vscode.Uri, text: string): Promise<void> {
  const document = await vscode.workspace.openTextDocument(uri);
  const edit = new vscode.WorkspaceEdit();
  edit.replace(uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), text);
  assert.ok(await vscode.workspace.applyEdit(edit));
  assert.ok(await document.save());
}

describe("the extension", () => {
  let api: Api;
  /** The folders of each server, and where its binary is from. */
  const servers = () => api.servers().map(({ folders, binary }) => [folders.map(({ name }) => name), binary.source]);
  /** Whether a server checks the folder `name`. */
  const checked = (name: string) => servers().some(([folders]) => folders.includes(name));

  before(async () => {
    const extension = vscode.extensions.getExtension<Api>("feds01.zenith");
    assert.ok(extension, "the extension is installed");
    api = await extension.activate();
  });

  it("runs a server for each project that the folders are in", () => {
    // `nested` has its zenith.toml above its folders, which share its server.
    // `pinned` depends on a zenith without `zenith lsp`, so the extension's
    // binary runs instead.
    assert.deepEqual(servers(), [
      [["web-api"], "bundled"],
      [["app-db"], "bundled"],
      [["nested-web", "nested-api"], "bundled"],
      [["pinned"], "bundled"],
    ]);
  });

  it("shows where each project breaks its contracts", async () => {
    const [web] = await diagnostics(file("web-api", "web/a.ts"), 1);
    assert.equal(web?.code, "web-api");
    assert.equal(web?.severity, vscode.DiagnosticSeverity.Error);
    assert.equal(web?.message, "`web` must not import `api`; remove this import\nMove what `web` needs out of `api`.");
    // `"../api/b"`, on the first line.
    assert.deepEqual(web?.range, new vscode.Range(0, 18, 0, 28));

    const [app] = await diagnostics(file("app-db", "app/main.ts"), 1);
    assert.equal(app?.code, "app-db");
    const [nested] = await diagnostics(file("nested-web", "a.ts"), 1);
    assert.equal(nested?.code, "packages");
    await diagnostics(file("pinned", "web/a.ts"), 1);
  });

  it("checks the project again when a file is saved", async () => {
    const uri = file("web-api", "web/a.ts");
    const original = (await vscode.workspace.openTextDocument(uri)).getText();
    await write(uri, "export const a = 1;\n");
    await diagnostics(uri, 0);
    await write(uri, original);
    await diagnostics(uri, 1);
  });

  it("checks a project again when a file of any of its folders is saved", async () => {
    const uri = file("nested-web", "a.ts");
    const original = (await vscode.workspace.openTextDocument(uri)).getText();
    await writeFile(uri, "export const a = 1;\n");
    // A save in the other folder of the project.
    await write(file("nested-api", "b.ts"), "export const b = 2;\n");
    await diagnostics(uri, 0);
    await writeFile(uri, original);
    await write(file("nested-api", "b.ts"), "export const b = 1;\n");
    await diagnostics(uri, 1);
  });

  it("shows a mistake in zenith.toml where it is, until it is fixed", async () => {
    const uri = file("web-api", "zenith.toml");
    const original = (await vscode.workspace.openTextDocument(uri)).getText();
    await write(uri, original.replace('type = "forbidden"', 'type = "forbidden"\nseverty = "warn"'));
    const [error] = await diagnostics(uri, 1);
    assert.equal(error?.message, "unknown key `severty`\ndid you mean `severity`?");
    await write(uri, original);
    await diagnostics(uri, 0);
  });

  it("checks a folder once it has a zenith.toml, until it no longer has one", async () => {
    const config = file("plain", "zenith.toml");
    writeFileSync(config.fsPath, "");
    await waitFor("a server of `plain`", () => (checked("plain") ? true : undefined));
    rmSync(config.fsPath);
    await waitFor("no server of `plain`", () => (checked("plain") ? undefined : true));
  });

  it("checks a folder that is added to the workspace, until it is removed", async () => {
    const count = vscode.workspace.workspaceFolders?.length ?? 0;
    const extra = vscode.Uri.joinPath(folder("web-api").uri, "..", "extra");
    await changeFolders(() => vscode.workspace.updateWorkspaceFolders(count, 0, { uri: extra, name: "extra" }));
    await waitFor("a server of `extra`", () => (checked("extra") ? true : undefined));
    await changeFolders(() => vscode.workspace.updateWorkspaceFolders(count, 1));
    await waitFor("no server of `extra`", () => (checked("extra") ? undefined : true));
  });

  it("runs no server while `zenith.path` isn't a file", async () => {
    const configuration = vscode.workspace.getConfiguration("zenith", folder("app-db").uri);
    await configuration.update("path", "missing/zenith", vscode.ConfigurationTarget.WorkspaceFolder);
    await waitFor("no server of `app-db`", () => (checked("app-db") ? undefined : true));
    await diagnostics(file("app-db", "app/main.ts"), 0);
    await configuration.update("path", undefined, vscode.ConfigurationTarget.WorkspaceFolder);
    await waitFor("a server of `app-db`", () => (checked("app-db") ? true : undefined));
    await diagnostics(file("app-db", "app/main.ts"), 1);
  });

  it("restarts the servers, which check the projects again", async () => {
    const before = servers();
    await vscode.commands.executeCommand("zenith.restart");
    assert.deepEqual(servers(), before);
    await diagnostics(file("web-api", "web/a.ts"), 1);
    await diagnostics(file("app-db", "app/main.ts"), 1);
  });
});
