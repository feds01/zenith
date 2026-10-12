"use client";

import { Button } from "@acme/ui/button";
import { createDocument } from "legacy-editor";
import type { EditorState } from "../types";

export function EditorCanvas({ initial }: { initial?: EditorState }) {
  const document = initial?.document ?? createDocument();
  return (
    <div>
      {document.blocks.length}
      <Button label="Save" />
    </div>
  );
}
