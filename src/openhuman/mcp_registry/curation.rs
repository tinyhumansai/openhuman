//! Catalog curation: badge the canonical first-party server for well-known
//! services.
//!
//! The registry lists many community servers per popular service and carries no
//! single "official" flag. Rather than collapse a service to one (arbitrary)
//! row — which hides genuinely different community servers and, across the
//! whole 13k-server catalog, only removes ~0.9% of rows — we keep the full
//! deduped catalog browsable and simply mark the *known canonical vendor
//! server* for a service with an `official` badge.
//!
//! Matching is on the exact `qualified_name`, never a name substring: a term
//! like "stripe" or "github" also appears in unrelated community servers (an
//! Obsidian-GitHub plugin, a `meok-stripe-acp-checkout` fork, …), so a substring
//! "verified" badge would vouch for servers nobody has vetted. Extend the list
//! as vendors publish official servers.

use super::types::SmitheryServerSummary;

/// Canonical first-party servers, by exact registry `qualified_name`. Each was
/// confirmed present in the official registry export (2026-06). These get the
/// `official` badge; every other server is shown without one.
const OFFICIAL_SERVERS: &[&str] = &[
    "io.github.github/github-mcp-server",
    "com.notion/mcp",
    "com.stripe/mcp",
    "com.atlassian/atlassian-mcp-server",
    "app.linear/linear",
    "com.gitlab/mcp",
    "com.paypal.mcp/mcp",
    "com.cloudflare.mcp/mcp",
    "com.airtable/mcp",
    "com.supabase/mcp",
    "com.vercel/vercel-mcp",
    "com.webflow/mcp",
    "com.wix/mcp",
];

/// Mark the canonical first-party server for each known service with the
/// `official` badge. Exact `qualified_name` match — never a name substring, so
/// a community server merely *named* after a vendor is never badged. Mutates in
/// place.
pub fn tag_official(servers: &mut [SmitheryServerSummary]) {
    for server in servers.iter_mut() {
        server.official = OFFICIAL_SERVERS.contains(&server.qualified_name.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(qualified_name: &str) -> SmitheryServerSummary {
        SmitheryServerSummary {
            qualified_name: qualified_name.to_string(),
            display_name: qualified_name.to_string(),
            description: None,
            icon_url: None,
            use_count: 0,
            is_deployed: true,
            source: "mcp_official".to_string(),
            official: false,
            extra: Default::default(),
        }
    }

    #[test]
    fn tags_only_exact_canonical_servers() {
        let mut servers = vec![
            server("io.github.github/github-mcp-server"), // official
            server("ai.smithery/Hint-Services-obsidian-github-mcp"), // 'github' in name, NOT official
            server("com.notion/mcp"),                                // official
            server("ai.smithery/smithery-notion"),                   // community
            server("io.github.CSOAI-ORG/meok-stripe-acp-checkout-mcp"), // 'stripe' in name, NOT official
        ];

        tag_official(&mut servers);

        assert!(servers[0].official);
        assert!(
            !servers[1].official,
            "a name merely containing 'github' must not be marked official"
        );
        assert!(servers[2].official);
        assert!(!servers[3].official);
        assert!(
            !servers[4].official,
            "a name merely containing 'stripe' must not be marked official"
        );
    }
}
