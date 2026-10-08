import get from "lodash/get";

export const pick = (o: object, k: string) => get(o, k);
