"use client";

import { summarizeCampaign } from "@acme/ai";
import { useCampaignBrief } from "../hooks/use-campaign-brief";

export function CampaignAiPanel({ campaignId }: { campaignId: string }) {
  const brief = useCampaignBrief(campaignId);
  return <button onClick={() => summarizeCampaign(campaignId, new Uint8Array())}>{brief.goal}</button>;
}
