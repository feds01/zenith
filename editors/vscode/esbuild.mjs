// Bundles the extension, with the language client, into dist/extension.js,
// which is all that the packaged extension runs: `node esbuild.mjs`, or
// `node esbuild.mjs --watch` to rebuild on every change.

import * as esbuild from "esbuild";

const options = {
  entryPoints: ["src/extension.ts"],
  outfile: "dist/extension.js",
  bundle: true,
  platform: "node",
  target: "node20",
  format: "cjs",
  // VS Code provides the `vscode` module to its extensions.
  external: ["vscode"],
  sourcemap: true,
  minify: !process.argv.includes("--watch"),
  logLevel: "info",
};

if (process.argv.includes("--watch")) {
  const context = await esbuild.context(options);
  await context.watch();
} else {
  await esbuild.build(options);
}
