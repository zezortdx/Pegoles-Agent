import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/shell.css";
import "./styles/composer.css";
import "./styles/home.css";
import "./styles/task.css";
import "./styles/computer.css";
import "./styles/pages.css";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("missing #root");
const root = ReactDOM.createRoot(rootEl);

const DESIGN_LAB_ROUTE = "#/dev/design";

// Dev-only labs. `import.meta.env.DEV` is statically `false` in production
// builds, so these branches and their dynamic imports are removed and the
// labs never ship (checked by src/dev/prodBundle.test.ts).
if (import.meta.env.DEV && window.location.hash.startsWith("#/dev/shell")) {
  // Simulated Core for visual QA of the real App (never shipped).
  import("./dev/shellLab").then(({ installShellLab }) => {
    installShellLab(window.location.hash);
    root.render(<React.StrictMode><App /></React.StrictMode>);
  }).catch((error: unknown) => root.render(<pre>Shell lab failed to load: {String(error)}</pre>));
} else if (import.meta.env.DEV && window.location.hash.startsWith("#/dev/experience")) {
  import("./dev/ExperienceLab").then(({ ExperienceLab }) => root.render(<React.StrictMode><ExperienceLab /></React.StrictMode>))
    .catch((error: unknown) => root.render(<pre>Experience lab failed to load: {String(error)}</pre>));
} else if (import.meta.env.DEV && (window.location.pathname === "/dev/design" || window.location.hash.startsWith(DESIGN_LAB_ROUTE))) {
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
  const onLab = window.location.hash.startsWith("#/dev/");
  const lab = window.location.hash;
  window.addEventListener("hashchange", () => {
    const next = window.location.hash;
    if (next.startsWith("#/dev/") !== onLab || (onLab && next !== lab)) window.location.reload();
  });
}
