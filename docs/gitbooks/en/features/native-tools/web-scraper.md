---
description: >-
  The `web_fetch` tool reads a web page and returns it as Markdown instead of
  raw HTML, behind an allowlist and an SSRF guard.
icon: globe
---

# Web scraper

`web_fetch` is the agent's tool for reading a page. It is separate from `http_request` and `curl` because the agent rarely wants the markup. It wants the page. HTML comes back as Markdown, with links kept and scripts dropped.

Use `http_request` when you need POST, custom headers or richer HTTP behavior. Use `curl` to stream a file to disk.

## Arguments

| Argument | Required | Effect |
| --- | --- | --- |
| `url` | Yes | An absolute `http` or `https` URL. |
| `max_bytes` | No | Truncate the body at this many bytes instead of the configured cap. |
| `raw` | No | Skip the Markdown conversion and return the body as served. This does not bypass the limits. The response cap and any `max_bytes` still truncate, and the result still carries the truncation marker. |
| `summary_focus` | No | Steers the [TokenJuice](../token-compression.md) summary toward a topic. |

The tool is read-only and never asks for approval. Parallel `web_fetch` calls are safe, because a GET is idempotent.

Conversion follows the response `Content-Type`, and falls back to content sniffing when the header is missing. A response that is explicitly not HTML is returned as is.

## Limits

| Limit | Value | Where it comes from |
| --- | --- | --- |
| Body cap | 1,000,000 bytes | `[http_request].max_response_size` |
| Request timeout | 30 seconds | `[http_request].timeout_secs` |
| Text handed to the model in one result | 24,000 characters | `max_result_size_chars` in `web_fetch.rs` |
| Redirects followed | None | `redirect::Policy::none()` |

The first two are the shipped defaults in `crates/openhuman-core/src/config/schema/tools/http.rs`, shared with `http_request`. A `0` in either key counts as unset and falls back to the default, so a stale zero cannot disable every fetch.

- **Oversize pages are truncated, not dropped.** The body is cut at a character boundary, and the result header records `download_capped_at=<N>B` so the agent knows the page is partial. The cap bounds what reaches the model, not what is downloaded, because the response is read before it is trimmed. Streaming with a hard download ceiling is `curl`'s job.
- **Past 24,000 characters the rest is kept.** The full extracted page is saved as an artifact, and the result carries the `file_read` call that pages through it.

Redirects are never followed. A redirect target can sit on a host the allowlist would refuse, so a 3xx returns a result naming its `Location` and leaves the decision to the agent. Calling `web_fetch` again with that URL runs the whole guard from scratch.

## The allowlist and the URL guard

Every fetch passes the shared URL guard in TinyAgents' harness before any connection is made. `http_request` and `curl` use the same gate.

**Shape.** Only `http` and `https` are accepted. URLs with whitespace, a backslash, userinfo (`user:pass@`), a percent-encoded authority or an IPv6 literal host are rejected.

**Allowed websites.** `[http_request].allowed_domains` ships as `["*"]`, so research works out of the box. An exact host also matches its subdomains. `"*"` allows all public sites, and an empty list blocks all web access. If every entry is malformed, the list fails closed. To narrow it, use the allowed-websites setting described in [Web search](web-search.md#settings), which writes the same key.

**Address checks.** The guard resolves the hostname and refuses the fetch if any resolved address is non-global. That covers loopback, RFC1918 private, link-local (including the cloud metadata address `169.254.169.254`), CGNAT, unspecified and broadcast ranges, and the IPv6 equivalents including unique-local, link-local and IPv4-mapped forms. Hostnames such as `localhost`, anything ending in `.localhost` and anything under a `.local` TLD are refused by name. These checks apply even under `"*"`, so the wildcard opens public hosts and never the private network.

Under privacy mode's local-only setting, the fetch is refused before validation or DNS, so a blocked destination is never looked up. Rate limiting applies, and the egress disclosure records the destination host before the request leaves.

## What it is good for

- Reading articles, documentation pages and GitHub READMEs without the surrounding noise.
- Following up on a [web search](web-search.md) result.
- Summarizing a single known page on demand.

## See also

- [Web search](web-search.md): finds the URLs to feed this tool, and owns the allowed-websites list.
- [Token compression](../token-compression.md): what trims a long page before it reaches the model.
- [Browser and computer control](browser-and-computer.md): for pages that need clicking rather than reading.
- [Privacy and security](../privacy-and-security.md): the local-only mode and egress disclosure behind the guard.
