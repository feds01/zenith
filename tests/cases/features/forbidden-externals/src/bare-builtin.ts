import { readFileSync } from "fs";

export const read = (path: string) => readFileSync(path, "utf8");
