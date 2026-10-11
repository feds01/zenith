"use client";

// A stale copy of web/ inside a git worktree. .worktrees/ is gitignored, so
// zenith must never scan this file. If it did, mastra-protected would flag it.
import sharp from "sharp";
import { assistantAgent } from "../../../../web/src/mastra/agent";

export const stale = { sharp, assistantAgent };
