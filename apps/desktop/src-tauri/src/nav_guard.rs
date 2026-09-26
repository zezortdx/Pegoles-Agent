//! The main webview stays on the app's own origin. The CSP stops fetch,
//! XHR and image beacons, but not top-level navigation: without this
//! guard a script could send data out in a URL (`location.href =
//! "https://…?d=…"`) or load a remote page, chrome-less, in the app
//! window. New windows (`window.open`, `target=_blank`) are refused.

#[cfg(not(target_os = "macos"))]
use tauri::webview::NewWindowResponse;
use tauri::{Url, WebviewWindow, WebviewWindowBuilder};

/// Label of the only window (tauri.conf.json, `create: false`).
pub const MAIN_WINDOW: &str = "main";

/// Whether the webview may navigate to `url`: the bundled app
/// (`tauri://localhost`, `http://tauri.localhost` on Windows), or the dev
/// server origin when this is a `tauri dev` build. Everything else (remote
/// sites, other local ports, `file:`, `data:`, `blob:`, `about:`) is denied.
pub fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    let bundled = if cfg!(windows) {
        matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost")
    } else {
        url.scheme() == "tauri" && url.host_str() == Some("localhost")
    };
    bundled
        || dev_url.is_some_and(|dev| {
            url.scheme() == dev.scheme()
                && url.host_str() == dev.host_str()
                && url.port_or_known_default() == dev.port_or_known_default()
        })
}

/// Build the main window from its tauri.conf.json entry with the
/// navigation guard and new-window refusal attached (a window created from
/// config directly can't take either handler).
pub fn build_main_window(app: &tauri::App) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN_WINDOW)
        .cloned()
        .ok_or_else(|| {
            tauri::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "tauri.conf.json has no main window",
            ))
        })?;
    let dev_url = tauri::is_dev()
        .then(|| app.config().build.dev_url.clone())
        .flatten();
    let builder = WebviewWindowBuilder::from_config(app.handle(), &config)?
        .on_navigation(move |url| is_app_url(url, dev_url.as_ref()));
    // WKWebView with no new-window handler already refuses window.open().
    // Installing one on macOS adds nothing and routes window.open() through
    // wry code that unwraps the window's screen (nil while the window is off
    // every display), so it is only installed on the other platforms.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.on_new_window(|_, _| NewWindowResponse::Deny);
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        raw.parse().unwrap()
    }

    fn app_url(raw: &str) -> String {
        if cfg!(windows) {
            raw.replace("tauri://localhost", "http://tauri.localhost")
        } else {
            raw.to_string()
        }
    }

    #[test]
    fn the_bundled_app_may_navigate_within_itself() {
        for raw in [
            "tauri://localhost",
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "tauri://localhost/#main-content",
            "tauri://localhost/#/dev/shell/running",
        ] {
            assert!(is_app_url(&url(&app_url(raw)), None), "{raw}");
        }
    }

    #[test]
    fn everything_else_is_denied() {
        for raw in [
            "https://evil.example/?d=secret",
            "http://evil.example/",
            "tauri://evil.example/",
            "tauri://localhost.evil.example/",
            "tauri://localhost@evil.example/",
            "http://localhost:1420/",
            "http://localhost:1430/",
            "http://127.0.0.1/",
            "https://tauri.localhost.evil.example/",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "about:blank",
            "blob:tauri://localhost/8c1f",
            "javascript:alert(1)",
            "ipc://localhost/get_status",
            "asset://localhost/etc/passwd",
        ] {
            assert!(!is_app_url(&url(raw), None), "{raw}");
        }
    }

    #[test]
    fn the_main_window_is_built_by_the_app_so_it_carries_the_guard() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0]["label"], MAIN_WINDOW);
        assert_eq!(
            windows[0]["create"], false,
            "Tauri would build it unguarded"
        );
    }

    #[test]
    fn devurl_devcsp_and_the_vite_port_agree() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let dev_url = url(config["build"]["devUrl"].as_str().unwrap());
        assert_eq!(dev_url.as_str(), "http://localhost:1430/");
        let port = dev_url.port().unwrap();
        let vite = include_str!("../../vite.config.ts");
        assert!(
            vite.contains(&format!("port: {port},")),
            "vite.config.ts serves {port}"
        );
        assert!(vite.contains("strictPort: true"));
        let dev_csp = config["app"]["security"]["devCsp"].as_str().unwrap();
        let ports: Vec<&str> = dev_csp
            .split("localhost:")
            .skip(1)
            .map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next().unwrap())
            .collect();
        assert!(!ports.is_empty());
        assert!(ports.iter().all(|p| *p == port.to_string()), "{ports:?}");
        // The release CSP stays strict: no inline or eval script, no
        // network but IPC, no forms, never framed.
        let csp = config["app"]["security"]["csp"].as_str().unwrap();
        for directive in [
            "script-src 'self';",
            "connect-src ipc: http://ipc.localhost;",
            "object-src 'none'",
            "base-uri 'none'",
            "form-action 'none'",
            "frame-ancestors 'none'",
        ] {
            assert!(csp.contains(directive), "{directive}");
        }
        assert!(!csp.contains("unsafe-eval") && !csp.contains("localhost:"));
    }

    #[test]
    fn a_dev_build_also_allows_exactly_its_dev_server() {
        let dev = url("http://localhost:1430");
        assert!(is_app_url(&url("http://localhost:1430/"), Some(&dev)));
        assert!(is_app_url(
            &url("http://localhost:1430/#/dev/design"),
            Some(&dev)
        ));
        assert!(is_app_url(
            &url("http://localhost:1430/dev/design"),
            Some(&dev)
        ));
        for raw in [
            "http://localhost:1420/",
            "http://localhost/",
            "https://localhost:1430/",
            "http://127.0.0.1:1430/",
            "http://localhost.evil.example:1430/",
            "https://evil.example/?d=secret",
        ] {
            assert!(!is_app_url(&url(raw), Some(&dev)), "{raw}");
        }
    }
}
