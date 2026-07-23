// SPDX-License-Identifier: GPL-3.0-or-later
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./tokens.css";

// Empty canvas shell — panels, toolbars and the Renderer wiring land in Task 13.
function App() {
  return <div className="workspace" style={{ width: "100%", height: "100%" }} />;
}

const container = document.getElementById("root");
if (!container) throw new Error("#root element missing");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
