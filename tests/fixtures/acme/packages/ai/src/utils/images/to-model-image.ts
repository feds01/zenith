export async function toModelImage(input: Uint8Array): Promise<Uint8Array> {
  const { default: sharp } = await import("sharp");
  return sharp(input).resize(1024).png().toBuffer();
}
