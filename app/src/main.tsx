import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./styles.css";
import { Toast } from "./views/Toast";

// The small on-screen confirmation is the same page, opened in its own window.
const isToast = window.location.hash === "#toast";
if (isToast) document.documentElement.classList.add("toast-mode");

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isToast ? <Toast /> : <App />}</StrictMode>,
);
