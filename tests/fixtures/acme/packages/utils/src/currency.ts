import data from "./data.json";

export const currencySymbol = (code: string) => (data as Record<string, string>)[code] ?? code;
