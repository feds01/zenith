import { type Campaign } from "@acme/api";

export type CampaignRow = Campaign & { selected: boolean };

export const CAMPAIGN_STATUS_LABELS = { draft: "Draft", live: "Live", archived: "Archived" } as const;
