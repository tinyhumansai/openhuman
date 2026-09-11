
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

// MARK: - AXUIElement App Interaction

/// Find a running application by display name (exact, prefix, or contains match).
func findRunningApp(named appName: String) -> NSRunningApplication? {
    let apps = NSWorkspace.shared.runningApplications
    let lower = appName.lowercased()
    if let app = apps.first(where: { $0.localizedName?.lowercased() == lower }) { return app }
    if let app = apps.first(where: { $0.localizedName?.lowercased().hasPrefix(lower) ?? false }) { return app }
    return apps.first(where: { $0.localizedName?.lowercased().contains(lower) ?? false })
}

/// Walk the AX element tree depth-first. Visitor returns true to stop early.
func axWalk(_ element: AXUIElement, depth: Int = 0, maxDepth: Int = 12,
            visitor: (AXUIElement, String, String) -> Bool) -> Bool {
    if depth > maxDepth { return false }
    var roleRef: AnyObject?
    AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &roleRef)
    let role = (roleRef as? String) ?? ""
    var labelRef: AnyObject?
    AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &labelRef)
    var label = (labelRef as? String) ?? ""
    if label.isEmpty {
        var descRef: AnyObject?
        AXUIElementCopyAttributeValue(element, kAXDescriptionAttribute as CFString, &descRef)
        label = (descRef as? String) ?? ""
    }
    if visitor(element, role, label) { return true }
    var childrenRef: AnyObject?
    guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
          let children = childrenRef as? [AXUIElement] else { return false }
    for child in children {
        if axWalk(child, depth: depth + 1, maxDepth: maxDepth, visitor: visitor) { return true }
    }
    return false
}

/// List interactive UI elements in the named app.
func axListElements(appName: String, id: String?) -> [String: Any] {
    guard let app = findRunningApp(named: appName) else {
        return ["type": "ax_list", "id": id ?? "", "ok": false,
                "error": "App '\(appName)' not found or not running", "elements": [] as [Any]]
    }
    let axApp = AXUIElementCreateApplication(app.processIdentifier)
    let interactiveRoles: Set<String> = [
        "AXButton", "AXMenuItem", "AXMenuBarItem", "AXTextField", "AXTextArea",
        "AXCheckBox", "AXRadioButton", "AXSlider", "AXPopUpButton",
        "AXComboBox", "AXLink", "AXTab"
    ]
    var elements: [[String: Any]] = []
    axWalk(axApp, maxDepth: 10) { el, role, label in
        if interactiveRoles.contains(role) && !label.isEmpty {
            elements.append(["role": role, "label": label, "enabled": axEnabled(el)])
        }
        return false
    }
    return ["type": "ax_list", "id": id ?? "", "ok": true, "error": NSNull(), "elements": elements]
}

/// Read the AXEnabled attribute; default to `true` when the attribute is absent
/// (most static/text elements don't expose it, and we don't want to hide them).
func axEnabled(_ element: AXUIElement) -> Bool {
    var ref: AnyObject?
    if AXUIElementCopyAttributeValue(element, kAXEnabledAttribute as CFString, &ref) == .success,
       let b = ref as? Bool {
        return b
    }
    return true
}

/// Collect all AX elements whose label contains `label` (case-insensitive).
/// Returns matches sorted exact-first so "Play" beats "Playlist".
struct AXCandidate {
    var element: AXUIElement
    var label: String
    var exact: Bool
}

func axCollectMatches(_ root: AXUIElement, label: String, depth: Int = 0, maxDepth: Int = 12) -> [AXCandidate] {
    if depth > maxDepth { return [] }
    var roleRef: AnyObject?; AXUIElementCopyAttributeValue(root, kAXRoleAttribute as CFString, &roleRef)
    var titleRef: AnyObject?; AXUIElementCopyAttributeValue(root, kAXTitleAttribute as CFString, &titleRef)
    var elemLabel = (titleRef as? String) ?? ""
    if elemLabel.isEmpty { var d: AnyObject?; AXUIElementCopyAttributeValue(root, kAXDescriptionAttribute as CFString, &d); elemLabel = (d as? String) ?? "" }
    let lower = label.lowercased(); let elLower = elemLabel.lowercased()
    var results: [AXCandidate] = []
    if !elemLabel.isEmpty, elLower.contains(lower) {
        results.append(AXCandidate(element: root, label: elemLabel, exact: elLower == lower))
    }
    var childrenRef: AnyObject?
    guard AXUIElementCopyAttributeValue(root, kAXChildrenAttribute as CFString, &childrenRef) == .success,
          let children = childrenRef as? [AXUIElement] else { return results }
    for child in children { results += axCollectMatches(child, label: label, depth: depth+1, maxDepth: maxDepth) }
    return results
}

/// Press a UI element by label — exact match preferred over contains match.
func axPress(appName: String, label: String, id: String?) -> [String: Any] {
    guard let app = findRunningApp(named: appName) else {
        return ["type": "ax_press", "id": id ?? "", "ok": false,
                "error": "App '\(appName)' not found or not running"]
    }
    let axApp = AXUIElementCreateApplication(app.processIdentifier)
    var matches = axCollectMatches(axApp, label: label)
    // Exact matches first so "Play" beats "Playlist"
    matches.sort { $0.exact && !$1.exact }
    for match in matches {
        if AXUIElementPerformAction(match.element, kAXPressAction as CFString) == .success {
            return ["type": "ax_press", "id": id ?? "", "ok": true, "error": NSNull(),
                    "pressed": match.label]
        }
    }
    return ["type": "ax_press", "id": id ?? "", "ok": false,
            "error": "No pressable element matching '\(label)' found in '\(appName)' (\(matches.count) label match(es) not actionable)"]
}

