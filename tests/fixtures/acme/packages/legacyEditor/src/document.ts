export type EditorDocument = { blocks: string[] };

export function createDocument(): EditorDocument {
  return { blocks: [] };
}
