import { baseStrategy } from "../shared";

export function blueprintFor(channel: string) {
  return { ...baseStrategy, channel, sections: ["hero", "proof", "cta"] };
}
