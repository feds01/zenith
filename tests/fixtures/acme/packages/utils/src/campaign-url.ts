import { campaigns } from "@acme/api";

export const campaignUrl = (id: string) => `/campaigns/${campaigns.getCampaign(id).id}`;
