import path from "path";
import { assistantAgent } from "@/mastra/agent";

export async function POST(request: Request) {
  const transcript = path.join("transcripts", assistantAgent.name);
  return Response.json({ transcript, from: request.url });
}
