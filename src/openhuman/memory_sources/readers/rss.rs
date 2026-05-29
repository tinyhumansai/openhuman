//! RSS/Atom feed source reader.
//!
//! Fetches and parses an RSS or Atom feed, returning entries as
//! source items. Uses a lightweight XML parser (`quick-xml` via
//! manual parsing) to avoid pulling in heavy feed crates.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

const DEFAULT_MAX_ITEMS: u32 = 50;
const MAX_FEED_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB — guards against pathological feeds

pub struct RssReader;

#[async_trait]
impl SourceReader for RssReader {
    fn kind(&self) -> SourceKind {
        SourceKind::RssFeed
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let url = source.url.as_deref().ok_or("rss source requires a url")?;
        let max_items = source.max_items.unwrap_or(DEFAULT_MAX_ITEMS) as usize;

        tracing::debug!(
            host = %url_host(url),
            max_items = max_items,
            "[memory_sources:rss] listing items"
        );

        let body = fetch_url(url).await?;
        let entries = parse_feed(&body, max_items)?;

        tracing::debug!(count = entries.len(), "[memory_sources:rss] parsed entries");

        Ok(entries)
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        _config: &Config,
    ) -> Result<SourceContent, String> {
        let url = source.url.as_deref().ok_or("rss source requires a url")?;

        tracing::debug!(
            host = %url_host(url),
            item_id = %item_id,
            "[memory_sources:rss] reading item"
        );

        let body = fetch_url(url).await?;
        let entries = parse_feed_full(&body)?;

        let entry = entries
            .into_iter()
            .find(|e| e.id == item_id)
            .ok_or_else(|| format!("item '{item_id}' not found in feed"))?;

        let content_type = if entry.body.contains('<') {
            ContentType::Html
        } else {
            ContentType::Plaintext
        };

        Ok(SourceContent {
            id: entry.id,
            title: entry.title,
            body: entry.body,
            content_type,
            metadata: serde_json::json!({
                "link": entry.link,
                "published": entry.published,
            }),
        })
    }
}

/// Extract just the host portion of a URL for debug-log redaction so we
/// don't leak query params, paths, or embedded credentials.
fn url_host(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(stripped)
        .to_string()
}

async fn fetch_url(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    let resp = client
        .get(url)
        .header("User-Agent", "openhuman")
        .send()
        .await
        .map_err(|e| format!("failed to fetch feed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("feed returned {}", resp.status()));
    }

    // Guard against pathologically large feeds before buffering into memory.
    if let Some(len) = resp.content_length() {
        if len > MAX_FEED_BYTES {
            return Err(format!(
                "feed body too large: {len} bytes (limit {MAX_FEED_BYTES})"
            ));
        }
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("failed to read feed body: {e}"))?;

    if bytes.len() as u64 > MAX_FEED_BYTES {
        return Err(format!(
            "feed body too large: {} bytes (limit {MAX_FEED_BYTES})",
            bytes.len()
        ));
    }

    String::from_utf8(bytes.to_vec()).map_err(|e| format!("feed body is not valid UTF-8: {e}"))
}

#[derive(Debug)]
struct FeedEntry {
    id: String,
    title: String,
    body: String,
    link: Option<String>,
    published: Option<String>,
}

fn parse_feed(xml: &str, max_items: usize) -> Result<Vec<SourceItem>, String> {
    let entries = parse_feed_full(xml)?;
    Ok(entries
        .into_iter()
        .take(max_items)
        .map(|e| SourceItem {
            id: e.id,
            title: e.title,
            updated_at_ms: None,
        })
        .collect())
}

fn parse_feed_full(xml: &str) -> Result<Vec<FeedEntry>, String> {
    // Detect RSS vs Atom by looking for <rss or <feed
    if xml.contains("<rss") || xml.contains("<channel") {
        parse_rss(xml)
    } else if xml.contains("<feed") {
        parse_atom(xml)
    } else {
        Err("unrecognized feed format (expected RSS or Atom)".to_string())
    }
}

fn parse_rss(xml: &str) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();
    let mut offset = 0;

    while let Some(item_start) = xml[offset..].find("<item") {
        let abs_start = offset + item_start;
        let item_end = xml[abs_start..]
            .find("</item>")
            .map(|i| abs_start + i + 7)
            .unwrap_or(xml.len());

        let item_xml = &xml[abs_start..item_end];
        let title = extract_tag(item_xml, "title").unwrap_or_default();
        let link = extract_tag(item_xml, "link");
        let guid = extract_tag(item_xml, "guid");
        let description = extract_tag(item_xml, "description")
            .or_else(|| extract_cdata(item_xml, "content:encoded"))
            .unwrap_or_default();
        let pub_date = extract_tag(item_xml, "pubDate");

        let id = guid
            .or_else(|| link.clone())
            .unwrap_or_else(|| format!("rss-{}", entries.len()));

        entries.push(FeedEntry {
            id,
            title,
            body: description,
            link,
            published: pub_date,
        });

        offset = item_end;
    }

    Ok(entries)
}

