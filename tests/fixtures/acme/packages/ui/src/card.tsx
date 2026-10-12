import { cn } from "@acme/utils";

export function Card({ title, muted }: { title: string; muted?: boolean }) {
  return <section className={cn("acme-card", muted && "acme-card--muted")}>{title}</section>;
}
