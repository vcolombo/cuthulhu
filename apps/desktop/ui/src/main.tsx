// SPDX-License-Identifier: GPL-3.0-or-later
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter";
import "./tokens.css";
import { App } from "./App";

const container = document.getElementById("root");
if (!container) throw new Error("#root element missing");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
