import { readFile } from "node:fs/promises";
import { tools } from "./tools.js";

type ToolsModule = typeof import("./tools.js");

export async function startServer(): Promise<ToolsModule["tools"]> {
  const manifest = await readFile("manifest.json", "utf8");
  console.log(`mcp: ${manifest.length} bytes of manifest`);
  return tools;
}
