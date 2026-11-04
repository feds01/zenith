import { expect, test, vi } from "vitest";
import { fetchUser } from "../api/client";
import { profile } from "./profile";

vi.mock("../api/client", () => ({ fetchUser: vi.fn() }));
vi.mock(import("../flags"), () => ({ isEnabled: () => true }));

test("profile fetches the user", async () => {
  await profile("1");
  expect(fetchUser).toHaveBeenCalledWith("1");
});
