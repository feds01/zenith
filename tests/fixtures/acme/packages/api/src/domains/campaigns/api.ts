import { slugify } from "@acme/utils";
import { type Campaign, CAMPAIGN_STATUSES } from "./types";

export function getCampaign(id: string): Campaign {
  return { id, name: slugify(id), status: CAMPAIGN_STATUSES[0] };
}

export type CampaignsApi = { getCampaign: typeof getCampaign };
