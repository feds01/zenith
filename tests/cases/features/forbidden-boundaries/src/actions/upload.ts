"use server";

import "server-only";
import { writeFile } from "node:fs/promises";
import { thumbnail } from "../lib/images";

export async function upload(data: FormData) {
  const file = data.get("file") as File;
  await writeFile(`/tmp/${file.name}`, await thumbnail(file));
}
