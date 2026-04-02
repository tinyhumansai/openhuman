//! Unified Swift helper process: focus queries, paste, and overlay in one native binary.
//!
//! Replaces the separate osascript subprocess spawns and standalone overlay binary
//! with a single persistent Swift process communicating via stdin/stdout JSON.

#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "macos")]
use std::sync::Mutex as StdMutex;
#[cfg(target_os = "macos")]
use std::{
    fs,
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

#[cfg(target_os = "macos")]
struct UnifiedHelperProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[cfg(target_os = "macos")]
static UNIFIED_HELPER: Lazy<StdMutex<Option<UnifiedHelperProcess>>> =
    Lazy::new(|| StdMutex::new(None));

/// Send a JSON request and read a JSON response (one line each).
/// Used for `focus` and `paste` commands that produce a response.
#[cfg(target_os = "macos")]
pub(super) fn helper_send_receive(
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    ensure_helper_running()?;
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;
    let helper = guard
        .as_mut()
        .ok_or_else(|| "unified helper unavailable".to_string())?;

    // Write request
    let line = request.to_string();
    helper
        .stdin
        .write_all(line.as_bytes())
        .and_then(|_| helper.stdin.write_all(b"\n"))
        .and_then(|_| helper.stdin.flush())
        .map_err(|e| format!("failed to write to helper stdin: {e}"))?;

    // Read response (one line)
    let mut response_line = String::new();
    helper
        .stdout
        .read_line(&mut response_line)
        .map_err(|e| format!("failed to read helper stdout: {e}"))?;

    if response_line.trim().is_empty() {
        return Err("helper returned empty response".to_string());
    }

    serde_json::from_str(response_line.trim())
        .map_err(|e| format!("failed to parse helper response: {e}"))
}

/// Send a JSON request without waiting for a response.
/// Used for `show`, `hide`, and `quit` commands.
#[cfg(target_os = "macos")]
pub(super) fn helper_send_fire_and_forget(request: &serde_json::Value) -> Result<(), String> {
    ensure_helper_running()?;
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;
    let helper = guard
        .as_mut()
        .ok_or_else(|| "unified helper unavailable".to_string())?;

    let line = request.to_string();
    helper
        .stdin
        .write_all(line.as_bytes())
        .and_then(|_| helper.stdin.write_all(b"\n"))
        .and_then(|_| helper.stdin.flush())
        .map_err(|e| format!("failed to write to helper stdin: {e}"))?;
    Ok(())
}

/// Quit and clean up the helper process.
#[cfg(target_os = "macos")]
pub(super) fn helper_quit() -> Result<(), String> {
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;
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
fn ensure_helper_running() -> Result<(), String> {
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;

    if let Some(helper) = guard.as_mut() {
        if helper
            .child
            .try_wait()
            .map_err(|e| format!("failed to query helper state: {e}"))?
            .is_none()
        {
            return Ok(()); // Still running
        }
        log::debug!("[accessibility] unified helper exited, restarting");
        *guard = None;
    }

    let binary_path = ensure_helper_binary()?;
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn unified helper: {e}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to capture helper stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture helper stdout".to_string())?;

    *guard = Some(UnifiedHelperProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    });
    log::debug!("[accessibility] unified helper started");
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_helper_binary() -> Result<PathBuf, String> {
    let cache_dir = std::env::temp_dir().join("openhuman-accessibility-helper");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("failed to create cache dir: {e}"))?;
    let source_path = cache_dir.join("unified_helper.swift");
    let binary_path = cache_dir.join("unified_helper_bin");
    let source = unified_swift_source();

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, &source)
            .map_err(|e| format!("failed to write helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        log::debug!("[accessibility] compiling unified Swift helper");
        let output = Command::new("xcrun")
            .args([
                "swiftc",
                "-O",
                "-framework",
                "Cocoa",
                "-framework",
                "ApplicationServices",
            ])
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .args([
                        "-O",
                        "-framework",
                        "Cocoa",
                        "-framework",
                        "ApplicationServices",
                    ])
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&binary_path)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile unified helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
        log::debug!("[accessibility] unified helper compiled successfully");
    }

    Ok(binary_path)
}

