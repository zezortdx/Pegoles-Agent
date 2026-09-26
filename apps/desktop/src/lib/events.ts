/** Friendly one-liner for a structured action (mirrors the Rust
 * `ComputerAction::describe`; never raw protocol JSON). Secret typing
 * shows no text (the wire already arrives redacted). */
export function describeAction(action: { type: string; [k: string]: unknown }): string {
  switch (action.type) {
    case "screenshot":
    case "observe_screen":
      return "Looking at the screen";
    case "get_display_info":
      return "Checking display size";
    case "move_pointer":
      return "Moving pointer";
    case "click":
      return "Clicking";
    case "double_click":
      return "Double-clicking";
    case "mouse_down":
      return "Pressing pointer";
    case "mouse_up":
      return "Releasing pointer";
    case "drag":
      return "Dragging";
    case "scroll":
      return "Scrolling";
    case "key_press":
      return typeof action.key === "string" ? `Pressing ${action.key}` : "Pressing key";
    case "key_chord":
      return Array.isArray(action.keys) ? `Pressing ${(action.keys as string[]).join("+")}` : "Pressing keys";
    case "type_text":
    case "type": {
      if (action.sensitive === true) return "Typing";
      const text = typeof action.text === "string" ? action.text : "";
      if (text.length === 0) return "Typing";
      const short = [...text].slice(0, 42).join("");
      return text.length > 42 ? `Typing “${short}…”` : `Typing “${short}”`;
    }
    case "wait":
      return "Waiting";
    case "open_url":
      return "Opening link";
    case "shell":
      return "Running a command";
    case "read_file":
      return "Reading a file";
    case "list_directory":
      return "Looking through a folder";
    case "write_file":
      return "Writing a file";
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
      return "Looked at the screen";
    case "get_display_info":
      return "Checked the display";
    case "move_pointer":
      return "Moved the pointer";
    case "click":
      return "Clicked";
    case "double_click":
      return "Double-clicked";
    case "mouse_down":
      return "Pressed the pointer";
    case "mouse_up":
      return "Released the pointer";
    case "drag":
      return "Dragged";
    case "scroll":
      return "Scrolled";
    case "key_press":
      return typeof action.key === "string" ? `Pressed ${action.key}` : "Pressed a key";
    case "key_chord":
      return Array.isArray(action.keys) ? `Pressed ${(action.keys as string[]).join("+")}` : "Pressed keys";
    case "type_text":
    case "type":
      return "Typed text";
    case "wait":
      return "Waited";
    case "open_url":
      return "Opened a website";
    case "shell":
      return "Ran a command";
    case "read_file":
      return "Read a file";
    case "list_directory":
      return "Looked through a folder";
    case "write_file":
      return "Wrote a file";
    default:
      return String(action.type).replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase());
  }
}
