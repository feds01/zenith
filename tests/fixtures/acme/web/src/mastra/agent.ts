import { Agent } from "@mastra/core/agent";
import { researchTool } from "./tools";

export const assistantAgent = new Agent({
  name: "assistant",
  instructions: "Help marketers plan campaigns.",
  model: "openai/gpt-5",
  tools: { researchTool },
});
