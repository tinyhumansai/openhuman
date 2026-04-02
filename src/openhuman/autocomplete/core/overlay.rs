//! Overflow badge, overlay helper process, and macOS notifications.

#[cfg(target_os = "macos")]
use chrono::Utc;
#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::sync::Mutex as StdMutex;
#[cfg(target_os = "macos")]
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};

use super::text::truncate_tail;
use super::types::FocusedElementBounds;

#[cfg(target_os = "macos")]
static LAST_OVERFLOW_BADGE: Lazy<StdMutex<Option<(String, i64)>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg(target_os = "macos")]
struct OverlayHelperProcess {
    child: Child,
    stdin: ChildStdin,
}

#[cfg(target_os = "macos")]
static OVERLAY_HELPER_PROCESS: Lazy<StdMutex<Option<OverlayHelperProcess>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub(super) fn show_overflow_badge(
    kind: &str,
    suggestion: Option<&str>,
    error: Option<&str>,
    app_name: Option<&str>,
    anchor_bounds: Option<&FocusedElementBounds>,
) {
    #[cfg(target_os = "macos")]
    {
        const READY_THROTTLE_MS: i64 = 1_200;
        let now_ms = Utc::now().timestamp_millis();
        let signature = format!(
            "{}:{}:{}:{}",
            kind,
            app_name.unwrap_or_default(),
            suggestion.unwrap_or_default(),
            error.unwrap_or_default()
        );

        if let Ok(mut guard) = LAST_OVERFLOW_BADGE.lock() {
            if let Some((last_signature, last_ms)) = guard.as_ref() {
                if *last_signature == signature {
                    return;
                }
                if kind == "ready" && (now_ms - *last_ms) < READY_THROTTLE_MS {
                    return;
                }
            }
            *guard = Some((signature, now_ms));
        }

        if kind == "ready" {
            if let Some(suggestion_text) = suggestion {
                // Use anchor bounds if available, otherwise pass zero bounds
                // (the overlay helper will fall back to mouse cursor position).
                let fallback_bounds = FocusedElementBounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                };
                let bounds = anchor_bounds.unwrap_or(&fallback_bounds);
                if overlay_helper_show(bounds, suggestion_text).is_ok() {
                    return;
                }
            }
        } else {
            let _ = overlay_helper_hide();
        }

        let title = match kind {
            "ready" => "OpenHuman suggestion",
            "accepted" => "OpenHuman applied",
            "rejected" => "OpenHuman dismissed",
            "error" => "OpenHuman autocomplete error",
            _ => "OpenHuman autocomplete",
        };

        let mut body = match kind {
            "ready" => suggestion.unwrap_or_default().to_string(),
            "accepted" => format!("Inserted: {}", suggestion.unwrap_or_default()),
            "rejected" => "Suggestion dismissed.".to_string(),
            "error" => error.unwrap_or("Autocomplete failed").to_string(),
            _ => suggestion.unwrap_or_default().to_string(),
        };
        if body.trim().is_empty() {
            body = "No suggestion".to_string();
        }
        body = truncate_tail(&body, 140);

        let subtitle = app_name.unwrap_or_default().trim().to_string();
        let escaped_title = escape_osascript_text(title);
        let escaped_body = escape_osascript_text(&body);
        let escaped_subtitle = escape_osascript_text(&subtitle);

        let script = if subtitle.is_empty() {
            format!(
                r#"display notification "{}" with title "{}""#,
                escaped_body, escaped_title
            )
        } else {
            format!(
                r#"display notification "{}" with title "{}" subtitle "{}""#,
                escaped_body, escaped_title, escaped_subtitle
            )
        };

        std::thread::spawn(move || {
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
        });
    }
}

#[cfg(target_os = "macos")]
fn escape_osascript_text(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace(['\n', '\r'], " ")
}