#[cfg(target_os = "macos")]
fn unified_swift_source() -> String {
    r##"import Cocoa
import Foundation
import ApplicationServices

// MARK: - Thread-safe stdout writer

let stdoutLock = NSLock()

func writeResponse(_ dict: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: dict),
          let line = String(data: data, encoding: .utf8) else { return }
    stdoutLock.lock()
    print(line)
    fflush(stdout)
    stdoutLock.unlock()
}

// MARK: - Accessibility Focus Query

let textRoles: Set<String> = ["AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"]

// Apps that need AXEnhancedUserInterface to expose focused text elements properly.
let chromiumAppPatterns = ["chrom", "electron", "code", "slack", "discord", "brave", "edge", "opera", "vivaldi", "arc"]

func isChromiumApp(_ name: String) -> Bool {
    let lower = name.lowercased()
    return chromiumAppPatterns.contains(where: { lower.contains($0) })
}

func getAXStringAttr(_ element: AXUIElement, _ attr: String) -> String? {
    var value: AnyObject?
    let err = AXUIElementCopyAttributeValue(element, attr as CFString, &value)
    guard err == .success, let str = value as? String, str != "missing value" else { return nil }
    return str
}

func getAXPosition(_ element: AXUIElement) -> (x: Int, y: Int)? {
    var value: AnyObject?
    let err = AXUIElementCopyAttributeValue(element, kAXPositionAttribute as String as CFString, &value)
    guard err == .success else { return nil }
    var point = CGPoint.zero
    AXValueGetValue(value as! AXValue, .cgPoint, &point)
    return (Int(point.x), Int(point.y))
}

func getAXSize(_ element: AXUIElement) -> (w: Int, h: Int)? {
    var value: AnyObject?
    let err = AXUIElementCopyAttributeValue(element, kAXSizeAttribute as String as CFString, &value)
    guard err == .success else { return nil }
    var size = CGSize.zero
    AXValueGetValue(value as! AXValue, .cgSize, &size)
    return (Int(size.width), Int(size.height))
}

func scanChildrenForText(_ parent: AXUIElement, depth: Int = 0) -> (role: String, text: String, pos: (Int, Int)?, size: (Int, Int)?)? {
    if depth > 5 { return nil }
    var childrenRef: AnyObject?
    let err = AXUIElementCopyAttributeValue(parent, kAXChildrenAttribute as String as CFString, &childrenRef)
    guard err == .success, let children = childrenRef as? [AXUIElement] else { return nil }

    // First pass: look for text-role elements with content
    for child in children.prefix(200) {
        let role = getAXStringAttr(child, kAXRoleAttribute as String) ?? ""
        if textRoles.contains(role) {
            var text = getAXStringAttr(child, kAXValueAttribute as String) ?? ""
            if text.isEmpty {
                text = getAXStringAttr(child, kAXSelectedTextAttribute as String) ?? ""
            }
            if !text.isEmpty {
                return (role, text, getAXPosition(child), getAXSize(child))
            }
        }
    }

    // Second pass: look for AXStaticText with prompt patterns (terminal support)
    var staticFallback: (role: String, text: String, pos: (Int, Int)?, size: (Int, Int)?)?
    for child in children.prefix(200) {
        let role = getAXStringAttr(child, kAXRoleAttribute as String) ?? ""
        if role == "AXStaticText" {
            let text = getAXStringAttr(child, kAXValueAttribute as String) ?? ""
            if !text.isEmpty {
                if text.contains("$ ") || text.contains("# ") || text.contains("> ") {
                    return (role, text, getAXPosition(child), getAXSize(child))
                }
                if staticFallback == nil {
                    staticFallback = (role, text, getAXPosition(child), getAXSize(child))
                }
            }
        }
    }
    if let fb = staticFallback { return fb }

    // Recurse into children
    for child in children.prefix(50) {
        if let result = scanChildrenForText(child, depth: depth + 1) {
            return result
        }
    }
    return nil
}

