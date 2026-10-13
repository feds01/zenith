"use client";

import { thumbnail } from "../lib/images";

export function Preview({ file }: { file: Blob }) {
  return <button onClick={() => thumbnail(file)}>Preview</button>;
}
