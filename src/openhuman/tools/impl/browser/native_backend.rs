use super::BrowserAction;
use anyhow::{Context, Result};
use base64::Engine;
use fantoccini::actions::{InputSource, MouseActions, PointerAction};
use fantoccini::key::Key;
use fantoccini::{Client, ClientBuilder, Locator};
use serde_json::{json, Map, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Default)]
pub struct NativeBrowserState {
    client: Option<Client>,
}

impl NativeBrowserState {
    pub fn is_available(_headless: bool, webdriver_url: &str, _chrome_path: Option<&str>) -> bool {
        webdriver_endpoint_reachable(webdriver_url, Duration::from_millis(500))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn execute_action(
        &mut self,
        action: BrowserAction,
        headless: bool,
        webdriver_url: &str,
        chrome_path: Option<&str>,
    ) -> Result<Value> {
        match action {
            BrowserAction::Open { url } => {
                self.ensure_session(headless, webdriver_url, chrome_path)
                    .await?;
                let client = self.active_client()?;
                client
                    .goto(&url)
                    .await
                    .with_context(|| format!("Failed to open URL: {url}"))?;
                let current_url = client
                    .current_url()
                    .await
                    .context("Failed to read current URL after navigation")?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "open",
                    "url": current_url.as_str(),
                }))
            }
            BrowserAction::Snapshot {
                interactive_only,
                compact,
                depth,
            } => {
                let client = self.active_client()?;
                let snapshot = client
                    .execute(
                        &snapshot_script(interactive_only, compact, depth.map(i64::from)),
                        vec![],
                    )
                    .await
                    .context("Failed to evaluate snapshot script")?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "snapshot",
                    "data": snapshot,
                }))
            }
            BrowserAction::Click { selector } => {
                let client = self.active_client()?;
                find_element(client, &selector).await?.click().await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "click",
                    "selector": selector,
                }))
            }
            BrowserAction::Fill { selector, value } => {
                let client = self.active_client()?;
                let element = find_element(client, &selector).await?;
                let _ = element.clear().await;
                element.send_keys(&value).await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "fill",
                    "selector": selector,
                }))
            }
            BrowserAction::Type { selector, text } => {
                let client = self.active_client()?;
                find_element(client, &selector)
                    .await?
                    .send_keys(&text)
                    .await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "type",
                    "selector": selector,
                    "typed": text.len(),
                }))
            }
            BrowserAction::GetText { selector } => {
                let client = self.active_client()?;
                let text = find_element(client, &selector).await?.text().await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "get_text",
                    "selector": selector,
                    "text": text,
                }))
            }
            BrowserAction::GetTitle => {
                let client = self.active_client()?;
                let title = client.title().await.context("Failed to read page title")?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "get_title",
                    "title": title,
                }))
            }
            BrowserAction::GetUrl => {
                let client = self.active_client()?;
                let url = client
                    .current_url()
                    .await
                    .context("Failed to read current URL")?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "get_url",
                    "url": url.as_str(),
                }))
            }
            BrowserAction::Screenshot { path, full_page } => {
                let client = self.active_client()?;
                let png = client
                    .screenshot()
                    .await
                    .context("Failed to capture screenshot")?;
                let mut payload = json!({
                    "backend": "rust_native",
                    "action": "screenshot",
                    "full_page": full_page,
                    "bytes": png.len(),
                });

                if let Some(path_str) = path {
                    tokio::fs::write(&path_str, &png)
                        .await
                        .with_context(|| format!("Failed to write screenshot to {path_str}"))?;
                    payload["path"] = Value::String(path_str);
                } else {
                    payload["png_base64"] =
                        Value::String(base64::engine::general_purpose::STANDARD.encode(&png));
                }

                Ok(payload)
            }
            BrowserAction::Wait { selector, ms, text } => {
                let client = self.active_client()?;
                if let Some(sel) = selector.as_ref() {
                    wait_for_selector(client, sel).await?;
                    Ok(json!({
                        "backend": "rust_native",
                        "action": "wait",
                        "selector": sel,
                    }))
                } else if let Some(duration_ms) = ms {
                    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                    Ok(json!({
                        "backend": "rust_native",
                        "action": "wait",
                        "ms": duration_ms,
                    }))
                } else if let Some(needle) = text.as_ref() {
                    let xpath = xpath_contains_text(needle);
                    client
                        .wait()
                        .for_element(Locator::XPath(&xpath))
                        .await
                        .with_context(|| {
                            format!("Timed out waiting for text to appear: {needle}")
                        })?;
                    Ok(json!({
                        "backend": "rust_native",
                        "action": "wait",
                        "text": needle,
                    }))
                } else {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    Ok(json!({
                        "backend": "rust_native",
                        "action": "wait",
                        "ms": 250,
                    }))
                }
            }
            BrowserAction::Press { key } => {
                let client = self.active_client()?;
                let key_input = webdriver_key(&key);
                match client.active_element().await {
                    Ok(element) => {
                        element.send_keys(&key_input).await?;
                    }
                    Err(_) => {
                        find_element(client, "body")
                            .await?
                            .send_keys(&key_input)
                            .await?;
                    }
                }

                Ok(json!({
                    "backend": "rust_native",
                    "action": "press",
                    "key": key,
                }))
            }
            BrowserAction::Hover { selector } => {
                let client = self.active_client()?;
                let element = find_element(client, &selector).await?;
                hover_element(client, &element).await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "hover",
                    "selector": selector,
                }))
            }
            BrowserAction::Scroll { direction, pixels } => {
                let client = self.active_client()?;
                let amount = i64::from(pixels.unwrap_or(600));
                let (dx, dy) = match direction.as_str() {
                    "up" => (0, -amount),
                    "down" => (0, amount),
                    "left" => (-amount, 0),
                    "right" => (amount, 0),
                    _ => anyhow::bail!(
                        "Unsupported scroll direction '{direction}'. Use up/down/left/right"
                    ),
                };

                let position = client
                    .execute(
                        "window.scrollBy(arguments[0], arguments[1]); return { x: window.scrollX, y: window.scrollY };",
                        vec![json!(dx), json!(dy)],
                    )
                    .await
                    .context("Failed to execute scroll script")?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "scroll",
                    "position": position,
                }))
            }
            BrowserAction::IsVisible { selector } => {
                let client = self.active_client()?;
                let visible = find_element(client, &selector)
                    .await?
                    .is_displayed()
                    .await?;

                Ok(json!({
                    "backend": "rust_native",
                    "action": "is_visible",
                    "selector": selector,
                    "visible": visible,
                }))
            }
            BrowserAction::Close => {
                if let Some(client) = self.client.take() {
                    let _ = client.close().await;
                }

                Ok(json!({
                    "backend": "rust_native",
                    "action": "close",
                    "closed": true,
                }))
            }
            BrowserAction::Find {
                by,
                value,
                action,
                fill_value,
            } => {
                let client = self.active_client()?;
                let selector = selector_for_find(&by, &value);
                let element = find_element(client, &selector).await?;

                let payload = match action.as_str() {
                    "click" => {
                        element.click().await?;
                        json!({"result": "clicked"})
                    }
                    "fill" => {
                        let fill = fill_value.ok_or_else(|| {
                            anyhow::anyhow!("find_action='fill' requires fill_value")
                        })?;
                        let _ = element.clear().await;
                        element.send_keys(&fill).await?;
                        json!({"result": "filled", "typed": fill.len()})
                    }
                    "text" => {
                        let text = element.text().await?;
                        json!({"result": "text", "text": text})
                    }
                    "hover" => {
                        hover_element(client, &element).await?;
                        json!({"result": "hovered"})
                    }
                    "check" => {
                        let checked_before = element_checked(&element).await?;
                        if !checked_before {
                            element.click().await?;
                        }
                        let checked_after = element_checked(&element).await?;
                        json!({
                            "result": "checked",
                            "checked_before": checked_before,
                            "checked_after": checked_after,
                        })
                    }
                    _ => anyhow::bail!(
                        "Unsupported find_action '{action}'. Use click/fill/text/hover/check"
                    ),
                };

                Ok(json!({
                    "backend": "rust_native",
                    "action": "find",
                    "by": by,
                    "value": value,
                    "selector": selector,
                    "data": payload,
                }))
            }
        }
    }

    async fn ensure_session(
        &mut self,
        headless: bool,
        webdriver_url: &str,
        chrome_path: Option<&str>,
    ) -> Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        let mut capabilities: Map<String, Value> = Map::new();
        let mut chrome_options: Map<String, Value> = Map::new();
        let mut args: Vec<Value> = Vec::new();

        if headless {
            args.push(Value::String("--headless=new".to_string()));
            args.push(Value::String("--disable-gpu".to_string()));
        }

        if !args.is_empty() {
            chrome_options.insert("args".to_string(), Value::Array(args));
        }

        if let Some(path) = chrome_path {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                chrome_options.insert("binary".to_string(), Value::String(trimmed.to_string()));
            }
        }

        if !chrome_options.is_empty() {
            capabilities.insert(
                "goog:chromeOptions".to_string(),
                Value::Object(chrome_options),
            );
        }

        let mut builder =
            ClientBuilder::rustls().context("Failed to initialize rustls connector")?;
        if !capabilities.is_empty() {
            builder.capabilities(capabilities);
        }

        let client = builder
            .connect(webdriver_url)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to WebDriver at {webdriver_url}. Start chromedriver/geckodriver first"
                )
            })?;

        self.client = Some(client);
        Ok(())
    }

    fn active_client(&self) -> Result<&Client> {
        self.client.as_ref().ok_or_else(|| {
            anyhow::anyhow!("No active native browser session. Run browser action='open' first")
        })
    }
}

