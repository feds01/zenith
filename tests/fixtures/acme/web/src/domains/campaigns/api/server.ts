import "server-only";
import { getCampaign } from "@acme/api/src/domains/campaigns/api";

export async function getCampaignForPage(id: string) {
  return getCampaign(id);
}
