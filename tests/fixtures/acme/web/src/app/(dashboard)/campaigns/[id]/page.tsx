import { AnalyticsChart } from "@/domains/analytics/components/AnalyticsChart";
import { getCampaignForPage } from "@/domains/campaigns/api/server";
import { CampaignAiPanel } from "@/domains/campaigns/components/CampaignAiPanel";
import { CampaignHeader } from "@/domains/campaigns/components/CampaignHeader";

export default async function CampaignPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const campaign = await getCampaignForPage(id);
  return (
    <>
      <CampaignHeader campaign={{ ...campaign, selected: false }} />
      <CampaignAiPanel campaignId={id} />
      <AnalyticsChart campaignId={id} />
    </>
  );
}
