import { createDocument } from "legacy-editor";

export function toEditorDocument(title: string) {
  return { ...createDocument(), title };
}
