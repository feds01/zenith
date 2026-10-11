import { z } from "zod";
import tpl from "../liquid/prompt.liquid";

export const researchTools = [{ name: "research", template: tpl, input: z.object({ query: z.string() }) }];
