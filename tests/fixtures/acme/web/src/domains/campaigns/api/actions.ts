"use server";

import { campaigns } from "@acme/api";

export async function archiveCampaign(id: string) {
  return { ...campaigns.getCampaign(id), status: "archived" as const };
}
