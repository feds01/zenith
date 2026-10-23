import sharp from "sharp";

import { resize } from "../../images/resize";

export const avatar = (input: Buffer) => resize(input);
export const crop = (input: Buffer) => sharp(input).extract({ left: 0, top: 0, width: 64, height: 64 });
