import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
// Tokens and page rules first, so component styles come after them.
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.tsx";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
