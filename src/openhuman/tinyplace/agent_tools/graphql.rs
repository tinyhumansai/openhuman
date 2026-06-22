//! `tinyplace_graphql` — the batched read gateway.
//!
//! One tool fronts the SDK's read-only GraphQL surface (`client.graphql.*`).
//! A single request hydrates a list **and** every embedded author/creator
//! profile, so the agent never fans out one REST call per author. The agent
//! picks a named `query` and passes a few common params; the Rust side builds
//! the typed query, calls the SDK, and renders the result as markdown.

use serde_json::{json, Value};

use tinyplace::api::graphql::{BountyGraphQLParams, PostGraphQLParams};
use tinyplace::types::{AgentQueryParams, JobQueryParams, LedgerListParams, ProductQueryParams};

use crate::openhuman::tools::traits::Tool;

use super::common::{
    client, err_md, ok_md, opt_bool, opt_i64, opt_str, req_str, val_or_err, FlowFuture, FlowTool,
};
use super::render::{render_json, Markdown};

const SUPPORTED: &[&str] = &[
    "home_feed",
    "posts",
    "post",
    "agents",
    "agent_card",
    "identity",
    "profile",
    "user",
    "jobs",
    "job",
    "bounties",
    "bounty",
    "products",
    "product",
    "ledger_transactions",
    "ledger_transaction",
];

pub fn graphql_tool() -> Box<dyn Tool> {
    FlowTool::read(
        "tinyplace_graphql",
        "Read tiny.place data through the batched GraphQL gateway (one request \
         hydrates embedded author/creator profiles). Pick a `query` and pass the \
         params it needs. Supported queries: home_feed, posts, post, agents, \
         agent_card, identity, profile, user, jobs, job, bounties, bounty, \
         products, product, ledger_transactions, ledger_transaction. Read-only.",
        schema(),
        graphql_flow,
    )
    .boxed()
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "enum": SUPPORTED,
                "description": "Which GraphQL read to run."
            },
            "handle": { "type": "string", "description": "@handle (for posts/post)." },
            "id": { "type": "string", "description": "Resource id (post/agent/job/bounty/product/tx)." },
            "post_id": { "type": "string", "description": "Post id (for the `post` query)." },
            "username": { "type": "string", "description": "Username (identity/profile)." },
            "crypto_id": { "type": "string", "description": "Wallet cryptoId (user query)." },
            "q": { "type": "string", "description": "Free-text query (agents/products)." },
            "skill": { "type": "string", "description": "Skill filter (agents/jobs)." },
            "status": { "type": "string", "description": "Status filter (jobs/bounties)." },
            "creator": { "type": "string", "description": "Creator filter (bounties)." },
            "include_self": { "type": "boolean", "description": "Include your own posts (home_feed)." },
            "limit": { "type": "integer", "description": "Max items (default per query)." },
            "offset": { "type": "integer", "description": "Pagination offset." }
        },
        "required": ["query"]
    })
}

fn graphql_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let query = req_str(&args, "query")?;
        log::debug!("[tinyplace][flow] graphql start query={query}");
        let client = client().await?;
        // Bound limit/offset to non-negative so a bad value can't reach the SDK.
        let limit = opt_i64(&args, "limit").filter(|v| *v > 0);
        let offset = opt_i64(&args, "offset").filter(|v| *v >= 0);

        // Each arm calls the typed SDK method, then serialises to JSON so the
        // generic markdown renderer can present it uniformly.
        let value: Value = match query.as_str() {
            "home_feed" => val_or_err(
                "Read home feed",
                client
                    .graphql
                    .home_feed(limit, offset, opt_bool(&args, "include_self"))
                    .await,
            )?,
            "posts" => {
                let handle = req_str(&args, "handle")?;
                let params = PostGraphQLParams {
                    limit,
                    before: None,
                    viewer: None,
                };
                val_or_err(
                    "Read posts",
                    client.graphql.posts(&handle, Some(&params)).await,
                )?
            }
            "post" => {
                let handle = req_str(&args, "handle")?;
                let post_id = req_str(&args, "post_id").or_else(|_| req_str(&args, "id"))?;
                val_or_err(
                    "Read post",
                    client.graphql.post(&handle, &post_id, None).await,
                )?
            }
            "agents" => {
                let params = AgentQueryParams {
                    q: opt_str(&args, "q"),
                    skill: opt_str(&args, "skill"),
                    limit,
                    offset,
                    ..Default::default()
                };
                val_or_err("Read agents", client.graphql.agents(Some(&params)).await)?
            }
            "agent_card" => val_or_err(
                "Read agent card",
                client.graphql.agent_card(&req_str(&args, "id")?).await,
            )?,
            "identity" => val_or_err(
                "Read identity",
                client.graphql.identity(&req_str(&args, "username")?).await,
            )?,
            "profile" => val_or_err(
                "Read profile",
                client.graphql.profile(&req_str(&args, "username")?).await,
            )?,
            "user" => val_or_err(
                "Read user",
                client.graphql.user(&req_str(&args, "crypto_id")?).await,
            )?,
            "jobs" => {
                let params = JobQueryParams {
                    skill: opt_str(&args, "skill"),
                    // status is a typed enum on JobQueryParams; parse the string.
                    status: opt_str(&args, "status")
                        .and_then(|s| serde_json::from_value(Value::String(s)).ok()),
                    limit,
                    offset,
                    ..Default::default()
                };
                val_or_err("Read jobs", client.graphql.jobs(Some(&params)).await)?
            }
            "job" => val_or_err("Read job", client.graphql.job(&req_str(&args, "id")?).await)?,
            "bounties" => {
                let params = BountyGraphQLParams {
                    status: opt_str(&args, "status"),
                    creator: opt_str(&args, "creator"),
                    limit,
                    offset,
                };
                val_or_err(
                    "Read bounties",
                    client.graphql.bounties(Some(&params)).await,
                )?
            }
            "bounty" => val_or_err(
                "Read bounty",
                client.graphql.bounty(&req_str(&args, "id")?).await,
            )?,
            "products" => {
                let params = ProductQueryParams {
                    q: opt_str(&args, "q"),
                    limit,
                    offset,
                    ..Default::default()
                };
                val_or_err(
                    "Read products",
                    client.graphql.products(Some(&params)).await,
                )?
            }
            "product" => val_or_err(
                "Read product",
                client.graphql.product(&req_str(&args, "id")?).await,
            )?,
            "ledger_transactions" => {
                let params = LedgerListParams {
                    limit,
                    offset,
                    ..Default::default()
                };
                val_or_err(
                    "Read ledger",
                    client.graphql.ledger_transactions(Some(&params)).await,
                )?
            }
            "ledger_transaction" => val_or_err(
                "Read ledger transaction",
                client
                    .graphql
                    .ledger_transaction(&req_str(&args, "id")?)
                    .await,
            )?,
            other => {
                let mut md = Markdown::new();
                md.heading("Unknown query");
                md.paragraph(format!("`{other}` is not a supported GraphQL read."));
                md.kv([("Supported", SUPPORTED.join(", "))]);
                return Ok(err_md(md.build()));
            }
        };

        let mut md = Markdown::new();
        md.heading(format!("tiny.place · {query}"));
        md.raw_section(render_json(&value));
        Ok(ok_md(md.build()))
    })
}
