"use client";

import { DASHBOARD_TABS } from "@/app/(dashboard)/tabs";
import { useDebounce } from "@/hooks/use-debounce";

export function Sidebar() {
  const tabs = useDebounce(DASHBOARD_TABS);
  return (
    <nav>
      {tabs.map((tab) => (
        <a key={tab} href={`/${tab}`}>
          {tab}
        </a>
      ))}
    </nav>
  );
}
