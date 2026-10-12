import { assistantAgent } from "@/mastra/agent";

export function Assistant() {
  return <aside>{assistantAgent.name}</aside>;
}
