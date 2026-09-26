//! Network containment for the main webview.
//!
//! The CSP stops fetch, XHR and beacons, and the navigation guard stops
//! top-level navigation, but WebKit starts some requests outside both:
//! `<link rel=preconnect>` / `dns-prefetch` open DNS, TCP and TLS to any
//! host, and WebRTC sends STUN/TURN over UDP. A compromised page could use
//! them to carry data out (task text, screenshots it can request over IPC).
//!
//! So, on macOS outside `tauri dev`, the main webview is created with a
//! WebKit content rule list that blocks every network (and `file:`) URL,
//! for subresources, preconnects and navigations alike: the list is
//! compiled first and the window is only built once it is attached (no
//! window, no page code, before it is in force; a list that fails to
//! compile quits the app). In every frame, before page code runs, the
//! WebRTC constructors are removed.

/// Block every load whose URL starts with a network scheme or `file:`
/// (one rule per scheme: WebKit's url-filter has no disjunction). The app
/// itself is served from `tauri://` and talks over `ipc://`, which stay
/// allowed.
pub const BLOCK_NETWORK_RULES: &str = r#"[
  {"trigger":{"url-filter":"^https?:"},"action":{"type":"block"}},
  {"trigger":{"url-filter":"^wss?:"},"action":{"type":"block"}},
  {"trigger":{"url-filter":"^ftps?:"},"action":{"type":"block"}},
  {"trigger":{"url-filter":"^file:"},"action":{"type":"block"}}
]"#;

/// Identifier of the compiled list in WebKit's rule list store.
pub const RULE_LIST_ID: &str = "pegoles-block-network";

/// Injected at document start into every frame: WebRTC is not used by the
/// UI, and its ICE traffic is not subject to CSP or content rules.
pub const DISABLE_WEBRTC: &str = r#"(() => {
  const names = [
    "RTCPeerConnection", "webkitRTCPeerConnection", "RTCDataChannel",
    "RTCIceCandidate", "RTCSessionDescription", "RTCRtpSender",
    "RTCRtpReceiver", "RTCRtpTransceiver", "RTCDtlsTransport",
    "RTCIceTransport", "RTCSctpTransport", "RTCCertificate",
    "RTCDTMFSender", "RTCPeerConnectionIceEvent", "RTCRtpScriptTransform"
  ];
  for (const name of names) {
    try {
      Object.defineProperty(globalThis, name, { value: undefined, writable: false, configurable: false });
    } catch (_) {}
  }
})();"#;

/// Whether this build contains the webview: every non-`tauri dev` build
/// (the dev server itself is plain HTTP).
pub fn contained() -> bool {
    !tauri::is_dev()
}

#[cfg(target_os = "macos")]
pub use native::with_network_blocked;

#[cfg(target_os = "macos")]
mod native {
    use std::cell::Cell;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKWebViewConfiguration};

    use super::{BLOCK_NETWORK_RULES, RULE_LIST_ID};

    /// Compile the blocking rule list, then call `build` with a webview
    /// configuration that already carries it. Runs on the main thread (app
    /// setup); `build` runs later on the main thread, from WebKit's
    /// completion. If the list cannot be compiled, or `build` fails, the app
    /// exits: it never shows an uncontained webview.
    pub fn with_network_blocked(
        app: tauri::AppHandle,
        build: impl FnOnce(Retained<WKWebViewConfiguration>) -> tauri::Result<()> + 'static,
    ) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("the webview must be set up on the main thread")?;
        // SAFETY: called on the main thread (checked above).
        let store = unsafe { WKContentRuleListStore::defaultStore(mtm) }
            .ok_or("WebKit has no content rule list store")?;
        let pending = Cell::new(Some(build));
        let completion = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
            let Some(build) = pending.take() else {
                return;
            };
            // SAFETY: WebKit passes either a valid list or nil, and either a
            // valid error or nil; `retain` takes a +1 reference we own.
            let list = unsafe { Retained::retain(list) };
            let result = match (list, MainThreadMarker::new()) {
                (Some(list), Some(mtm)) => {
                    // SAFETY: main thread; `list` is a compiled rule list.
                    let config = unsafe { WKWebViewConfiguration::new(mtm) };
                    unsafe { config.userContentController().addContentRuleList(&list) };
                    build(config).map_err(|e| e.to_string())
                }
                (None, _) => Err(if error.is_null() {
                    "the network rule list did not compile".to_string()
                } else {
                    // SAFETY: non-null NSError from WebKit.
                    unsafe { (*error).localizedDescription() }.to_string()
                }),
                (_, None) => Err("rule list completion ran off the main thread".to_string()),
            };
            if let Err(e) = result {
                eprintln!("pegoles: refusing to open the window without network containment: {e}");
                app.exit(1);
            }
        });
        // SAFETY: all arguments are valid; the completion block is retained
        // by WebKit until it runs.
        unsafe {
            store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                Some(&NSString::from_str(RULE_LIST_ID)),
                Some(&NSString::from_str(BLOCK_NETWORK_RULES)),
                Some(&completion),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_list_blocks_network_and_file_schemes_only() {
        let rules: serde_json::Value = serde_json::from_str(BLOCK_NETWORK_RULES).unwrap();
        let rules = rules.as_array().unwrap();
        let filters: Vec<&str> = rules
            .iter()
            .map(|r| {
                assert_eq!(r["action"]["type"], "block");
                r["trigger"]["url-filter"].as_str().unwrap()
            })
            .collect();
        assert_eq!(filters, ["^https?:", "^wss?:", "^ftps?:", "^file:"]);
        // WebKit's url-filter subset: no disjunction, no groups.
        assert!(filters.iter().all(|f| !f.contains('|') && !f.contains('(')));
        // Case-insensitive prefix semantics, as WebKit applies them.
        let blocked = |url: &str| {
            let url = url.to_ascii_lowercase();
            filters.iter().any(|f| {
                let scheme = f.trim_start_matches('^').trim_end_matches(':');
                let (base, optional_s) = match scheme.strip_suffix("s?") {
                    Some(b) => (b, true),
                    None => (scheme, false),
                };
                url.starts_with(&format!("{base}:"))
                    || (optional_s && url.starts_with(&format!("{base}s:")))
            })
        };
        for url in [
            "https://attacker.example/",
            "http://127.0.0.1:9/x",
            "wss://x.example/s",
            "HTTPS://CASE.example/",
            "file:///etc/hosts",
            "ftp://x.example/",
        ] {
            assert!(blocked(url), "{url}");
        }
        for url in [
            "tauri://localhost/index.html",
            "ipc://localhost/get_status",
            "data:image/png;base64,AA",
        ] {
            assert!(!blocked(url), "{url}");
        }
    }

    #[test]
    fn webrtc_constructors_are_removed_for_good() {
        for name in [
            "RTCPeerConnection",
            "webkitRTCPeerConnection",
            "RTCDataChannel",
        ] {
            assert!(DISABLE_WEBRTC.contains(&format!("\"{name}\"")), "{name}");
        }
        assert!(DISABLE_WEBRTC.contains("configurable: false"));
        assert!(DISABLE_WEBRTC.contains("writable: false"));
    }
}
