import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { createTauriNativeClient } from "./native";

// Composition root: the one place concrete services are created and handed to the UI.
const container = document.getElementById("root");
if (!container) {
  throw new Error("index.html is missing the #root element");
}

createRoot(container).render(
  <StrictMode>
    <App native={createTauriNativeClient()} />
  </StrictMode>,
);
