---
description: Open URLs, inspect DOM snapshots, click, type, and move the mouse - natively.
icon: display
---

# Browser & Computer Control

When the agent needs to *use* your machine the way a person would - open a page, inspect its DOM snapshot, click a button, type a phrase - these tools are how it does it.

## Browser

* **Open** a URL in an embedded webview the agent can read back from.
* **Snapshot** the current page's accessibility/DOM structure, including stable element references for later actions.

The browser surface runs through CEF (Chromium Embedded Framework) and includes a security layer that scopes what pages can do. See [Chromium Embedded Framework](../../developing/cef.md) for the platform details.

## Computer (mouse + keyboard)

* **Mouse** - move, click, drag.
* **Keyboard** - type text, send key chords.
* **Human path** - moves and clicks follow human-like trajectories rather than teleporting, so they don't trip naive bot detection.

## What it's good for

* Driving sites that don't have an API or a [native integration](../integrations/README.md).
* Multi-step UI flows where an interactive DOM snapshot provides the next actionable element.
* Automating local apps from inside a chat.

## See also

* [Web Scraper](web-scraper.md) - when you only need the article, not the whole page.
* [Chromium Embedded Framework](../../developing/cef.md) - the runtime browser layer.
