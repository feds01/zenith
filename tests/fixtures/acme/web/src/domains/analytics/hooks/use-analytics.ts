import { useEffect, useState } from "react";
import { fetchAnalytics } from "../api/client";
import type { AnalyticsSummary } from "../types";

export function useAnalytics(campaignId: string) {
  const [summary, setSummary] = useState<AnalyticsSummary>();
  useEffect(() => void fetchAnalytics(campaignId).then(setSummary), [campaignId]);
  return summary;
}
