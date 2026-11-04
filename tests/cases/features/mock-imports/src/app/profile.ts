import { fetchUser } from "../api/client";
import { isEnabled } from "../flags";

export async function profile(id: string) {
  return isEnabled("profiles") ? fetchUser(id) : null;
}
