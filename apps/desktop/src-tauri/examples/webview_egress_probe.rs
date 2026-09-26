//! Proof that the main webview cannot reach the network (macOS).
//!
//! Loads the real app page (bundled frontend, release CSP) in a hidden
//! window and runs what a script injected into it could try: fetch, an
//! image beacon, `<link rel=preconnect>`, WebRTC (also from a fresh
//! iframe), and a top-level navigation, all aimed at a local TCP listener
//! and UDP socket that count what reaches them.
//!
//! ```sh
//! pnpm --filter @pegoles/desktop build
//! cargo run -p pegoles-desktop --example webview_egress_probe -- control    # no containment
//! cargo run -p pegoles-desktop --example webview_egress_probe -- contained  # product setup
//! ```
//! `control` must show hits (so the probes work); `contained` must show
//! none. Exit 0 when the observed result matches the mode.

use std::io::Read;
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pegoles_desktop::nav_guard::{is_app_url, MAIN_WINDOW};
use pegoles_desktop::webview_egress;
use tauri::{WebviewUrl, WebviewWindowBuilder};

/// The real UI calls `get_status` as soon as it starts: answering it here
/// proves the page loaded and IPC (`ipc://`) works under the rule list.
static IPC_CALLS: AtomicUsize = AtomicUsize::new(0);
/// Navigations the page script attempted (seen by the guard).
static NAV_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

#[tauri::command]
fn get_status() -> Result<(), String> {
    IPC_CALLS.fetch_add(1, Ordering::SeqCst);
    Err("probe".into())
}

fn probe_script(tcp: u16, udp: u16) -> String {
    format!(
        r#"
window.addEventListener("DOMContentLoaded", () => {{
  const base = "http://127.0.0.1:{tcp}";
  try {{ fetch(base + "/fetch").catch(() => {{}}); }} catch (_) {{}}
  try {{ new Image().src = base + "/img"; }} catch (_) {{}}
  for (const rel of ["preconnect", "dns-prefetch", "prefetch"]) {{
    try {{
      const link = document.createElement("link");
      link.rel = rel;
      link.href = base + "/" + rel;
      document.head.appendChild(link);
    }} catch (_) {{}}
  }}
  const rtc = (Ctor) => {{
    try {{
      const pc = new Ctor({{ iceServers: [{{ urls: "stun:127.0.0.1:{udp}" }}] }});
      pc.createDataChannel("x");
      pc.createOffer().then((o) => pc.setLocalDescription(o)).catch(() => {{}});
    }} catch (_) {{}}
  }};
  if (typeof RTCPeerConnection === "function") rtc(RTCPeerConnection);
  try {{
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    const Ctor = frame.contentWindow && frame.contentWindow.RTCPeerConnection;
    if (typeof Ctor === "function") rtc(Ctor);
  }} catch (_) {{}}
  setTimeout(() => {{ try {{ location.href = base + "/navigate"; }} catch (_) {{}} }}, 2500);
}});
"#
    )
}

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "contained".into());
    let contained = match mode.as_str() {
        "contained" => true,
        "control" => false,
        other => {
            eprintln!("usage: webview_egress_probe control|contained (got {other})");
            std::process::exit(2);
        }
    };
    let tcp = TcpListener::bind("127.0.0.1:0").expect("tcp listener");
    let tcp_port = tcp.local_addr().unwrap().port();
    let udp = UdpSocket::bind("127.0.0.1:0").expect("udp socket");
    let udp_port = udp.local_addr().unwrap().port();
    let tcp_hits = Arc::new(Mutex::new(Vec::<String>::new()));
    let udp_hits = Arc::new(AtomicUsize::new(0));
    {
        let hits = tcp_hits.clone();
        std::thread::spawn(move || {
            for stream in tcp.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buf = [0u8; 512];
                let n = stream.read(&mut buf).unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or("(connect only)")
                    .to_string();
                hits.lock().unwrap().push(line);
            }
        });
    }
    {
        let hits = udp_hits.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while udp.recv_from(&mut buf).is_ok() {
                hits.fetch_add(1, Ordering::SeqCst);
            }
        });
    }
    let script = probe_script(tcp_port, udp_port);
    let report = (tcp_hits.clone(), udp_hits.clone());
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_status])
        .setup(move |app| {
            let handle = app.handle().clone();
            let make = move |builder: WebviewWindowBuilder<'_, tauri::Wry, tauri::AppHandle>| {
                builder
                    .visible(false)
                    .initialization_script(script.clone())
                    .build()
                    .map(|_| ())
            };
            let url = WebviewUrl::App("index.html".into());
            if contained {
                let h = handle.clone();
                webview_egress::with_network_blocked(handle.clone(), move |config| {
                    make(
                        WebviewWindowBuilder::new(&h, MAIN_WINDOW, url)
                            .on_navigation(|u| {
                                let ok = is_app_url(u, None);
                                if !ok {
                                    NAV_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
                                }
                                ok
                            })
                            .initialization_script_for_all_frames(webview_egress::DISABLE_WEBRTC)
                            .with_webview_configuration(config),
                    )
                })
                .map_err(std::io::Error::other)?;
            } else {
                make(WebviewWindowBuilder::new(&handle, MAIN_WINDOW, url))?;
            }
            let exit = handle.clone();
            let (tcp_hits, udp_hits) = report.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(8));
                let tcp = tcp_hits.lock().unwrap().clone();
                let udp = udp_hits.load(Ordering::SeqCst);
                let ipc = IPC_CALLS.load(Ordering::SeqCst);
                let navs = NAV_ATTEMPTS.load(Ordering::SeqCst);
                println!(
                    "mode={mode} tcp_hits={} udp_packets={udp} ipc_calls={ipc} blocked_navigations={navs}",
                    tcp.len()
                );
                for h in &tcp {
                    println!("  tcp: {h}");
                }
                let reached = !tcp.is_empty() || udp > 0;
                // Contained: nothing reached the listeners, yet the page ran
                // (its IPC call arrived and its navigation attempt was seen).
                let ok = if contained { !reached && ipc > 0 && navs > 0 } else { reached };
                println!(
                    "{}",
                    if ok {
                        "EGRESS PROBE OK"
                    } else {
                        "EGRESS PROBE FAIL"
                    }
                );
                exit.exit(if ok { 0 } else { 2 });
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri app");
}
