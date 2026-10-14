import { readFileSync } from "node:fs";

export const readReport = () => readFileSync("report.txt", "utf8");
