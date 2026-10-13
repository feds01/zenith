"use client";

import { upload } from "../actions/upload";

export function UploadForm() {
  return <form action={upload} />;
}
