import sharp from "sharp";

export const resize = (input: Buffer) => sharp(input).resize(256).toBuffer();
