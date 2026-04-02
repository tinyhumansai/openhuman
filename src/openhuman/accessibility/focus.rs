//! Accessibility focus queries and foreground app context.
//!
//! Primary path: unified Swift helper (native AX API, fast, persistent process).
//! Fallback: osascript subprocess (slower, but works without compiled helper).

use super::terminal::{is_terminal_app, is_text_role};
use super::text_util::{normalize_ax_value, parse_ax_number};
use super::types::{AppContext, ElementBounds, FocusedTextContext};

// ---------------------------------------------------------------------------
// Focus query: unified helper → osascript fallback
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn focused_text_context() -> Result<FocusedTextContext, String> {
    let ctx = focused_text_context_verbose()?;
    if let Some(err) = ctx.raw_error.as_ref() {
        return Err(format!(
            "focused text unavailable via accessibility api: {err}"
        ));
    }
    Ok(ctx)
}

/// Query the focused text element. Tries the unified Swift helper first (native AX, ~5-15ms),
/// falls back to osascript (~50-100ms) if the helper is unavailable.
#[cfg(target_os = "macos")]
pub fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    match focused_text_via_helper() {
        Ok(ctx) if ctx.raw_error.is_some() => {
            log::debug!(
                "[accessibility] helper returned raw_error={:?}, falling back to osascript",
                ctx.raw_error
            );
            focused_text_via_osascript()
        }
        Ok(ctx) => Ok(ctx),
        Err(helper_err) => {
            log::debug!(
                "[accessibility] helper focus query failed ({}), falling back to osascript",
                helper_err
            );
            focused_text_via_osascript()
        }
    }
}

