import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import TrayMenu from "./TrayMenu";
import { applyTheme, getStoredTheme } from "./theme";
import "./theme.css";

// Apply the persisted theme before the first paint to avoid a flash.
applyTheme(getStoredTheme());

// Both windows load the same bundle; the window label picks the view.
const isTrayMenu = getCurrentWindow().label === "traymenu";

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    {isTrayMenu ? <TrayMenu /> : <App />}
  </React.StrictMode>,
);
