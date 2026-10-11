import type { ReactNode } from "react";
import { Logo } from "@/components/ui/Logo";
import { Assistant } from "../assistant";
import { Sidebar } from "../nav/Sidebar";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <div>
      <Logo />
      <Sidebar />
      <main>{children}</main>
      <Assistant />
    </div>
  );
}
