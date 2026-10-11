"use client";

import { Button, Card } from "@acme/ui";
import { formatStatus } from "@/lib/format";
import { archiveCampaign } from "../api/actions";
import type { CampaignRow } from "../types";

export function CampaignHeader({ campaign }: { campaign: CampaignRow }) {
  return (
    <header>
      <Card title={campaign.name} />
      <span>{formatStatus(campaign.status)}</span>
      <Button label="Archive" onClick={() => archiveCampaign(campaign.id)} />
    </header>
  );
}