#[cfg(target_os = "macos")]
fn overlay_helper_show(bounds: &FocusedElementBounds, text: &str) -> Result<(), String> {
    let message = serde_json::json!({
        "type": "show",
        "x": bounds.x,
        "y": bounds.y,
        "w": bounds.width,
        "h": bounds.height,
        "text": truncate_tail(text, 96),
        "ttl_ms": 1100
    })
    .to_string();
    overlay_helper_send_line(&message)
}

#[cfg(target_os = "macos")]
fn overlay_helper_hide() -> Result<(), String> {
    overlay_helper_send_line(r#"{"type":"hide"}"#)
}

#[cfg(target_os = "macos")]
pub(super) fn overlay_helper_quit() -> Result<(), String> {
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;
    if let Some(mut helper) = guard.take() {
        let _ = helper.stdin.write_all(br#"{"type":"quit"}"#);
        let _ = helper.stdin.write_all(b"\n");
        let _ = helper.stdin.flush();
        let _ = helper.child.kill();
        let _ = helper.child.wait();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn overlay_helper_send_line(line: &str) -> Result<(), String> {
    ensure_overlay_helper_running()?;
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;
    let Some(helper) = guard.as_mut() else {
        return Err("overlay helper unavailable".to_string());
    };
    helper
        .stdin
        .write_all(line.as_bytes())
        .and_then(|_| helper.stdin.write_all(b"\n"))
        .and_then(|_| helper.stdin.flush())
        .map_err(|e| format!("failed to write overlay helper stdin: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_overlay_helper_running() -> Result<(), String> {
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;

    if let Some(helper) = guard.as_mut() {
        if helper
            .child
            .try_wait()
            .map_err(|e| format!("failed to query overlay helper state: {e}"))?
            .is_none()
        {
            return Ok(());
        }
        *guard = None;
    }

    let binary_path = ensure_overlay_helper_binary()?;
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn overlay helper: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to capture overlay helper stdin".to_string())?;
    *guard = Some(OverlayHelperProcess { child, stdin });
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_overlay_helper_binary() -> Result<PathBuf, String> {
    let cache_dir = std::env::temp_dir().join("openhuman-autocomplete-overlay");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("failed to create cache dir: {e}"))?;
    let source_path = cache_dir.join("overlay_helper.swift");
    let binary_path = cache_dir.join("overlay_helper_bin");
    let source = overlay_helper_swift_source();

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, source)
            .map_err(|e| format!("failed to write overlay helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        let output = Command::new("xcrun")
            .arg("swiftc")
            .arg("-O")
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .arg("-O")
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&binary_path)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile overlay helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
    }

    Ok(binary_path)
}

#[cfg(target_os = "macos")]
fn overlay_helper_swift_source() -> &'static str {
    r#"import Cocoa
import Foundation

final class OverlayController {
    private var panel: NSPanel?
    private var textField: NSTextField?
    private var hideWorkItem: DispatchWorkItem?

    func show(x: CGFloat, yTop: CGFloat, width: CGFloat, height: CGFloat, text: String, ttlMs: Int) {
        let panelWidth = min(420, max(140, CGFloat(text.count) * 7 + 26))
        let panelHeight: CGFloat = 26

        // Multi-monitor: find the screen that contains the target bounds.
        // Fall back to the screen containing the mouse cursor, then main screen.
        let targetPoint = NSPoint(x: x + width / 2, y: yTop + height / 2)
        let screen: NSScreen? = {
            // Convert AX top-left coords to Cocoa bottom-left for screen matching
            let mainHeight = NSScreen.screens.first?.frame.height ?? 900
            let cocoaPoint = NSPoint(x: targetPoint.x, y: mainHeight - targetPoint.y)
            if let s = NSScreen.screens.first(where: { $0.frame.contains(cocoaPoint) }) {
                return s
            }
            // Fallback: screen containing mouse cursor
            let mouseLocation = NSEvent.mouseLocation
            if let s = NSScreen.screens.first(where: { $0.frame.contains(mouseLocation) }) {
                return s
            }
            return NSScreen.main ?? NSScreen.screens.first
        }()
        let screenFrame = screen?.frame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let screenHeight = screenFrame.height + screenFrame.origin.y

        var originX: CGFloat
        var originYCocoa: CGFloat

        // If bounds are valid, position relative to the text field
        if width > 0 && height > 0 {
            originX = x + max(8, min(width - panelWidth - 8, 28))
            let originYTop = yTop + max(5, min(height - panelHeight - 4, 10))
            originYCocoa = max(6, screenHeight - originYTop - panelHeight)
        } else {
            // Bounds unavailable — position near the mouse cursor
            let mouseLocation = NSEvent.mouseLocation
            originX = mouseLocation.x + 8
            originYCocoa = mouseLocation.y - panelHeight - 8
        }

        // Clamp to screen bounds to prevent off-screen positioning
        originX = max(screenFrame.origin.x + 4, min(originX, screenFrame.origin.x + screenFrame.width - panelWidth - 4))
        originYCocoa = max(screenFrame.origin.y + 4, min(originYCocoa, screenFrame.origin.y + screenFrame.height - panelHeight - 4))

        if panel == nil {
            let p = NSPanel(
                contentRect: NSRect(x: originX, y: originYCocoa, width: panelWidth, height: panelHeight),
                styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered,
                defer: false
            )
            p.level = .statusBar
            p.hasShadow = false
            p.isOpaque = false
            p.backgroundColor = .clear
            p.ignoresMouseEvents = true
            p.collectionBehavior = [.canJoinAllSpaces, .transient]

            let content = NSView(frame: NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight))
            content.wantsLayer = true
            content.layer?.cornerRadius = 6
            content.layer?.backgroundColor = NSColor(white: 0.08, alpha: 0.35).cgColor
            p.contentView = content

            let label = NSTextField(labelWithString: text)
            label.frame = NSRect(x: 8, y: 4, width: panelWidth - 12, height: 18)
            label.textColor = NSColor(white: 1.0, alpha: 0.46)
            label.font = NSFont.systemFont(ofSize: 13)
            label.lineBreakMode = .byTruncatingTail
            content.addSubview(label)

            panel = p
            textField = label
        }

        panel?.setFrame(NSRect(x: originX, y: originYCocoa, width: panelWidth, height: panelHeight), display: true)
        panel?.contentView?.frame = NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight)
        textField?.frame = NSRect(x: 8, y: 4, width: panelWidth - 12, height: 18)
        textField?.stringValue = text
        panel?.orderFrontRegardless()

        hideWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.hide()
        }
        hideWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(max(120, ttlMs)), execute: work)
    }

    func hide() {
        panel?.orderOut(nil)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = OverlayController()

DispatchQueue.global(qos: .utility).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = payload["type"] as? String else {
            continue
        }
        if kind == "show" {
            let x = CGFloat((payload["x"] as? NSNumber)?.doubleValue ?? 0)
            let y = CGFloat((payload["y"] as? NSNumber)?.doubleValue ?? 0)
            let w = CGFloat((payload["w"] as? NSNumber)?.doubleValue ?? 0)
            let h = CGFloat((payload["h"] as? NSNumber)?.doubleValue ?? 0)
            let text = (payload["text"] as? String) ?? ""
            let ttl = (payload["ttl_ms"] as? NSNumber)?.intValue ?? 900
            DispatchQueue.main.async {
                controller.show(x: x, yTop: y, width: w, height: h, text: text, ttlMs: ttl)
            }
        } else if kind == "hide" {
            DispatchQueue.main.async {
                controller.hide()
            }
        } else if kind == "quit" {
            DispatchQueue.main.async {
                controller.hide()
                NSApplication.shared.terminate(nil)
            }
            break
        }
    }
}

app.run()
"#
}
