import { afterEach, describe, expect, it } from "vitest";
import { activateLocale, t } from "./index";
import { ptBR, type CopyKey } from "./catalog";
import { describeAction } from "../lib/events";
import { capabilityFor } from "../lib/execution";
import { plural } from "../artifacts/format";
import { taskSections } from "../state/taskSections";
import { humanizeError, stopReason } from "../state/errors";

afterEach(() => activateLocale("en"));
describe("static product localization", () => {
  it("has matching placeholders for every English and pt-BR entry", () => {
    for (const [key, value] of Object.entries(ptBR)) {
      expect(value.trim().length, key).toBeGreaterThan(0);
      const placeholders = (text: string) => [...text.matchAll(/\{\w+\}/g)].map((match) => match[0]).sort();
      expect(placeholders(value), key).toEqual(placeholders(key));
      activateLocale("en"); expect(t(key as CopyKey)).toBe(key);
      activateLocale("pt-BR"); expect(t(key as CopyKey)).toBe(value);
    }
  });
  it("keeps stored error presentation reactive while diagnostic details stay original", () => {
    const error = humanizeError("ENOSPC: original disk detail", "computer");
    expect(error.title).toBe("Not enough disk space.");
    activateLocale("pt-BR");
    expect(error.title).toBe(ptBR["Not enough disk space."]);
    expect(error.detail).toBe("ENOSPC: original disk detail");
  });
  it("localizes presentation at lookup time and preserves wire identifiers and original content", () => {
    const original = "<svg onload=bad> Working {0}";
    activateLocale("pt-BR");
    expect(describeAction({ type: "click" })).toBe("Clicando");
    expect(capabilityFor("click")).toBe("Computer");
    expect(describeAction({ type: "type_text", text: original })).toContain(original);
    expect(stopReason(original)).toBe(original);
    expect(humanizeError(original, "task").detail).toBe(original);
    expect(plural(1, "action")).toBe("1 ação");
    expect(plural(2, "action")).toBe("2 ações");
    expect(t("Preview of {name}", { name: original })).toBe(`Prévia de ${original}`);
    expect(taskSections([{ id: "1", title: original, status: "running", created_at: "", updated_at: "" }])[0]).toMatchObject({ label: "Trabalhando", tasks: [{ title: original }] });
    activateLocale("en");
    expect(describeAction({ type: "click" })).toBe("Clicking");
    expect(plural(2, "action")).toBe("2 actions");
  });
});
