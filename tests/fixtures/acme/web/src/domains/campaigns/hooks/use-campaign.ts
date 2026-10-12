import { useCampaigns } from "@acme/api/src/domains/campaigns/hooks/use-campaigns";
import { CAMPAIGNS_BASE_PATH } from "../api/client";

export function useCampaign(id: string) {
  return { query: useCampaigns(id), href: `${CAMPAIGNS_BASE_PATH}/${id}` };
}
