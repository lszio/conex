import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "@/app";

// The stylesheet is not imported here: Tailwind's at-rules (@theme, @import
// "tailwindcss") are not valid input for Bun's CSS pipeline. `web/build.ts`
// runs the Tailwind CLI and writes /style.css, which index.html links.

const host = document.querySelector("#root");
if (!host) {
  throw new Error("missing #root");
}

createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
