export const slugify = (value: string) => value.trim().toLowerCase().replace(/\s+/g, "-");

export const cn = (...classes: Array<string | false | undefined>) => classes.filter(Boolean).join(" ");
