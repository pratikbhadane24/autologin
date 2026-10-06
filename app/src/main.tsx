import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/public-sans";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/base.css";

async function start() {
  // Dev-only: view the UI in a normal browser with fake data (?mock).
  if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
    const { installMockBackend } = await import("./dev/mockBackend");
    installMockBackend();
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

start();
