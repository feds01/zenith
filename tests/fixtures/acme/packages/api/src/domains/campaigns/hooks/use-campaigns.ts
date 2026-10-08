import { useQuery } from "@tanstack/react-query";
import { getCampaign } from "../api";

export function useCampaigns(id: string) {
  return useQuery({ queryKey: ["campaign", id], queryFn: () => getCampaign(id) });
}
