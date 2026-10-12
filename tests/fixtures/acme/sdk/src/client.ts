import type { Campaign } from "@acme/api";
import { z } from "zod";
import { legacyCampaignShape } from "./does-not-exist";

export const campaignSchema = z.object({ id: z.string(), name: z.string() });

export type SdkCampaign = Campaign & { shape: typeof legacyCampaignShape };
