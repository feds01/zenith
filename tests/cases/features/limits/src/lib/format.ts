import { log } from "./log";

export const format = (value: string) => {
  log(value);
  return value.trim();
};
