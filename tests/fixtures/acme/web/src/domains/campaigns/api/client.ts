import { campaigns } from "@acme/api";

export const CAMPAIGNS_BASE_PATH = "/campaigns";

export const fetchCampaign = async (id: string) => campaigns.getCampaign(id);
