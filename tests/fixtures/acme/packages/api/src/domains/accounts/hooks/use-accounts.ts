import { useQuery } from "@tanstack/react-query";
import { getAccount } from "../api";

export function useAccounts(domain: string) {
  return useQuery({ queryKey: ["account", domain], queryFn: () => getAccount(domain) });
}