fn webdriver_endpoint_reachable(webdriver_url: &str, timeout: Duration) -> bool {
    let parsed = match reqwest::Url::parse(webdriver_url) {
        Ok(url) => url,
        Err(_) => return false,
    };

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) if !h.is_empty() => h,
        _ => return false,
    };

    let port = parsed.port_or_known_default().unwrap_or(4444);
    let mut addrs = match (host, port).to_socket_addrs() {
        Ok(iter) => iter,
        Err(_) => return false,
    };

    let addr = match addrs.next() {
        Some(a) => a,
        None => return false,
    };

    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

fn selector_for_find(by: &str, value: &str) -> String {
    let escaped = css_attr_escape(value);
    match by {
        "role" => format!(r#"[role=\"{escaped}\"]"#),
        "label" => format!("label={value}"),
        "placeholder" => format!(r#"[placeholder=\"{escaped}\"]"#),
        "testid" => format!(r#"[data-testid=\"{escaped}\"]"#),
        _ => format!("text={value}"),
    }
}

async fn wait_for_selector(client: &Client, selector: &str) -> Result<()> {
    match parse_selector(selector) {
        SelectorKind::Css(css) => {
            client
                .wait()
                .for_element(Locator::Css(&css))
                .await
                .with_context(|| format!("Timed out waiting for selector '{selector}'"))?;
        }
        SelectorKind::XPath(xpath) => {
            client
                .wait()
                .for_element(Locator::XPath(&xpath))
                .await
                .with_context(|| format!("Timed out waiting for selector '{selector}'"))?;
        }
    }
    Ok(())
}

async fn find_element(client: &Client, selector: &str) -> Result<fantoccini::elements::Element> {
    let element = match parse_selector(selector) {
        SelectorKind::Css(css) => client
            .find(Locator::Css(&css))
            .await
            .with_context(|| format!("Failed to find element by CSS '{css}'"))?,
        SelectorKind::XPath(xpath) => client
            .find(Locator::XPath(&xpath))
            .await
            .with_context(|| format!("Failed to find element by XPath '{xpath}'"))?,
    };
    Ok(element)
}

async fn hover_element(client: &Client, element: &fantoccini::elements::Element) -> Result<()> {
    let actions = MouseActions::new("mouse".to_string()).then(PointerAction::MoveToElement {
        element: element.clone(),
        duration: Some(Duration::from_millis(150)),
        x: 0.0,
        y: 0.0,
    });

    client
        .perform_actions(actions)
        .await
        .context("Failed to perform hover action")?;
    let _ = client.release_actions().await;
    Ok(())
}

async fn element_checked(element: &fantoccini::elements::Element) -> Result<bool> {
    let checked = element
        .prop("checked")
        .await
        .context("Failed to read checkbox checked property")?
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(matches!(checked.as_str(), "true" | "checked" | "1"))
}

enum SelectorKind {
    Css(String),
    XPath(String),
}

fn parse_selector(selector: &str) -> SelectorKind {
    let trimmed = selector.trim();
    if let Some(text_query) = trimmed.strip_prefix("text=") {
        return SelectorKind::XPath(xpath_contains_text(text_query));
    }

    if let Some(label_query) = trimmed.strip_prefix("label=") {
        let literal = xpath_literal(label_query);
        return SelectorKind::XPath(format!(
            "(//label[contains(normalize-space(.), {literal})]/following::*[self::input or self::textarea or self::select][1] | //*[@aria-label and contains(normalize-space(@aria-label), {literal})] | //label[contains(normalize-space(.), {literal})])"
        ));
    }

    if trimmed.starts_with('@') {
        let escaped = css_attr_escape(trimmed);
        return SelectorKind::Css(format!(r#"[data-zc-ref=\"{escaped}\"]"#));
    }

    SelectorKind::Css(trimmed.to_string())
}

fn css_attr_escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

fn xpath_contains_text(text: &str) -> String {
    format!("//*[contains(normalize-space(.), {})]", xpath_literal(text))
}

fn xpath_literal(input: &str) -> String {
    if !input.contains('"') {
        return format!("\"{input}\"");
    }
    if !input.contains('\'') {
        return format!("'{input}'");
    }

    let segments: Vec<&str> = input.split('"').collect();
    let mut parts: Vec<String> = Vec::new();
    for (index, part) in segments.iter().enumerate() {
        if !part.is_empty() {
            parts.push(format!("\"{part}\""));
        }
        if index + 1 < segments.len() {
            parts.push("'\"'".to_string());
        }
    }

    if parts.is_empty() {
        "\"\"".to_string()
    } else {
        format!("concat({})", parts.join(","))
    }
}

fn webdriver_key(key: &str) -> String {
    match key.trim().to_ascii_lowercase().as_str() {
        "enter" => Key::Enter.to_string(),
        "return" => Key::Return.to_string(),
        "tab" => Key::Tab.to_string(),
        "escape" | "esc" => Key::Escape.to_string(),
        "backspace" => Key::Backspace.to_string(),
        "delete" => Key::Delete.to_string(),
        "space" => Key::Space.to_string(),
        "arrowup" | "up" => Key::Up.to_string(),
        "arrowdown" | "down" => Key::Down.to_string(),
        "arrowleft" | "left" => Key::Left.to_string(),
        "arrowright" | "right" => Key::Right.to_string(),
        "home" => Key::Home.to_string(),
        "end" => Key::End.to_string(),
        "pageup" => Key::PageUp.to_string(),
        "pagedown" => Key::PageDown.to_string(),
        other => other.to_string(),
    }
}

fn snapshot_script(interactive_only: bool, compact: bool, depth: Option<i64>) -> String {
    let depth_literal = depth
        .map(|level| level.to_string())
        .unwrap_or_else(|| "null".to_string());

    format!(
        r#"(() => {{
  const interactiveOnly = {interactive_only};
  const compact = {compact};
  const maxDepth = {depth_literal};
  const nodes = [];
  const root = document.body || document.documentElement;
  let counter = 0;

  const isVisible = (el) => {{
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity || 1) === 0) {{
      return false;
    }}
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }};

  const isInteractive = (el) => {{
    if (el.matches('a,button,input,select,textarea,summary,[role],*[tabindex]')) return true;
    return typeof el.onclick === 'function';
  }};

  const describe = (el, depth) => {{
    const interactive = isInteractive(el);
    const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 140);
    if (interactiveOnly && !interactive) return;
    if (compact && !interactive && !text) return;

    const ref = '@e' + (++counter);
    el.setAttribute('data-zc-ref', ref);
    nodes.push({{
      ref,
      depth,
      tag: el.tagName.toLowerCase(),
      id: el.id || null,
      role: el.getAttribute('role'),
      text,
      interactive,
    }});
  }};

  const walk = (el, depth) => {{
    if (!(el instanceof Element)) return;
    if (maxDepth !== null && depth > maxDepth) return;
    if (isVisible(el)) {{
      describe(el, depth);
    }}
    for (const child of el.children) {{
      walk(child, depth + 1);
      if (nodes.length >= 400) return;
    }}
  }};

  if (root) walk(root, 0);

  return {{
    title: document.title,
    url: window.location.href,
    count: nodes.length,
    nodes,
  }};
}})();"#
    )
}
