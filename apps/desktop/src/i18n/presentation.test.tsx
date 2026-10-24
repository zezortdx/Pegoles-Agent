import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { Details } from "../computer/ComputerInfo";
import { computerModel } from "../state/computerModel";
import { activateLocale } from "./index";

afterEach(() => { cleanup(); activateLocale("en"); });
it("localizes the computer configuration label without changing Core's reported specifications", () => {
  activateLocale("pt-BR");
  const model = computerModel({ connected: false, native: false, status: null, events: [] });
  render(<Details model={{ ...model, specs: ["Debian 13 · arm64 · 2 CPU"] }} status={null} error={null}
    loadBootLog={async () => ({ available: false, total_lines: 0, tail: [] })} />);
  expect(screen.getByText("Configuração")).toBeTruthy();
  expect(screen.getByText("Debian 13 · arm64 · 2 CPU")).toBeTruthy();
});
