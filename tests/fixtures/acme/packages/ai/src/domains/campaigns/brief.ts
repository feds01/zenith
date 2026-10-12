import { z } from "zod";

export const briefSchema = z.object({ goal: z.string(), audience: z.string() });
