import { readFileSync } from "node:fs";

export function readReportTemplate(name: string): string {
  return readFileSync(name, "utf8");
}
