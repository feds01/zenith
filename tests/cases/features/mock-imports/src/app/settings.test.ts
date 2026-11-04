import { expect, jest, test } from "@jest/globals";

jest.mock(
  "../api/client",
  () => ({ fetchUser: jest.fn() }),
);

test("settings", () => {
  expect(true).toBe(true);
});
