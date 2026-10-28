// The zenith extension: it runs `zenith lsp` for each project that the
// workspace's folders are in, which shows where the code breaks its contracts
// as diagnostics, and checks the project again whenever a file is saved.

import type { ChildProcess } from "node:child_process";

import * as vscode from "vscode";
import { LanguageClient, type LanguageClientOptions, type ServerOptions } from "vscode-languageclient/node";

import { type Binary, findBinary, isFile, type Source } from "./binary";
import { CONFIG_FILE_NAME, projectOf } from "./project";

/** The guide's page on installing zenith. */
const INSTALL_URL = "https://feds01.github.io/zenith/installation.html";

/** How long to wait for a server to exit once it is stopped, in milliseconds. */
const EXIT_TIMEOUT = 5_000;

/** What the extension offers the code that uses it, such as its tests. */
export interface Api {
  /** The servers that run, each with the workspace folders it checks and the binary it runs. */
  servers(): { folders: vscode.WorkspaceFolder[]; binary: Binary }[];
}

/** The servers, which `deactivate` stops. */
let servers: Servers | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<Api> {
  servers = new Servers(context.asAbsolutePath("bin"));
  const watcher = vscode.workspace.createFileSystemWatcher(`**/${CONFIG_FILE_NAME}`);
  const running = servers;
  context.subscriptions.push(
    watcher,
    // A folder that gains or loses its configuration file starts or stops
    // being checked, and one whose root lists it among its members, or stops
    // listing it, moves to another project.
    watcher.onDidCreate(() => running.sync()),
    watcher.onDidChange(() => running.sync()),
    watcher.onDidDelete(() => running.sync()),
    vscode.workspace.onDidChangeWorkspaceFolders(() => running.sync()),
    vscode.workspace.onDidChangeConfiguration((event) => running.configurationChanged(event)),
    vscode.commands.registerCommand("zenith.restart", () => running.restart()),
    vscode.commands.registerCommand("zenith.showOutput", () => running.showOutput()),
  );
  await running.sync();
  return { servers: () => running.list() };
}

export async function deactivate(): Promise<void> {
  await servers?.stop();
  servers = undefined;
}

/**
 * The server of each project that the workspace's folders are in. Its
 * changes are made one at a time, in the order they are asked for, as the
 * events that ask for them can come together.
 */
class Servers {
  /** The server of each project, by the project's key. */
  private readonly servers = new Map<string, Server>();

  /** The changes asked for so far, which each wait for the last. */
  private queue: Promise<void> = Promise.resolve();

  /** Whether the servers have stopped for good, after which none starts. */
  private stopped = false;

  /** @param bundled The directory of the binary that comes with the extension. */
  constructor(private readonly bundled: string) {}

  /** The servers that run, each with the folders it checks and the binary it runs. */
  list(): { folders: vscode.WorkspaceFolder[]; binary: Binary }[] {
    return [...this.servers.values()].flatMap((server) =>
      server.binary === undefined ? [] : [{ folders: server.folders, binary: server.binary }],
    );
  }

  /**
   * Start a server for each project that the folders are in and has none,
   * and stop those of the projects that no folder is in any more.
   */
  sync(): Promise<void> {
    return this.enqueue(() => this.syncNow());
  }

  /** Restart the servers of the folders whose settings change, and then sync them. */
  configurationChanged(event: vscode.ConfigurationChangeEvent): Promise<void> {
    const changed = (folder: vscode.WorkspaceFolder) =>
      ["zenith.path", "zenith.args"].some((section) => event.affectsConfiguration(section, folder));
    return this.enqueue(async () => {
      for (const server of this.servers.values()) {
        if (server.folders.some(changed)) {
          await server.restart();
        }
      }
      await this.syncNow();
    });
  }

  /** Restart every server, which finds its binary and configuration anew, and then sync them. */
  restart(): Promise<void> {
    return this.enqueue(async () => {
      for (const server of this.servers.values()) {
        await server.restart();
      }
      await this.syncNow();
    });
  }

