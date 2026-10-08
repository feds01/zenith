"use client";

import "./styles.css";

export function Button({ label, onClick }: { label: string; onClick?: () => void }) {
  return (
    <button className="acme-button" onClick={onClick}>
      {label}
    </button>
  );
}
