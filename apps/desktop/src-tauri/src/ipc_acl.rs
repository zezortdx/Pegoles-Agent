//! The IPC surface stays least-privilege: the release capability grants
//! exactly the commands the product UI invokes, debug-only commands are
//! granted only by the debug capability, and Tauri's ACL (enabled by the
//! build.rs app manifest) denies everything else, including event emits,
//! window control, remote origins and other windows.

use std::collections::BTreeSet;

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{DEBUG_COMMANDS, RELEASE_COMMANDS};

const RELEASE_CAPABILITY: &str = include_str!("../capabilities/default.json");
const DEBUG_CAPABILITY: &str = include_str!("../debug-capabilities/design-lab.json");
const FRONTEND_BRIDGE: &str = include_str!("../../src/lib/tauri.ts");

/// Core permissions the UI needs: listening to Core's events, and the
/// overlay title bar's drag region (drag, double-click zoom).
const CORE_PERMISSIONS: &[&str] = &[
    "core:event:allow-listen",
    "core:event:allow-unlisten",
    "core:window:allow-start-dragging",
    "core:window:allow-internal-toggle-maximize",
];

fn permission(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

fn permissions(capability: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let v: Value = serde_json::from_str(capability).unwrap();
    assert_eq!(
        v["windows"],
        serde_json::json!(["main"]),
        "main window only"
    );
    assert!(
        v.get("remote").is_none(),
        "no remote origin is ever granted"
    );
    let all: Vec<String> = serde_json::from_value(v["permissions"].clone()).unwrap();
    let count = all.len();
    let (core, app): (BTreeSet<String>, BTreeSet<String>) =
        all.into_iter().partition(|p| p.contains(':'));
    assert_eq!(core.len() + app.len(), count, "no duplicate permission");
    (core, app)
}

fn set(items: impl IntoIterator<Item = impl Into<String>>) -> BTreeSet<String> {
    items.into_iter().map(Into::into).collect()
}

/// Command names passed to `invoke(...)` in one section of tauri.ts.
fn invoked(section: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = section;
    while let Some(at) = rest.find("invoke") {
        rest = &rest[at + "invoke".len()..];
        let mut chars = rest.char_indices().peekable();
        // Skip a generic argument (`invoke<{ … }>(`), nested angles included.
        let mut depth = 0usize;
        let mut open = None;
        for (i, c) in chars.by_ref() {
            match c {
                '<' => depth += 1,
                '>' if depth > 0 => depth -= 1,
                '(' if depth == 0 => {
                    open = Some(i);
                    break;
                }
                _ if depth == 0 => break,
                _ => {}
            }
        }
        let Some(open) = open else { continue };
        let args = rest[open + 1..].trim_start();
        if let Some(literal) = args.strip_prefix('"') {
            let end = literal.find('"').unwrap();
            names.insert(literal[..end].to_string());
        }
    }
    names
}

fn bridge_sections() -> (&'static str, &'static str) {
    let api = FRONTEND_BRIDGE
        .split_once("export const api = {")
        .expect("tauri.ts exports `api`")
        .1;
    let (api, debug) = api
        .split_once("export const debugApi = {")
        .expect("tauri.ts exports `debugApi` after `api`");
    (api, debug)
}

#[test]
fn the_release_capability_grants_exactly_the_release_commands() {
    let (core, app) = permissions(RELEASE_CAPABILITY);
    assert_eq!(core, set(CORE_PERMISSIONS.iter().copied()));
    assert_eq!(app, set(RELEASE_COMMANDS.iter().map(|c| permission(c))));
    let v: Value = serde_json::from_str(RELEASE_CAPABILITY).unwrap();
    assert_eq!(v["identifier"], "default");
}

#[test]
fn the_debug_capability_grants_only_debug_commands() {
    let (core, app) = permissions(DEBUG_CAPABILITY);
    assert!(
        core.is_empty(),
        "no extra core or plugin permission: {core:?}"
    );
    assert_eq!(app, set(DEBUG_COMMANDS.iter().map(|c| permission(c))));
    let release = set(RELEASE_COMMANDS.iter().copied());
    let debug = set(DEBUG_COMMANDS.iter().copied());
    assert_eq!(release.len(), RELEASE_COMMANDS.len(), "no duplicate");
    assert_eq!(debug.len(), DEBUG_COMMANDS.len(), "no duplicate");
    assert!(
        release.is_disjoint(&debug),
        "{:?}",
        release.intersection(&debug)
    );
}

#[test]
fn the_release_commands_are_exactly_what_the_product_ui_invokes() {
    let (api, debug) = bridge_sections();
    assert_eq!(invoked(api), set(RELEASE_COMMANDS.iter().copied()));
    let debug_invoked = invoked(debug);
    assert!(!debug_invoked.is_empty());
    assert!(
        debug_invoked.is_subset(&set(DEBUG_COMMANDS.iter().copied())),
        "the Design Lab calls only debug commands: {debug_invoked:?}"
    );
    // Diagnostics the product never calls stay out of the release set.
    for name in [
        "guest_ping",
        "guest_info",
        "pump",
        "input_audit",
        "set_api_key",
    ] {
        assert!(!RELEASE_COMMANDS.contains(&name), "{name}");
    }
}

#[test]
fn the_invoke_parser_reads_generic_and_multiline_calls() {
    let found = invoked(
        r#"a: () => invoke<{ x: Array<number> }>("one"),
           b: () => invoke("two", { y }),
           c: () => invoke<{ z: number }>(
             "three",
             { w },
           ),
           import { invoke } from "x";"#,
    );
    assert_eq!(found, set(["one", "two", "three"]));
}

fn app() -> tauri::App<MockRuntime> {
    mock_builder()
        .invoke_handler(|invoke| {
            invoke.resolver.resolve("handled");
            true
        })
        .build(crate::context())
        .expect("the app's context builds")
}

fn window(app: &tauri::App<MockRuntime>, label: &str) -> WebviewWindow<MockRuntime> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::default())
        .build()
        .unwrap()
}

