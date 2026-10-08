import { accounts } from "@acme/api";
import { campaignUrl } from "@acme/utils";

export const portalApi = { title: "Customer portal", campaignUrl, getAccount: accounts.getAccount };
