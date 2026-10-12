import type { ReactNode } from "react";
import Layout from "components/layout/Layout";

export default function DashboardLayout({ children }: { children: ReactNode }) {
  return <Layout>{children}</Layout>;
}
