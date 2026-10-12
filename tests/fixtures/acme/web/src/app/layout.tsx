import type { ReactNode } from "react";
import { appConfig } from "config";
import "./styles.css";

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body data-app={appConfig.name}>{children}</body>
    </html>
  );
}
