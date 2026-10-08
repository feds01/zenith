import { createTool } from "@mastra/core/tools";
import { z } from "zod";

export const researchTool = createTool({
  id: "research",
  description: "Research an account",
  inputSchema: z.object({ domain: z.string() }),
  execute: async () => ({ ok: true }),
});