/// Focus query via the unified Swift helper.
#[cfg(target_os = "macos")]
fn focused_text_via_helper() -> Result<FocusedTextContext, String> {
    let request = serde_json::json!({"type": "focus"});
    let resp = super::helper::helper_send_receive(&request)?;

    let app_name = resp
        .get("app_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let role = resp
        .get("role")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let text = resp
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let selected_text = resp
        .get("selected_text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let raw_error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let x = resp.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
    let y = resp.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
    let w = resp.get("w").and_then(|v| v.as_i64()).map(|v| v as i32);
    let h = resp.get("h").and_then(|v| v.as_i64()).map(|v| v as i32);

    Ok(FocusedTextContext {
        app_name,
        role,
        text,
        selected_text,
        raw_error,
        bounds: match (x, y, w, h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(ElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

/// Focus query via osascript (fallback when helper is unavailable).
#[cfg(target_os = "macos")]
fn focused_text_via_osascript() -> Result<FocusedTextContext, String> {
    let script = r##"
      tell application "System Events"
        set sep to character id 31
        set frontApp to first application process whose frontmost is true
        set appName to name of frontApp
        set roleValue to "unknown"
        set textValue to ""
        set selectedValue to ""
        set errValue to ""
        set posX to ""
        set posY to ""
        set sizeW to ""
        set sizeH to ""
        set targetRoles to {"AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"}

        try
          set value of attribute "AXEnhancedUserInterface" of frontApp to true
        end try

        try
          set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
          try
            set roleValue to value of attribute "AXRole" of focusedElement as text
          end try
          try
            set textValue to value of attribute "AXValue" of focusedElement as text
          end try
          try
            set p to value of attribute "AXPosition" of focusedElement
            set posX to item 1 of p as text
            set posY to item 2 of p as text
          end try
          try
            set s to value of attribute "AXSize" of focusedElement
            set sizeW to item 1 of s as text
            set sizeH to item 2 of s as text
          end try
          if textValue is "missing value" then set textValue to ""
          if textValue is "" then
            try
              set selectedValue to value of attribute "AXSelectedText" of focusedElement as text
            end try
            if selectedValue is "missing value" then set selectedValue to ""
            if selectedValue is not "" then set textValue to selectedValue
          end if
          if textValue is "" then
            try
              set textValue to value of attribute "AXTitle" of focusedElement as text
            end try
            if textValue is "missing value" then set textValue to ""
          end if
        on error errMsg number errNum
          set errValue to "ERROR:" & errNum & ":" & errMsg
        end try

        if textValue is "" then
          try
            set focusedWindow to value of attribute "AXFocusedWindow" of frontApp
            set childElems to entire contents of focusedWindow
            set staticPromptValue to ""
            set staticFallbackValue to ""
            repeat with childElem in childElems
              set childRole to ""
              set childValue to ""
              set childSelectedValue to ""
              try
                set childRole to value of attribute "AXRole" of childElem as text
              end try
              if childRole is in targetRoles then
                try
                  set childValue to value of attribute "AXValue" of childElem as text
                end try
                set childPosX to ""
                set childPosY to ""
                set childSizeW to ""
                set childSizeH to ""
                try
                  set cp to value of attribute "AXPosition" of childElem
                  set childPosX to item 1 of cp as text
                  set childPosY to item 2 of cp as text
                end try
                try
                  set cs to value of attribute "AXSize" of childElem
                  set childSizeW to item 1 of cs as text
                  set childSizeH to item 2 of cs as text
                end try
                if childValue is "missing value" then set childValue to ""
                if childValue is "" then
                  try
                    set childSelectedValue to value of attribute "AXSelectedText" of childElem as text
                  end try
                  if childSelectedValue is "missing value" then set childSelectedValue to ""
                  if childSelectedValue is not "" then set childValue to childSelectedValue
                end if
                if childValue is not "" then
                  set roleValue to childRole
                  set textValue to childValue
                  if childPosX is not "" then set posX to childPosX
                  if childPosY is not "" then set posY to childPosY
                  if childSizeW is not "" then set sizeW to childSizeW
                  if childSizeH is not "" then set sizeH to childSizeH
                  exit repeat
                end if
              end if
            end repeat
            if textValue is "" then
              repeat with childElem in childElems
                set childRole to ""
                set childValue to ""
                try
                  set childRole to value of attribute "AXRole" of childElem as text
                end try
                if childRole is "AXStaticText" then
                  try
                    set childValue to value of attribute "AXValue" of childElem as text
                  end try
                  if childValue is "missing value" then set childValue to ""
                  if childValue is not "" then
                    set staticFallbackValue to childValue
                    if childValue contains "$ " or childValue contains "# " or childValue contains "> " then
                      set staticPromptValue to childValue
                    end if
                  end if
                end if
              end repeat
              if staticPromptValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticPromptValue
              else if staticFallbackValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticFallbackValue
              end if
            end if
          on error errMsg2 number errNum2
            if errValue is "" then set errValue to "ERROR:" & errNum2 & ":" & errMsg2
          end try
        end if

        if textValue is "" and errValue is "" then
          set errValue to "ERROR:no_text_candidate_found"
        end if

        return appName & sep & roleValue & sep & textValue & sep & selectedValue & sep & errValue & sep & posX & sep & posY & sep & sizeW & sep & sizeH
      end tell
    "##;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("unable to query focused text context".to_string());
        }
        return Err(format!("unable to query focused text context: {stderr}"));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let mut segments = trimmed.splitn(9, '\u{1f}');
    let app_name = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let role = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut value = segments.next().map(normalize_ax_value).unwrap_or_default();
    let mut selected_text = segments
        .next()
        .map(normalize_ax_value)
        .filter(|s| !s.is_empty());
    let mut raw_error = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let pos_x = segments.next().and_then(parse_ax_number);
    let pos_y = segments.next().and_then(parse_ax_number);
    let size_w = segments.next().and_then(parse_ax_number);
    let size_h = segments.next().and_then(parse_ax_number);

    let allow_terminal_text_value =
        is_terminal_app(app_name.as_deref()) && !value.trim().is_empty();
    if !is_text_role(role.as_deref()) && !allow_terminal_text_value {
        value.clear();
        selected_text = None;
        if raw_error.is_none() {
            raw_error = Some("ERROR:no_text_candidate_found".to_string());
        }
    }

    Ok(FocusedTextContext {
        app_name,
        role,
        text: value,
        selected_text,
        raw_error,
        bounds: match (pos_x, pos_y, size_w, size_h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(ElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

#[cfg(not(target_os = "macos"))]
pub fn focused_text_context() -> Result<FocusedTextContext, String> {
    Err("accessibility focus queries are only supported on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    Err("accessibility focus queries are only supported on macOS".to_string())
}

// ---------------------------------------------------------------------------
// Focus target validation
// ---------------------------------------------------------------------------

/// Validate that the currently focused element still matches the target we generated the
/// suggestion for. Returns Ok if it matches or if validation is inconclusive.
#[cfg(target_os = "macos")]
pub fn validate_focused_target(
    expected_app: Option<&str>,
    expected_role: Option<&str>,
) -> Result<(), String> {
    if expected_app.is_none() {
        return Ok(());
    }
    let current = focused_text_context_verbose();
    match current {
        Ok(ctx) => {
            if let (Some(expected), Some(actual)) = (expected_app, ctx.app_name.as_deref()) {
                if expected.to_lowercase() != actual.to_lowercase() {
                    return Err(format!(
                        "focus shifted from '{}' to '{}', aborting insertion",
                        expected, actual
                    ));
                }
            }
            if let (Some(expected), Some(actual)) = (expected_role, ctx.role.as_deref()) {
                if expected != actual {
                    return Err(format!(
                        "target role changed from '{}' to '{}', aborting insertion",
                        expected, actual
                    ));
                }
            }
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Foreground app context (from screen_intelligence)
// ---------------------------------------------------------------------------

/// Parse the raw stdout from the AppleScript foreground-context query.
///
/// Expected format: 6 lines — app_name, window_title, x, y, width, height.
/// This is a pure function, fully testable without macOS.
pub fn parse_foreground_output(stdout: &str) -> Option<AppContext> {
    let mut lines = stdout.lines();
    let app = lines.next().map(|s| s.trim().to_string());
    let title = lines.next().map(|s| s.trim().to_string());
    let x = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let y = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let width = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let height = lines.next().and_then(|s| s.trim().parse::<i32>().ok());

    let bounds = match (x, y, width, height) {
        (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
            Some(ElementBounds {
                x,
                y,
                width,
                height,
            })
        }
        _ => None,
    };

    let app = app.filter(|s| !s.is_empty());
    let title = title.filter(|s| !s.is_empty());
    if app.is_none() && title.is_none() && bounds.is_none() {
        return None;
    }
    Some(AppContext {
        app_name: app,
        window_title: title,
        bounds,
    })
}

#[cfg(target_os = "macos")]
pub fn foreground_context() -> Option<AppContext> {
    let script = r#"
      tell application "System Events"
        set frontApp to name of first application process whose frontmost is true
        set frontWindow to ""
        set windowX to ""
        set windowY to ""
        set windowW to ""
        set windowH to ""
        try
          tell process frontApp
            if (count of windows) > 0 then
              set w to front window
              set frontWindow to name of w
              set p to position of w
              set s to size of w
              set windowX to item 1 of p as text
              set windowY to item 2 of p as text
              set windowW to item 1 of s as text
              set windowH to item 2 of s as text
            end if
          end tell
        end try
        return frontApp & "\n" & frontWindow & "\n" & windowX & "\n" & windowY & "\n" & windowW & "\n" & windowH
      end tell
    "#;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!(
            "[accessibility] osascript failed: status={:?} stderr={}",
            output.status.code(),
            stderr.trim()
        );
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let result = parse_foreground_output(&text);
    tracing::debug!(
        "[accessibility] foreground_context: app={:?} bounds_present={}",
        result.as_ref().and_then(|c| c.app_name.as_deref()),
        result.as_ref().map(|c| c.bounds.is_some()).unwrap_or(false)
    );
    result
}

#[cfg(not(target_os = "macos"))]
pub fn validate_focused_target(
    _expected_app: Option<&str>,
    _expected_role: Option<&str>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn foreground_context() -> Option<AppContext> {
    None
}
