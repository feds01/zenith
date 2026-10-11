"use client";

import { readReportTemplate } from "@/lib/files";
import { loadChart } from "@/lib/lazy";
import { useAnalytics } from "../hooks/use-analytics";

export function AnalyticsChart({ campaignId }: { campaignId: string }) {
  const summary = useAnalytics(campaignId);
  return (
    <figure data-template={readReportTemplate("chart.html")} onMouseEnter={() => loadChart()}>
      {summary?.visits ?? 0}
    </figure>
  );
}
