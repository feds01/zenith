import { campaigns } from "@acme/api";
import { toModelImage } from "../../utils/images/to-model-image";

export async function summarizeCampaign(id: string, cover: Uint8Array) {
  const campaign = campaigns.getCampaign(id);
  return { campaign, cover: await toModelImage(cover) };
}