fn parse_atom(xml: &str) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();
    let mut offset = 0;

    while let Some(entry_start) = xml[offset..].find("<entry") {
        let abs_start = offset + entry_start;
        let entry_end = xml[abs_start..]
            .find("</entry>")
            .map(|i| abs_start + i + 8)
            .unwrap_or(xml.len());

        let entry_xml = &xml[abs_start..entry_end];
        let title = extract_tag(entry_xml, "title").unwrap_or_default();
        let id = extract_tag(entry_xml, "id").unwrap_or_else(|| format!("atom-{}", entries.len()));
        let content = extract_tag(entry_xml, "content")
            .or_else(|| extract_tag(entry_xml, "summary"))
            .unwrap_or_default();
        let link = extract_attr(entry_xml, "link", "href");
        let updated =
            extract_tag(entry_xml, "updated").or_else(|| extract_tag(entry_xml, "published"));

        entries.push(FeedEntry {
            id,
            title,
            body: content,
            link,
            published: updated,
        });

        offset = entry_end;
    }

    Ok(entries)
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let content_start = xml[start..].find('>')? + start + 1;
    let end = xml[content_start..].find(&close)? + content_start;
    let content = &xml[content_start..end];
    Some(decode_xml_entities(content.trim()))
}

fn extract_cdata(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let content_start = xml[start..].find('>')? + start + 1;
    let end = xml[content_start..].find(&close)? + content_start;
    let content = &xml[content_start..end];
    let cleaned = content
        .trim()
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(content);
    Some(cleaned.trim().to_string())
}

fn extract_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{tag} ");
    let start = xml.find(&open)?;
    let tag_end = xml[start..].find('>')? + start;
    let tag_str = &xml[start..tag_end];
    let attr_start = tag_str.find(&format!("{attr}=\""))? + attr.len() + 2;
    let attr_end = tag_str[attr_start..].find('"')? + attr_start;
    Some(tag_str[attr_start..attr_end].to_string())
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rss_extracts_items() {
        let xml = r#"<?xml version="1.0"?>
        <rss version="2.0">
        <channel>
            <title>Test Feed</title>
            <item>
                <title>First post</title>
                <link>https://example.com/1</link>
                <description>Body of first post</description>
            </item>
            <item>
                <title>Second post</title>
                <guid>guid-2</guid>
                <description>Body of second</description>
            </item>
        </channel>
        </rss>"#;

        let entries = parse_rss(xml).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "First post");
        assert_eq!(entries[0].id, "https://example.com/1");
        assert_eq!(entries[1].id, "guid-2");
    }

    #[test]
    fn parse_atom_extracts_entries() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <entry>
                <title>Atom entry</title>
                <id>urn:entry:1</id>
                <content>Content here</content>
                <link href="https://example.com/atom/1" />
            </entry>
        </feed>"#;

        let entries = parse_atom(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Atom entry");
        assert_eq!(entries[0].id, "urn:entry:1");
        assert_eq!(
            entries[0].link.as_deref(),
            Some("https://example.com/atom/1")
        );
    }

    #[test]
    fn parse_feed_detects_format() {
        let rss = "<rss><channel><item><title>T</title></item></channel></rss>";
        assert!(parse_feed(rss, 10).is_ok());

        let atom = "<feed><entry><title>T</title><id>1</id></entry></feed>";
        assert!(parse_feed(atom, 10).is_ok());

        assert!(parse_feed("<html></html>", 10).is_err());
    }
}
