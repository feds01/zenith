import { readFile } from "node:fs/promises";

export const readAsync = (path: string) => readFile(path, "utf8");
