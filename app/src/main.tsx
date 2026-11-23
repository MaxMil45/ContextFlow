import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { ErrorBoundary } from "./components/ui";
import "./styles.css";

// `npm run dev` and http://localhost:1420/?mock: the interface with sample data
// in an ordinary browser. Development only; the check is removed from builds.
if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
  const { installMockBackend } = await import("./dev/mockBackend");
  installMockBackend();
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
