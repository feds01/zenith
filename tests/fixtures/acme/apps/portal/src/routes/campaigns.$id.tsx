import { portalApi } from "@/lib/api";

export function CampaignRoute({ id }: { id: string }) {
  return <a href={portalApi.campaignUrl(id)}>{id}</a>;
}
