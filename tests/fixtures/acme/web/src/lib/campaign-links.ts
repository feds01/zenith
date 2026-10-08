// zenith-ignore web-layers: legacy deep-link helper, tracked in ENG-123
import { CAMPAIGNS_BASE_PATH } from "@/domains/campaigns/api/client";

export const campaignLink = (id: string) => `${CAMPAIGNS_BASE_PATH}/${id}`;
