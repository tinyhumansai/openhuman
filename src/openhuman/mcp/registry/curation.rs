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

/// Whether a catalog row fully specifies how to connect *from its metadata
/// alone* — no probe, no guessing. A "perfect" server declares both a
/// `website_url` (a trust/quality signal and the user's get-key destination)
/// and a named static credential (`auth_kind == "api_key"`).
pub(super) fn is_perfect_server(s: &SmitheryServerSummary) -> bool {
    s.website_url
        .as_deref()
        .is_some_and(|u| !u.trim().is_empty())
        && s.auth_kind.as_deref() == Some("api_key")
}

/// Strict "perfect server" catalog filter. Keeps only [`is_perfect_server`]
/// rows, dropping OAuth-only, open, and under-declared servers (and every
/// Smithery summary, which carries neither website nor declared auth). This is
/// a deliberate quality-over-quantity trade-off (#4272): the user only ever
/// sees servers that can be installed and connected with confidence. Returns
/// the number of rows dropped so callers can log the trim. Mutates in place.
pub fn retain_perfect_servers(servers: &mut Vec<SmitheryServerSummary>) -> usize {
    let before = servers.len();
    servers.retain(is_perfect_server);
    before - servers.len()
}

/// Float the canonical first-party (`official`) servers to the top while
/// preserving the registry's relevance order for everything else (stable sort).
pub fn float_official_first(servers: &mut [SmitheryServerSummary]) {
    servers.sort_by_key(|s| !s.official);
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
            website_url: None,
            auth_kind: None,
            extra: Default::default(),
        }
    }

    /// A "perfect" server: declared website + api_key auth.
    fn perfect(qualified_name: &str) -> SmitheryServerSummary {
        SmitheryServerSummary {
            website_url: Some("https://vendor.example".to_string()),
            auth_kind: Some("api_key".to_string()),
            ..server(qualified_name)
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

    #[test]
    fn retain_perfect_keeps_only_website_plus_api_key() {
        let mut servers = vec![
            perfect("com.acme/mcp"), // website + api_key → kept
            SmitheryServerSummary {
                auth_kind: None,
                ..perfect("oauth/srv")
            }, // no key → dropped
            SmitheryServerSummary {
                website_url: None,
                ..perfect("nosite/srv")
            }, // no site → dropped
            SmitheryServerSummary {
                website_url: Some("   ".to_string()),
                ..perfect("blank/srv")
            }, // blank site → dropped
            server("smi/community"), // neither → dropped
        ];

        let dropped = retain_perfect_servers(&mut servers);

        let slugs: Vec<_> = servers.iter().map(|s| s.qualified_name.as_str()).collect();
        assert_eq!(slugs, vec!["com.acme/mcp"]);
        assert_eq!(dropped, 4);
    }

    #[test]
    fn float_official_first_is_stable() {
        let mut servers = vec![
            perfect("a/one"),
            SmitheryServerSummary {
                official: true,
                ..perfect("b/official")
            },
            perfect("c/two"),
        ];

        float_official_first(&mut servers);

        let slugs: Vec<_> = servers.iter().map(|s| s.qualified_name.as_str()).collect();
        // Official floats to the top; the rest keep their relative order.
        assert_eq!(slugs, vec!["b/official", "a/one", "c/two"]);
    }
}