func queryFocusedElement(id: String?) -> [String: Any] {
    var result: [String: Any] = [
        "type": "focus",
        "app_name": NSNull(),
        "role": NSNull(),
        "text": "",
        "selected_text": NSNull(),
        "x": NSNull(), "y": NSNull(), "w": NSNull(), "h": NSNull(),
        "error": NSNull(),
        "ax_trusted": AXIsProcessTrusted(),
    ]
    if let id = id { result["id"] = id }

    let systemWide = AXUIElementCreateSystemWide()

    // Get focused application
    var appRef: AnyObject?
    var appErr = AXUIElementCopyAttributeValue(systemWide, kAXFocusedApplicationAttribute as String as CFString, &appRef)
    guard appErr == .success, let appElement = appRef else {
        result["error"] = "ERROR:no_focused_application"
        return result
    }

    let appName = getAXStringAttr(appElement as! AXUIElement, kAXTitleAttribute as String) ?? "unknown"
    result["app_name"] = appName

    // Enable AXEnhancedUserInterface for Chromium apps
    if isChromiumApp(appName) {
        AXUIElementSetAttributeValue(appElement as! AXUIElement, "AXEnhancedUserInterface" as CFString, true as CFBoolean)
    }

    // Get focused element
    var focusedRef: AnyObject?
    let focusErr = AXUIElementCopyAttributeValue(appElement as! AXUIElement, kAXFocusedUIElementAttribute as String as CFString, &focusedRef)

    if focusErr == .success, let focused = focusedRef {
        let focusedElement = focused as! AXUIElement
        let role = getAXStringAttr(focusedElement, kAXRoleAttribute as String) ?? "unknown"
        result["role"] = role

        var text = getAXStringAttr(focusedElement, kAXValueAttribute as String) ?? ""
        let selectedText = getAXStringAttr(focusedElement, kAXSelectedTextAttribute as String)
        result["selected_text"] = selectedText ?? NSNull()

        if text.isEmpty, let sel = selectedText, !sel.isEmpty {
            text = sel
        }
        if text.isEmpty {
            text = getAXStringAttr(focusedElement, kAXTitleAttribute as String) ?? ""
        }

        if let pos = getAXPosition(focusedElement) {
            result["x"] = pos.x
            result["y"] = pos.y
        }
        if let size = getAXSize(focusedElement) {
            result["w"] = size.w
            result["h"] = size.h
        }

        // If we got text from a text-role element, we're done
        if !text.isEmpty && textRoles.contains(role) {
            result["text"] = text
            return result
        }

        // If role is not a text role, still return text if it looks terminal-like
        let terminalApps = ["terminal", "iterm", "wezterm", "warp", "alacritty", "kitty", "ghostty", "hyper", "rio"]
        let isTerminal = terminalApps.contains(where: { appName.lowercased().contains($0) })
        if isTerminal && !text.isEmpty {
            result["text"] = text
            return result
        }

        // Text is empty or not from a text role — scan window children
        if text.isEmpty || !textRoles.contains(role) {
            // Try scanning focused window's children
            var windowRef: AnyObject?
            let winErr = AXUIElementCopyAttributeValue(appElement as! AXUIElement, kAXFocusedWindowAttribute as String as CFString, &windowRef)
            if winErr == .success, let window = windowRef {
                if let found = scanChildrenForText(window as! AXUIElement) {
                    result["role"] = found.role
                    result["text"] = found.text
                    if let pos = found.pos { result["x"] = pos.0; result["y"] = pos.1 }
                    if let size = found.size { result["w"] = size.0; result["h"] = size.1 }
                    return result
                }
            }

            if text.isEmpty {
                result["error"] = "ERROR:no_text_candidate_found"
            } else {
                // Got text but from non-text role and not terminal — still return it
                result["text"] = text
            }
        } else {
            result["text"] = text
        }
    } else {
        // No focused element found — try window scanning
        var windowRef: AnyObject?
        let winErr = AXUIElementCopyAttributeValue(appElement as! AXUIElement, kAXFocusedWindowAttribute as String as CFString, &windowRef)
        if winErr == .success, let window = windowRef {
            if let found = scanChildrenForText(window as! AXUIElement) {
                result["role"] = found.role
                result["text"] = found.text
                if let pos = found.pos { result["x"] = pos.0; result["y"] = pos.1 }
                if let size = found.size { result["w"] = size.0; result["h"] = size.1 }
                return result
            }
        }
        result["error"] = "ERROR:-1728:no_focused_element"
    }

    return result
}

