import type { CampaignsApi } from "./api";

export const CAMPAIGN_STATUSES = ["draft", "live", "archived"] as const;

export type Campaign = { id: string; name: string; status: (typeof CAMPAIGN_STATUSES)[number] };

export type CampaignClient = CampaignsApi;
