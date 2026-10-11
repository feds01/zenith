import { Card } from "@acme/ui";
import { portalApi } from "@/lib/api";

export function HomeRoute() {
  return <Card title={portalApi.title} />;
}