// MARK: - Paste Helper

func pasteText(id: String?, text: String) -> [String: Any] {
    var result: [String: Any] = ["type": "paste", "ok": true, "error": NSNull()]
    if let id = id { result["id"] = id }

    let pb = NSPasteboard.general
    let originalContents = pb.string(forType: .string)

    // Set clipboard to new text
    pb.clearContents()
    pb.setString(text, forType: .string)

    // Brief delay for clipboard to settle
    usleep(10_000) // 10ms

    // Simulate Cmd+V via CGEvent
    guard let keyDown = CGEvent(keyboardEventSource: nil, virtualKey: 0x09, keyDown: true),
          let keyUp = CGEvent(keyboardEventSource: nil, virtualKey: 0x09, keyDown: false) else {
        result["ok"] = false
        result["error"] = "failed to create CGEvent"
        return result
    }
    keyDown.flags = .maskCommand
    keyUp.flags = .maskCommand
    keyDown.post(tap: .cgSessionEventTap)
    usleep(8_000) // 8ms between key down/up
    keyUp.post(tap: .cgSessionEventTap)

    // Restore clipboard after delay
    if let original = originalContents {
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + .milliseconds(250)) {
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(original, forType: .string)
        }
    }

    return result
}

// MARK: - Overlay Controller

final class OverlayController {
    private var panel: NSPanel?
    private var textField: NSTextField?
    private var hideWorkItem: DispatchWorkItem?

    func show(x: CGFloat, yTop: CGFloat, width: CGFloat, height: CGFloat, text: String, ttlMs: Int) {
        let panelWidth = min(420, max(140, CGFloat(text.count) * 7 + 26))
        let panelHeight: CGFloat = 26

        // Multi-monitor: find the screen containing the target or mouse cursor.
        let screen: NSScreen? = {
            let mainHeight = NSScreen.screens.first?.frame.height ?? 900
            if width > 0 && height > 0 {
                let cocoaPoint = NSPoint(x: x + width / 2, y: mainHeight - (yTop + height / 2))
                if let s = NSScreen.screens.first(where: { $0.frame.contains(cocoaPoint) }) {
                    return s
                }
            }
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

        if width > 0 && height > 0 {
            originX = x + max(8, min(width - panelWidth - 8, 28))
            let originYTop = yTop + max(5, min(height - panelHeight - 4, 10))
            originYCocoa = max(6, screenHeight - originYTop - panelHeight)
        } else {
            let mouseLocation = NSEvent.mouseLocation
            originX = mouseLocation.x + 8
            originYCocoa = mouseLocation.y - panelHeight - 8
        }

        // Clamp to screen bounds
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

// MARK: - Main Entry Point

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = OverlayController()

DispatchQueue.global(qos: .userInitiated).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = payload["type"] as? String else {
            continue
        }
        let id = payload["id"] as? String

        switch kind {
        case "focus":
            let response = queryFocusedElement(id: id)
            writeResponse(response)

        case "paste":
            let text = (payload["text"] as? String) ?? ""
            let response = pasteText(id: id, text: text)
            writeResponse(response)

        case "show":
            let x = CGFloat((payload["x"] as? NSNumber)?.doubleValue ?? 0)
            let y = CGFloat((payload["y"] as? NSNumber)?.doubleValue ?? 0)
            let w = CGFloat((payload["w"] as? NSNumber)?.doubleValue ?? 0)
            let h = CGFloat((payload["h"] as? NSNumber)?.doubleValue ?? 0)
            let text = (payload["text"] as? String) ?? ""
            let ttl = (payload["ttl_ms"] as? NSNumber)?.intValue ?? 900
            DispatchQueue.main.async {
                controller.show(x: x, yTop: y, width: w, height: h, text: text, ttlMs: ttl)
            }

        case "hide":
            DispatchQueue.main.async {
                controller.hide()
            }

        case "quit":
            DispatchQueue.main.async {
                controller.hide()
                NSApplication.shared.terminate(nil)
            }
            return

        default:
            break
        }
    }
}

app.run()
"##.to_string()
}
