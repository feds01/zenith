import type { AnalyticsSummary } from "../types";

export async function fetchAnalytics(campaignId: string): Promise<AnalyticsSummary> {
  const response = await fetch(`/api/analytics/${campaignId}`);
  return response.json();
}
