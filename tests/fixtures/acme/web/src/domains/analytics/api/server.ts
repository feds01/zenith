import "server-only";
import { accounts } from "@acme/api";

export const getAccountAnalytics = (domain: string) => ({ account: accounts.getAccount(domain), visits: 0 });
