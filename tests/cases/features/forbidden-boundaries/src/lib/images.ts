import sharp from "sharp";

export async function thumbnail(file: Blob) {
  return sharp(Buffer.from(await file.arrayBuffer())).resize(128).toBuffer();
}