  /** Show the output of the server of the active editor's folder, or else of the one the user picks. */
  async showOutput(): Promise<void> {
    const servers = [...this.servers.values()];
    const active = vscode.window.activeTextEditor?.document.uri;
    const folder = active === undefined ? undefined : vscode.workspace.getWorkspaceFolder(active);
    const current = servers.find((server) =>
      server.folders.some((each) => each.uri.toString() === folder?.uri.toString()),
    );
    if (current !== undefined || servers.length === 1) {
      (current ?? servers[0])?.output.show();
      return;
    }
    if (servers.length === 0) {
      void vscode.window.showInformationMessage(
        `zenith checks no folder of this workspace: none has a ${CONFIG_FILE_NAME}, in it or above it.`,
      );
      return;
    }
    const picked = await vscode.window.showQuickPick(
      servers.map((server) => ({ label: server.name, server })),
      { placeHolder: "The server whose output to show" },
    );
    picked?.server.output.show();
  }

  /** Stop every server, for good. */
  stop(): Promise<void> {
    this.stopped = true;
    return this.enqueue(async () => {
      const servers = [...this.servers.values()];
      this.servers.clear();
      await Promise.all(servers.map((server) => server.dispose()));
    });
  }

  /**
   * Start a server for each project that the folders are in and has none, or
   * whose folders change, and stop the others.
   */
  private async syncNow(): Promise<void> {
    if (this.stopped) {
      return;
    }

    // 1. The folders of each project, in the workspace's order.
    const projects = new Map<string, Folders>();
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      const project = folder.uri.scheme === "file" ? projectOf(folder.uri.fsPath, settings(folder).args) : undefined;
      const folders = project === undefined ? undefined : projects.get(project);
      if (folders !== undefined) {
        folders.push(folder);
      } else if (project !== undefined) {
        projects.set(project, [folder]);
      }
    }

    // 2. Stop the servers of projects that are gone, or whose folders change.
    const key = (folders: Folders) => folders.map((folder) => folder.uri.toString()).join("\n");
    for (const [project, server] of this.servers) {
      const folders = projects.get(project);
      if (folders === undefined || key(folders) !== key(server.folders)) {
        this.servers.delete(project);
        await server.dispose();
      }
    }

    // 3. Start a server for each project that has none.
    for (const [project, folders] of projects) {
      if (!this.servers.has(project)) {
        const server = new Server(folders, this.bundled);
        this.servers.set(project, server);
        await server.start();
      }
    }
  }

  /** Make `change` once the changes asked for before it are made, whether they failed or not. */
  private enqueue(change: () => Promise<void>): Promise<void> {
    const next = this.queue.then(change, change);
    this.queue = next.catch(() => undefined);
    return next;
  }
}

/** The settings of the extension, for a workspace folder. */
function settings(folder: vscode.WorkspaceFolder): { path: string; args: string[] } {
  const configuration = vscode.workspace.getConfiguration("zenith", folder);
  return { path: configuration.get("path", ""), args: configuration.get("args", []) };
}

/** The workspace folders of a project, of which there is at least one. */
type Folders = [vscode.WorkspaceFolder, ...vscode.WorkspaceFolder[]];

/** How a source of a binary is described in the server's output. */
const SOURCES: Record<Source, string> = {
  setting: "the `zenith.path` setting",
  project: "the project's `@feds01/zenith` package",
  bundled: "the extension",
  path: "the `PATH`",
};

/**
 * The language server of a project, which checks it for the workspace
 * folders that are in it. It runs in the first, with its settings, and is
 * told when a file of any of them is saved.
 */
class Server {
  /** What the server is called, after its folders. */
  readonly name: string;

  /** The output of the server, and of the extension about it. */
  readonly output: vscode.LogOutputChannel;

  /** The client that runs the server, while it runs. */
  private client: LanguageClient | undefined;

  /** The binary that the server runs, once it has started. */
  binary: Binary | undefined;

  /**
   * @param folders The workspace folders that the server checks.
   * @param bundled The directory of the binary that comes with the extension.
   */
  constructor(
    readonly folders: Folders,
    private readonly bundled: string,
  ) {
    this.name = `zenith (${folders.map((folder) => folder.name).join(", ")})`;
    this.output = vscode.window.createOutputChannel(this.name, { log: true });
  }