fn call(window: &WebviewWindow<MockRuntime>, cmd: &str, origin: &str) -> Result<(), String> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: origin.parse().unwrap(),
        body: InvokeBody::default(),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    get_ipc_response(window, request)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn denied(result: &Result<(), String>) -> bool {
    matches!(result, Err(e) if e.contains("not allowed"))
}

const LOCAL: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

#[test]
fn the_acl_allows_the_product_commands_and_denies_the_rest() {
    let app = app();
    let main = window(&app, crate::nav_guard::MAIN_WINDOW);

    for cmd in RELEASE_COMMANDS {
        assert_eq!(call(&main, cmd, LOCAL), Ok(()), "{cmd}");
    }
    // Registered only in debug builds, and not granted without the debug
    // capability (the release configuration).
    for cmd in DEBUG_COMMANDS {
        assert!(
            denied(&call(&main, cmd, LOCAL)),
            "{cmd} without the debug capability"
        );
    }
    for cmd in [
        "set_api_key",
        "not_a_command",
        "plugin:event|emit",
        "plugin:event|emit_to",
        "plugin:window|set_title",
        "plugin:window|close",
        "plugin:window|scale_factor",
        "plugin:webview|create_webview_window",
        "plugin:app|app_show",
        "plugin:path|resolve_directory",
    ] {
        assert!(denied(&call(&main, cmd, LOCAL)), "{cmd}");
    }
    // Granted core permissions pass the ACL (they may still fail on args).
    for cmd in ["plugin:event|listen", "plugin:event|unlisten"] {
        assert!(!denied(&call(&main, cmd, LOCAL)), "{cmd}");
    }
    // A remote page, even in the main window, gets nothing.
    for cmd in [
        "get_status",
        "set_provider",
        "enter_api_key",
        "capture_screen",
    ] {
        assert!(denied(&call(&main, cmd, "https://evil.example")), "{cmd}");
    }
    // Any other window gets nothing.
    let other = window(&app, "other");
    assert!(denied(&call(&other, "get_status", LOCAL)));

    // Debug builds add the Design Lab capability at startup.
    crate::grant_debug_commands(&app).unwrap();
    for cmd in DEBUG_COMMANDS {
        assert_eq!(
            call(&main, cmd, LOCAL),
            Ok(()),
            "{cmd} with the debug capability"
        );
    }
    assert!(denied(&call(&main, "plugin:event|emit", LOCAL)));
}
