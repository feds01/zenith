import { briefSchema } from "@acme/ai/src/domains/campaigns/brief";

export function useCampaignBrief(campaignId: string) {
  return briefSchema.parse({ goal: `Grow ${campaignId}`, audience: "CMOs" });
}
