import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("missing #root");
const root = ReactDOM.createRoot(rootEl);

const DESIGN_LAB_ROUTE = "#/dev/design";

// Dev-only Flux Glass design lab. `import.meta.env.DEV` is statically
// `false` in production builds, so this branch and its dynamic import are
// removed and the lab never ships (checked by src/dev/prodBundle.test.ts).
if (import.meta.env.DEV && (window.location.pathname === "/dev/design" || window.location.hash.startsWith(DESIGN_LAB_ROUTE))) {
  import("./dev/DesignLab")
    .then(({ DesignLab }) => {
      root.render(
        <React.StrictMode>
          <DesignLab />
        </React.StrictMode>,
      );
    })
    .catch((error: unknown) => {
      root.render(<pre>Design lab failed to load: {String(error)}</pre>);
    });
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

if (import.meta.env.DEV) {
  const onLab = window.location.hash.startsWith(DESIGN_LAB_ROUTE);
  window.addEventListener("hashchange", () => {
    if (window.location.hash.startsWith(DESIGN_LAB_ROUTE) !== onLab) window.location.reload();
  });
}
