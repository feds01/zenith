import type { Campaign } from "@acme/api";

export const formatCampaignDate = (campaign: Campaign, date: Date) => `${campaign.name} (${date.toISOString()})`;
