"use client";

import { upload } from "../lib/actions";

export function SaveButton({ data }: { data: FormData }) {
  return <button onClick={() => upload(data)}>Save</button>;
}
