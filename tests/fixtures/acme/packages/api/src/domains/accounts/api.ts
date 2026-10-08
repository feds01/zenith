import { slugify } from "@acme/utils";
import type { Account } from "./types";

export const ACCOUNTS_ENDPOINT = "/accounts";

export function getAccount(domain: string): Account {
  return { id: slugify(domain), domain };
}