/// Set the value of a text field by partial label match (or first text field if label is empty).
func axSetValue(appName: String, label: String, value: String, id: String?) -> [String: Any] {
    guard let app = findRunningApp(named: appName) else {
        return ["type": "ax_set_value", "id": id ?? "", "ok": false,
                "error": "App '\(appName)' not found or not running"]
    }
    let axApp = AXUIElementCreateApplication(app.processIdentifier)
    let textRoles: Set<String> = ["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"]
    let lower = label.lowercased()
    var setLabel = ""
    let found = axWalk(axApp) { element, role, elemLabel in
        guard textRoles.contains(role) else { return false }
        let matchLabel = lower.isEmpty || elemLabel.lowercased().contains(lower)
        guard matchLabel else { return false }
        if AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, value as CFTypeRef) == .success {
            setLabel = elemLabel
            return true
        }
        return false
    }
    if found {
        return ["type": "ax_set_value", "id": id ?? "", "ok": true, "error": NSNull(),
                "field": setLabel]
    }
    return ["type": "ax_set_value", "id": id ?? "", "ok": false,
            "error": "No text field matching '\(label)' found in '\(appName)'"]
}

// MARK: - Overlay Controller

final class OverlayController {
    private var panel: NSPanel?
    private var textField: NSTextField?
    private var hintField: NSTextField?
    private var hideWorkItem: DispatchWorkItem?

    func show(x: CGFloat, yTop: CGFloat, width: CGFloat, height: CGFloat, text: String, ttlMs: Int, tabHint: String) {
        let showTabHint = !tabHint.isEmpty
        // Detect current system appearance for contrast-appropriate colors.
        let isDark: Bool = {
            if #available(macOS 10.14, *) {
                return NSApp.effectiveAppearance
                    .bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            }
            return false
        }()
        let bgColor = isDark
            ? NSColor(white: 0.92, alpha: 0.82)   // light badge on dark background
            : NSColor(white: 0.10, alpha: 0.82)   // dark badge on light background
        let textColor = isDark
            ? NSColor(white: 0.08, alpha: 0.95)
            : NSColor(white: 1.0, alpha: 0.95)

        // Measure badge width from actual text metrics instead of char-count estimate.
        let font = NSFont.systemFont(ofSize: 13)
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let measured = (text as NSString).size(withAttributes: attrs)
        let hintPad: CGFloat = showTabHint ? 80 : 16
        let panelWidth = min(480, max(140, ceil(measured.width) + hintPad))
        let panelHeight: CGFloat = 28

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
            content.layer?.backgroundColor = bgColor.cgColor
            p.contentView = content

            let label = NSTextField(labelWithString: text)
            label.frame = NSRect(x: 8, y: 5, width: panelWidth - (showTabHint ? 62 : 16), height: 18)
            label.textColor = textColor
            label.font = font
            label.lineBreakMode = .byTruncatingTail
            content.addSubview(label)

            let hint = NSTextField(labelWithString: tabHint.isEmpty ? "Tab ↵" : tabHint)
            hint.frame = NSRect(x: panelWidth - 54, y: 5, width: 48, height: 18)
            hint.textColor = NSColor(white: isDark ? 0.35 : 0.65, alpha: 1.0)
            hint.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .regular)
            hint.alignment = .right
            hint.isHidden = !showTabHint
            content.addSubview(hint)

            panel = p
            textField = label
            hintField = hint
        }

        // Re-apply colors on every show so runtime appearance changes are reflected.
        panel?.contentView?.layer?.backgroundColor = bgColor.cgColor
        textField?.textColor = textColor
        hintField?.textColor = NSColor(white: isDark ? 0.35 : 0.65, alpha: 1.0)
        hintField?.isHidden = !showTabHint
        if showTabHint {
            hintField?.stringValue = tabHint
        }

        panel?.setFrame(NSRect(x: originX, y: originYCocoa, width: panelWidth, height: panelHeight), display: true)
        panel?.contentView?.frame = NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight)
        textField?.frame = NSRect(x: 8, y: 5, width: panelWidth - (showTabHint ? 62 : 16), height: 18)
        hintField?.frame = NSRect(x: panelWidth - 54, y: 5, width: 48, height: 18)
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

        case "ax_list":
            let appName = (payload["app_name"] as? String) ?? ""
            writeResponse(axListElements(appName: appName, id: id))

        case "ax_press":
            let appName = (payload["app_name"] as? String) ?? ""
            let label = (payload["label"] as? String) ?? ""
            writeResponse(axPress(appName: appName, label: label, id: id))

        case "ax_set_value":
            let appName = (payload["app_name"] as? String) ?? ""
            let label = (payload["label"] as? String) ?? ""
            let value = (payload["value"] as? String) ?? ""
            writeResponse(axSetValue(appName: appName, label: label, value: value, id: id))

        case "show":
            let x = CGFloat((payload["x"] as? NSNumber)?.doubleValue ?? 0)
            let y = CGFloat((payload["y"] as? NSNumber)?.doubleValue ?? 0)
            let w = CGFloat((payload["w"] as? NSNumber)?.doubleValue ?? 0)
            let h = CGFloat((payload["h"] as? NSNumber)?.doubleValue ?? 0)
            let text = (payload["text"] as? String) ?? ""
            let ttl = (payload["ttl_ms"] as? NSNumber)?.intValue ?? 900
            let tabHint: String = {
                if let s = payload["tab_hint"] as? String { return s }
                return "Tab ↵"
            }()
            DispatchQueue.main.async {
                controller.show(x: x, yTop: y, width: w, height: h, text: text, ttlMs: ttl, tabHint: tabHint)
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
