import { CAMPAIGN_STATUS_LABELS } from "@/domains/campaigns/types";

export const formatStatus = (status: keyof typeof CAMPAIGN_STATUS_LABELS) => CAMPAIGN_STATUS_LABELS[status];
