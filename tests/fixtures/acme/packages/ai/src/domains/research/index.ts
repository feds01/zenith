import { Agent } from "@mastra/core";
import { toEditorDocument } from "../../connectors/editor";

export const researchAgent = new Agent({ name: "research", instructions: "Research accounts.", model: "openai/gpt-5" });

export const researchNotes = (title: string) => toEditorDocument(title);
