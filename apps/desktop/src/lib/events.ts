import { t } from "../i18n";
/** Friendly one-liner for a structured action (mirrors the Rust
 * `ComputerAction::describe`; never raw protocol JSON). Secret typing
 * shows no text (the wire already arrives redacted). */
export function describeAction(action: { type: string; [k: string]: unknown }): string {
  switch (action.type) {
    case "screenshot":
    case "observe_screen":
      return t("Looking at the screen");
    case "get_display_info":
      return t("Checking display size");
    case "move_pointer":
      return t("Moving pointer");
    case "click":
      return t("Clicking");
    case "double_click":
      return t("Double-clicking");
    case "mouse_down":
      return t("Pressing pointer");
    case "mouse_up":
      return t("Releasing pointer");
    case "drag":
      return t("Dragging");
    case "scroll":
      return t("Scrolling");
    case "key_press":
      return typeof action.key === "string" ? t("Pressing {0}", { "0": action.key }) : t("Pressing key");
    case "key_chord":
      return Array.isArray(action.keys) ? t("Pressing {0}", { "0": (action.keys as string[]).join("+") }) : t("Pressing keys");
    case "type_text":
    case "type": {
      if (action.sensitive === true) return t("Typing");
      const text = typeof action.text === "string" ? action.text : "";
      if (text.length === 0) return t("Typing");
      const short = [...text].slice(0, 42).join("");
      return text.length > 42 ? t("Typing “{0}…”", { "0": short }) : t("Typing “{0}”", { "0": short });
    }
    case "wait":
      return t("Waiting");
    case "open_url":
      return t("Opening link");
    case "shell":
      return t("Running a command");
    case "read_file":
      return t("Reading a file");
    case "list_directory":
      return t("Looking through a folder");
    case "write_file":
      return t("Writing a file");
    default:
      return String(action.type).replaceAll("_", " ");
  }
}

/** Finished actions, in the past tense. The target (path, host) is shown
 * separately, so these stay short. Typed text never appears. */
export function describePast(action: { type: string; [k: string]: unknown }): string {
  switch (action.type) {
    case "screenshot":
    case "observe_screen":
      return t("Looked at the screen");
    case "get_display_info":
      return t("Checked the display");
    case "move_pointer":
      return t("Moved the pointer");
    case "click":
      return t("Clicked");
    case "double_click":
      return t("Double-clicked");
    case "mouse_down":
      return t("Pressed the pointer");
    case "mouse_up":
      return t("Released the pointer");
    case "drag":
      return t("Dragged");
    case "scroll":
      return t("Scrolled");
    case "key_press":
      return typeof action.key === "string" ? t("Pressed {0}", { "0": action.key }) : t("Pressed a key");
    case "key_chord":
      return Array.isArray(action.keys) ? t("Pressed {0}", { "0": (action.keys as string[]).join("+") }) : t("Pressed keys");
    case "type_text":
    case "type":
      return t("Typed text");
    case "wait":
      return t("Waited");
    case "open_url":
      return t("Opened a website");
    case "shell":
      return t("Ran a command");
    case "read_file":
      return t("Read a file");
    case "list_directory":
      return t("Looked through a folder");
    case "write_file":
      return t("Wrote a file");
    default:
      return String(action.type).replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase());
  }
}
