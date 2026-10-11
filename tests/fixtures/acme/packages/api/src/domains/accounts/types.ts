import { ACCOUNTS_ENDPOINT } from "./api";

export type Account = { id: string; domain: string };

export const ACCOUNT_URL = `${ACCOUNTS_ENDPOINT}/:id`;