  /** The folder that the server runs in, with its settings. */
  private get folder(): vscode.WorkspaceFolder {
    return this.folders[0];
  }

  /** Start the server, or say why it can't start. */
  async start(): Promise<void> {
    // 1. Find the binary to run. The project's own is passed over if it predates
    //    `zenith lsp`.
    const { path, args } = settings(this.folder);
    const { binary, outdated } = findBinary({
      folder: this.folder.uri.fsPath,
      setting: path,
      bundled: this.bundled,
      platform: process.platform,
      arch: process.arch,
      path: process.env["PATH"] ?? "",
    });
    if (outdated !== undefined) {
      this.output.warn(`The project's \`@feds01/zenith\` is ${outdated}, which has no \`zenith lsp\`: upgrade it.`);
    }
    if (binary === undefined) {
      const why =
        outdated === undefined
          ? "it isn't installed, and the extension has no binary of its own for this platform"
          : `the project's \`@feds01/zenith\` is ${outdated}, which has no \`zenith lsp\`; upgrade it, or set \`zenith.path\``;
      void this.fail(`zenith can't check ${this.folder.name}: ${why}.`, outdated === undefined);
      return;
    }
    if (binary.source === "setting" && !isFile(binary.path)) {
      void this.fail(
        `zenith can't check ${this.folder.name}: \`zenith.path\` is ${binary.path}, which isn't a file.`,
        false,
      );
      return;
    }
    this.output.info(`Running ${binary.path}, from ${SOURCES[binary.source]}.`);

    // 2. Run it in the folder, as `zenith check` would run there. It is told of
    //    every file of its folders that is saved, and checks the project again.
    const serverOptions: ServerOptions = {
      command: binary.path,
      args: ["lsp", ...args],
      options: { cwd: this.folder.uri.fsPath },
    };
    const clientOptions: LanguageClientOptions = {
      documentSelector: this.folders.map((folder) => ({
        scheme: "file",
        pattern: { baseUri: folder.uri.toString(), pattern: "**/*" },
      })),
      workspaceFolder: this.folder,
      outputChannel: this.output,
      diagnosticCollectionName: "zenith",
    };
    const client = new LanguageClient("zenith", this.name, serverOptions, clientOptions);
    try {
      await client.start();
    } catch (error) {
      // The client has shown why already.
      await client.dispose().catch(() => undefined);
      this.output.error(
        `zenith can't check ${this.folder.name}: ${error instanceof Error ? error.message : String(error)}`,
      );
      return;
    }
    this.client = client;
    this.binary = binary;
  }

  /** Stop the server, and clear its diagnostics. */
  async stop(): Promise<void> {
    const client = this.client;
    this.client = undefined;
    this.binary = undefined;
    if (client === undefined) {
      return;
    }
    // The client logs the server's exit to the output, so it is waited for:
    // the client ends the server soon after it stops, if it doesn't exit.
    const exited = client.serverProcess === undefined ? Promise.resolve() : exit(client.serverProcess);
    try {
      if (client.isRunning()) {
        await client.stop();
      }
    } catch (error) {
      this.output.warn(`The server didn't shut down: ${error instanceof Error ? error.message : String(error)}`);
    }
    await client.dispose().catch(() => undefined);
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, EXIT_TIMEOUT))]);
  }

  /** Stop the server, and start it again. */
  async restart(): Promise<void> {
    await this.stop();
    await this.start();
  }

  /** Stop the server, for good, and close its output. */
  async dispose(): Promise<void> {
    await this.stop();
    this.output.dispose();
  }

  /** Say why the server can't run, in its output and as an error, offering to install zenith if `install`. */
  private async fail(message: string, install: boolean): Promise<void> {
    this.output.error(message);
    const actions = install ? ["Install zenith", "Show output"] : ["Show output"];
    const action = await vscode.window.showErrorMessage(message, ...actions);
    if (action === "Install zenith") {
      await vscode.env.openExternal(vscode.Uri.parse(INSTALL_URL));
    } else if (action === "Show output") {
      this.output.show();
    }
  }
}

/** Once `child` has exited. */
function exit(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve();
  }
  return new Promise((resolve) => child.once("exit", () => resolve()));
}
