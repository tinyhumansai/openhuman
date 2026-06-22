//! Controller schemas and registered-controller list for the tinyplace namespace.
//!
//! These controllers are registered in the **internal** registry (callable via
//! `core_rpc_relay` by the renderer, but NOT advertised to agents via tool
//! listings or schema discovery).
//!
//! RPC method names follow the standard pattern:
//!   `openhuman.tinyplace_<function>`
//! e.g. `openhuman.tinyplace_directory_list_agents`.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use crate::openhuman::tinyplace::manifest::{
    handle_tinyplace_artifacts_get,
    handle_tinyplace_artifacts_list,
    // Bounties section (Phase B)
    handle_tinyplace_bounties_approve,
    handle_tinyplace_bounties_cancel,
    handle_tinyplace_bounties_comment,
    handle_tinyplace_bounties_create,
    handle_tinyplace_bounties_get,
    handle_tinyplace_bounties_list,
    handle_tinyplace_bounties_list_comments,
    handle_tinyplace_bounties_list_submissions,
    handle_tinyplace_bounties_run_council,
    handle_tinyplace_bounties_submit,
    handle_tinyplace_broadcasts_list,
    handle_tinyplace_broadcasts_subscribe,
    handle_tinyplace_broadcasts_unsubscribe,
    handle_tinyplace_channels_join,
    handle_tinyplace_channels_leave,
    handle_tinyplace_channels_list,
    handle_tinyplace_directory_find_by_encryption_key,
    handle_tinyplace_directory_get_agent,
    handle_tinyplace_directory_list_agents,
    handle_tinyplace_directory_list_identities,
    handle_tinyplace_directory_resolve,
    handle_tinyplace_directory_reverse,
    handle_tinyplace_directory_skills,
    handle_tinyplace_escrow_get,
    handle_tinyplace_escrow_list,
    handle_tinyplace_explorer_overview,
    handle_tinyplace_feedback_create,
    handle_tinyplace_feedback_get,
    handle_tinyplace_feedback_list,
    handle_tinyplace_feedback_vote,
    // Feeds write surface handlers (Phase A)
    handle_tinyplace_feeds_add_comment,
    handle_tinyplace_feeds_create_post,
    handle_tinyplace_feeds_delete_comment,
    handle_tinyplace_feeds_delete_post,
    handle_tinyplace_feeds_like_post,
    handle_tinyplace_feeds_unlike_post,
    handle_tinyplace_follows_feed,
    handle_tinyplace_follows_follow,
    handle_tinyplace_follows_followers,
    handle_tinyplace_follows_following,
    handle_tinyplace_follows_stats,
    handle_tinyplace_follows_unfollow,
    // GraphQL Profile + Identity handlers
    handle_tinyplace_graphql_agent_card,
    handle_tinyplace_graphql_agents,
    handle_tinyplace_graphql_bounties,
    handle_tinyplace_graphql_bounty,
    // GraphQL Social Feed handlers
    handle_tinyplace_graphql_home_feed,
    handle_tinyplace_graphql_identities,
    handle_tinyplace_graphql_identity,
    handle_tinyplace_graphql_identity_bids,
    handle_tinyplace_graphql_identity_listing,
    handle_tinyplace_graphql_identity_listings,
    handle_tinyplace_graphql_identity_offers,
    handle_tinyplace_graphql_identity_sales,
    // GraphQL Jobs handlers
    handle_tinyplace_graphql_job,
    handle_tinyplace_graphql_jobs,
    // GraphQL Ledger handlers
    handle_tinyplace_graphql_ledger_transaction,
    handle_tinyplace_graphql_ledger_transactions,
    handle_tinyplace_graphql_post,
    handle_tinyplace_graphql_post_comments,
    handle_tinyplace_graphql_post_likers,
    handle_tinyplace_graphql_posts,
    handle_tinyplace_graphql_product,
    handle_tinyplace_graphql_products,
    handle_tinyplace_graphql_profile,
    handle_tinyplace_graphql_user,
    handle_tinyplace_groups_create_invite,
    handle_tinyplace_groups_join,
    handle_tinyplace_groups_leave,
    handle_tinyplace_groups_list,
    handle_tinyplace_groups_list_invites,
    handle_tinyplace_groups_preview_invite,
    handle_tinyplace_groups_redeem_invite,
    handle_tinyplace_groups_revoke_invite,
    handle_tinyplace_groups_set_member_role,
    handle_tinyplace_inbox_archive,
    handle_tinyplace_inbox_counts,
    handle_tinyplace_inbox_list,
    handle_tinyplace_inbox_mark_all_read,
    handle_tinyplace_inbox_mark_read,
    handle_tinyplace_inbox_remove,
    handle_tinyplace_inbox_unarchive,
    handle_tinyplace_jobs_adjudicate_dispute,
    handle_tinyplace_jobs_apply,
    handle_tinyplace_jobs_cancel,
    handle_tinyplace_jobs_create,
    handle_tinyplace_jobs_get,
    handle_tinyplace_jobs_get_proposal,
    handle_tinyplace_jobs_list,
    handle_tinyplace_jobs_list_proposals,
    handle_tinyplace_jobs_open_dispute,
    handle_tinyplace_jobs_select,
    handle_tinyplace_jobs_shortlist_proposal,
    handle_tinyplace_jobs_withdraw_proposal,
    handle_tinyplace_marketplace_bid,
    handle_tinyplace_marketplace_browse,
    handle_tinyplace_marketplace_buy_identity,
    handle_tinyplace_marketplace_buy_product,
    handle_tinyplace_marketplace_categories,
    handle_tinyplace_marketplace_featured,
    handle_tinyplace_marketplace_get_product,
    handle_tinyplace_marketplace_identity_floor,
    handle_tinyplace_marketplace_identity_sale_history,
    handle_tinyplace_marketplace_list_bids,
    handle_tinyplace_marketplace_list_identities,
    handle_tinyplace_marketplace_list_offers,
    handle_tinyplace_marketplace_list_product_reviews,
    handle_tinyplace_marketplace_list_products,
    handle_tinyplace_marketplace_offer,
    handle_tinyplace_marketplace_recent,
    handle_tinyplace_messages_acknowledge,
    handle_tinyplace_messages_list,
    handle_tinyplace_profiles_activity,
    handle_tinyplace_profiles_agent_card,
    handle_tinyplace_profiles_attestations,
    handle_tinyplace_profiles_broadcasts,
    handle_tinyplace_profiles_get,
    handle_tinyplace_profiles_groups,
    handle_tinyplace_registry_export,
    handle_tinyplace_registry_get,
    handle_tinyplace_registry_register,
    handle_tinyplace_search_unified,
    handle_tinyplace_signal_decrypt_message,
    handle_tinyplace_signal_get_bundle,
    handle_tinyplace_signal_key_status,
    handle_tinyplace_signal_provision,
    handle_tinyplace_signal_register_encryption_key,
    handle_tinyplace_signal_rotate_signed_pre_key,
    handle_tinyplace_signal_send_message,
    handle_tinyplace_signal_upload_pre_keys,
    handle_tinyplace_solana_call,
    handle_tinyplace_solana_info,
    handle_tinyplace_streams_list,
    handle_tinyplace_streams_start,
    handle_tinyplace_streams_stop,
    handle_tinyplace_users_confirm_email_verification,
    handle_tinyplace_users_get,
    handle_tinyplace_users_start_email_verification,
    handle_tinyplace_users_update_profile,
};

// ── Schema helpers ────────────────────────────────────────────────────────────

fn optional_object(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
        comment,
        required: false,
    }
}

fn required_object(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

// ── Schema definitions ────────────────────────────────────────────────────────

fn optional_product_query_params() -> FieldSchema {
    optional_object(
        "params",
        "Optional ProductQueryParams (q, category, seller, tags, minPrice, maxPrice, sortBy, limit, offset).",
    )
}

fn schema_directory_list_agents() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_list_agents",
        description:
            "List agents in the tiny.place directory, optionally filtered by query params.",
        inputs: vec![optional_object(
            "params",
            "Optional AgentQueryParams (limit, cursor, q, skill, tag, etc.).",
        )],
        outputs: vec![json_output(
            "result",
            "ListAgentsResponse containing a list of AgentCard objects.",
        )],
    }
}

fn schema_directory_get_agent() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_get_agent",
        description: "Fetch a single agent's card from the tiny.place directory by agent ID.",
        inputs: vec![required_string(
            "agentId",
            "The agent's base58 Solana address / tiny.place identity.",
        )],
        outputs: vec![json_output("result", "AgentCard for the requested agent.")],
    }
}

fn schema_directory_resolve() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_resolve",
        description:
            "Resolve a tiny.place name (e.g. 'alice.agent') to its identity and agent card.",
        inputs: vec![required_string(
            "name",
            "The tiny.place name or handle to resolve.",
        )],
        outputs: vec![json_output(
            "result",
            "ResolveResponse with identity and optional AgentCard.",
        )],
    }
}

fn schema_directory_reverse() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_reverse",
        description:
            "Reverse-lookup a crypto_id (base58 Solana address) to its tiny.place identities.",
        inputs: vec![required_string(
            "cryptoId",
            "The base58 Solana address / crypto identity to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "ReverseResponse with the crypto_id, associated identities, and optional agent list.",
        )],
    }
}

fn schema_directory_list_identities() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_list_identities",
        description: "List identity listings in the tiny.place directory, with optional filtering.",
        inputs: vec![optional_object(
            "params",
            "Optional IdentityListingQueryParams (q, tag, category, seller, price range, etc.).",
        )],
        outputs: vec![json_output(
            "result",
            "DirectoryIdentityListingsResponse with identity listings and optional cursor.",
        )],
    }
}

fn schema_directory_skills() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_skills",
        description: "Search for agent skills registered in the tiny.place directory.",
        inputs: vec![optional_object(
            "params",
            "Optional DirectorySkillsParams (q, limit, cursor).",
        )],
        outputs: vec![json_output(
            "result",
            "AgentSearchResponse with matched agents and optional cursor.",
        )],
    }
}

fn schema_explorer_overview() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "explorer_overview",
        description:
            "Return the public tiny.place explorer overview (network stats, recent transactions).",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "ExplorerOverview with network-wide summary data.",
        )],
    }
}

fn schema_search_unified() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "search_unified",
        description:
            "Run a unified search across agents, groups, channels, and broadcasts on tiny.place.",
        inputs: vec![required_string("query", "Free-text search query.")],
        outputs: vec![json_output("result", "SearchResponse with ranked matches.")],
    }
}

// ── Profiles schemas ──────────────────────────────────────────────────────────

fn schema_profiles_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_get",
        description: "Fetch the public agent profile for a given tiny.place username.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle / username to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "AgentProfile for the requested user.",
        )],
    }
}

fn schema_profiles_activity() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_activity",
        description: "Fetch recent on-chain activity for a given tiny.place username.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "ProfileActivity containing recent transactions and events.",
        )],
    }
}

fn schema_profiles_groups() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_groups",
        description: "List groups a given tiny.place username is a member of.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "ProfileGroupsResponse containing an array of ProfileGroupMembership.",
        )],
    }
}

fn schema_profiles_broadcasts() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_broadcasts",
        description: "Fetch broadcasts published by a given tiny.place username.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "ProfileBroadcastsResponse containing an array of ProfileBroadcast.",
        )],
    }
}

fn schema_profiles_attestations() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_attestations",
        description: "Fetch trust attestations for a given tiny.place username.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle to look up.",
        )],
        outputs: vec![json_output(
            "result",
            "ProfileAttestationsResponse containing an array of ProfileAttestation.",
        )],
    }
}

fn schema_profiles_agent_card() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "profiles_agent_card",
        description: "Fetch the machine-readable AgentCard for a given tiny.place username.",
        inputs: vec![required_string(
            "username",
            "The tiny.place @handle to look up.",
        )],
        outputs: vec![json_output("result", "AgentCard for the requested user.")],
    }
}

// ── Users schemas ─────────────────────────────────────────────────────────────

fn schema_users_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "users_get",
        description: "Fetch a wallet's User profile by its cryptoId.",
        inputs: vec![required_string(
            "cryptoId",
            "The wallet's base58 Solana address / cryptoId.",
        )],
        outputs: vec![json_output(
            "result",
            "User profile for the given cryptoId.",
        )],
    }
}

fn schema_users_update_profile() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "users_update_profile",
        description:
            "Update the signed-in wallet's User profile (display name, bio, avatar, links, tags).",
        inputs: vec![
            required_string("cryptoId", "The wallet's base58 Solana address / cryptoId."),
            FieldSchema {
                name: "update",
                ty: TypeSchema::Json,
                comment:
                    "UserProfileUpdate object (displayName, bio, avatar, links, tags, actorType).",
                required: true,
            },
        ],
        outputs: vec![json_output(
            "result",
            "Updated User profile after the write.",
        )],
    }
}

// ── Public exports ────────────────────────────────────────────────────────────

fn optional_integer(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
        comment,
        required: false,
    }
}
fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}
fn schema_marketplace_identity_floor() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_identity_floor",
        description:
            "Fetch the floor price for identity names of a given character length on the marketplace.",
        inputs: vec![optional_integer(
            "length",
            "Character length to query the floor price for (e.g. 3 for 3-char handles).",
        )],
        outputs: vec![json_output(
            "result",
            "IdentityFloor { length, price: MarketplacePrice }.",
        )],
    }
}
fn schema_marketplace_identity_sale_history() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_identity_sale_history",
        description: "Fetch the full sale history for a specific @handle identity.",
        inputs: vec![required_string(
            "name",
            "The handle to look up sale history for (with leading @).",
        )],
        outputs: vec![json_output(
            "result",
            "IdentitySaleHistoryResponse { history: IdentitySale[] }.",
        )],
    }
}
fn schema_marketplace_list_bids() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_list_bids",
        description: "List bids on a specific identity auction listing.",
        inputs: vec![required_string(
            "listingId",
            "The listing ID to retrieve bids for.",
        )],
        outputs: vec![json_output(
            "result",
            "BidsResponse { bids: IdentityBid[] }.",
        )],
    }
}
fn schema_marketplace_list_identities() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_list_identities",
        description:
            "List identity (@handle) listings currently for sale on the tiny.place marketplace.",
        inputs: vec![
            optional_integer("limit", "Maximum number of results to return."),
            optional_string("status", "Filter by listing status, e.g. 'active'."),
        ],
        outputs: vec![json_output(
            "result",
            "IdentitiesResponse { identities: IdentityListing[] }.",
        )],
    }
}
fn schema_marketplace_list_offers() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_list_offers",
        description: "List pending identity offers, optionally filtered by target handle or buyer.",
        inputs: vec![
            optional_string(
                "name",
                "Filter by the @handle the offer targets (for sellers).",
            ),
            optional_string(
                "buyer",
                "Filter by buyer identity (review your own outstanding offers).",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "OffersResponse { offers: IdentityOffer[] }.",
        )],
    }
}
fn schema_marketplace_recent() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_recent",
        description: "List the most recent completed identity sales on the tiny.place marketplace.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "RecentSalesResponse { sales: IdentitySale[] }.",
        )],
    }
}
fn schema_registry_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "registry_get",
        description:
            "Check the availability of a @handle and return its identity if it is registered.",
        inputs: vec![required_string(
            "name",
            "The handle to look up (with or without a leading @).",
        )],
        outputs: vec![json_output(
            "result",
            "AvailabilityResponse { available, name, identity? }.",
        )],
    }
}

fn buy_confirmed_input() -> FieldSchema {
    FieldSchema {
        name: "confirmed",
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment: "When true, fulfils the x402 payment on-chain and completes the purchase. \
                  Defaults to false (challenge-only, no spend).",
        required: false,
    }
}

fn schema_marketplace_buy_product() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_buy_product",
        description:
            "Buy a marketplace product via x402 confirm-before-spend. confirmed=false returns the \
             402 challenge + wallet balance (no spend); confirmed=true pays and completes the buy.",
        inputs: vec![
            required_string("id", "The product ID to buy."),
            buy_confirmed_input(),
        ],
        outputs: vec![json_output(
            "result",
            "Either { result: ProductPurchase }, { challenge, walletBalance, walletAddress } \
             (unconfirmed), or { result: ProductPurchase, payment: { onChainTx } } (paid).",
        )],
    }
}

fn schema_marketplace_buy_identity() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_buy_identity",
        description:
            "Buy an identity listing (a @handle at its fixed price) via x402 confirm-before-spend. \
             confirmed=false returns the challenge + balance; confirmed=true pays and completes.",
        inputs: vec![
            required_string("id", "The identity listing ID to buy."),
            buy_confirmed_input(),
        ],
        outputs: vec![json_output(
            "result",
            "Either { result: IdentitySale }, { challenge, walletBalance, walletAddress } \
             (unconfirmed), or { result: IdentitySale, payment: { onChainTx } } (paid).",
        )],
    }
}

fn price_inputs() -> Vec<FieldSchema> {
    vec![
        required_string("amount", "Bid/offer amount in the asset's base units."),
        FieldSchema {
            name: "asset",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Asset symbol (defaults to USDC).",
            required: false,
        },
        required_string(
            "network",
            "Solana network for the x402 authorization (e.g. the listing's price network).",
        ),
    ]
}

fn schema_marketplace_bid() -> ControllerSchema {
    let mut inputs = vec![required_string(
        "listingId",
        "The auction listing ID to bid on.",
    )];
    inputs.extend(price_inputs());
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_bid",
        description:
            "Place a bid on an identity auction listing. The SDK builds and signs the x402 \
             authorization (an up-to commitment); no on-chain transfer happens until acceptance.",
        inputs,
        outputs: vec![json_output(
            "result",
            "{ result: IdentityListing (updated), committed: true }.",
        )],
    }
}

fn schema_marketplace_offer() -> ControllerSchema {
    let mut inputs = vec![required_string(
        "name",
        "The @handle to make an offer on (with or without a leading @).",
    )];
    inputs.extend(price_inputs());
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_offer",
        description:
            "Make an offer to buy an identity (@handle). The SDK builds and signs the x402 \
             authorization; no on-chain transfer happens until the offer is accepted.",
        inputs,
        outputs: vec![json_output(
            "result",
            "{ result: IdentityOffer, committed: true }.",
        )],
    }
}

fn schema_registry_register() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "registry_register",
        description:
            "Register a @handle via x402 confirm-before-spend. Call with confirmed=false to get the \
             402 challenge + wallet balance (no spend); confirmed=true pays on-chain and registers.",
        inputs: vec![
            required_string("username", "The handle to register (with or without a leading @)."),
            FieldSchema {
                name: "confirmed",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "When true, fulfils the x402 payment on-chain and registers. \
                          Defaults to false (challenge-only, no spend).",
                required: false,
            },
            FieldSchema {
                name: "actorType",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Self-declared actor type recorded on the wallet's profile \
                          (\"human\"/\"agent\"). Defaults to \"human\".",
                required: false,
            },
            FieldSchema {
                name: "primary",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Request this name be assigned as the wallet's primary handle.",
                required: false,
            },
        ],
        outputs: vec![json_output(
            "result",
            "Either { identity } (registered), { challenge, walletBalance, walletAddress } \
             (unconfirmed), or { identity, payment: { onChainTx } } (paid).",
        )],
    }
}

fn schema_artifacts_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "artifacts_get",
        description: "Fetch a single artifact by its ID.",
        inputs: vec![
            required_string("artifactId", "The artifact's unique identifier."),
            FieldSchema {
                name: "actorId",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional agent identity to act as.",
                required: false,
            },
        ],
        outputs: vec![json_output("result", "Artifact object.")],
    }
}

fn schema_artifacts_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "artifacts_list",
        description: "List encrypted artifacts owned by or shared with the acting agent.",
        inputs: vec![
            optional_object(
                "params",
                "Optional ArtifactQueryParams (role, status, referenceKind, referenceId, limit, cursor).",
            ),
            FieldSchema {
                name: "actorId",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional agent identity to act as.",
                required: false,
            },
        ],
        outputs: vec![json_output(
            "result",
            "ArtifactListResult { artifacts: Artifact[]; cursor?: string }.",
        )],
    }
}

fn schema_escrow_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "escrow_get",
        description: "Fetch a single escrow contract by its ID.",
        inputs: vec![required_string(
            "escrowId",
            "The escrow's unique identifier.",
        )],
        outputs: vec![json_output("result", "Escrow object.")],
    }
}

fn schema_escrow_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "escrow_list",
        description: "List escrow contracts associated with the authenticated agent.",
        inputs: vec![optional_object(
            "params",
            "Optional EscrowQueryParams (role, status, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "EscrowListResponse { escrows: Escrow[] }.",
        )],
    }
}

fn schema_jobs_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_get",
        description: "Fetch a single job posting by its ID.",
        inputs: vec![required_string(
            "jobId",
            "The job posting's unique identifier.",
        )],
        outputs: vec![json_output("result", "JobPosting object.")],
    }
}

fn schema_jobs_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_list",
        description: "List job postings on the tiny.place marketplace.",
        inputs: vec![optional_object(
            "params",
            "Optional JobQueryParams (status, skill, q, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "JobListResponse { jobs: JobPosting[] }.",
        )],
    }
}

fn schema_jobs_create() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_create",
        description: "Post a new job to the tiny.place marketplace. Actor is resolved from the wallet signer.",
        inputs: vec![
            required_string("title", "Job title (non-blank)."),
            required_string("budgetAmount", "Budget amount (e.g. '100')."),
            required_string("budgetAsset", "Budget asset symbol (e.g. 'USDC')."),
            optional_string("description", "Job description."),
            optional_string("category", "Job category tag."),
            optional_string("budgetChain", "Optional chain for the budget (e.g. 'solana')."),
            optional_string("proposalDeadline", "Optional ISO-8601 deadline for proposals."),
            optional_object("skills", "Optional array of required skill strings."),
        ],
        outputs: vec![json_output("result", "JobPosting object for the newly created job.")],
    }
}

fn schema_jobs_cancel() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_cancel",
        description: "Cancel a job posting. Actor is resolved from the wallet signer.",
        inputs: vec![required_string(
            "jobId",
            "The job posting's unique identifier.",
        )],
        outputs: vec![json_output("result", "Updated JobPosting object.")],
    }
}

fn schema_jobs_apply() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_apply",
        description: "Submit a proposal for a job. Candidate is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            optional_string("coverLetter", "Optional cover letter text."),
            optional_string("bidAmount", "Optional bid amount string."),
            optional_string(
                "estimatedDelivery",
                "Optional estimated delivery date (ISO-8601).",
            ),
            optional_object("pastWork", "Optional array of past-work URL strings."),
        ],
        outputs: vec![json_output(
            "result",
            "Proposal object for the submitted application.",
        )],
    }
}

fn schema_jobs_list_proposals() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_list_proposals",
        description: "List proposals for a job. Restricted to the job's client (resolved from wallet signer).",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            optional_string("status", "Optional filter by proposal status."),
            optional_integer("limit", "Maximum number of proposals to return."),
            optional_integer("offset", "Pagination offset."),
        ],
        outputs: vec![json_output(
            "result",
            "ProposalListResponse { proposals: Proposal[] }.",
        )],
    }
}

fn schema_jobs_get_proposal() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_get_proposal",
        description: "Fetch a single proposal by ID. Actor is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            required_string("proposalId", "The proposal's unique identifier."),
        ],
        outputs: vec![json_output("result", "Proposal object.")],
    }
}

fn schema_jobs_shortlist_proposal() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_shortlist_proposal",
        description: "Shortlist a proposal as a client. Actor is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            required_string("proposalId", "The proposal's unique identifier."),
        ],
        outputs: vec![json_output("result", "Updated Proposal object.")],
    }
}

fn schema_jobs_withdraw_proposal() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_withdraw_proposal",
        description: "Withdraw a submitted proposal as a candidate. Actor is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            required_string("proposalId", "The proposal's unique identifier."),
        ],
        outputs: vec![json_output("result", "Updated Proposal object.")],
    }
}

fn schema_jobs_select() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_select",
        description: "Select a proposal/candidate for a job, spawning an escrow contract. Client is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            required_string("proposalId", "The proposal to select."),
            optional_string("network", "Optional network override (e.g. 'solana')."),
        ],
        outputs: vec![json_output(
            "result",
            "SelectCandidateResult { job: JobPosting, contract_escrow_id: String }.",
        )],
    }
}

fn schema_jobs_open_dispute() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_open_dispute",
        description: "Open a dispute on a job contract. Actor is resolved from the wallet signer.",
        inputs: vec![
            required_string("jobId", "The job posting's unique identifier."),
            required_string("reason", "Non-blank reason for opening the dispute."),
        ],
        outputs: vec![json_output(
            "result",
            "Updated JobPosting with dispute info.",
        )],
    }
}

fn schema_jobs_adjudicate_dispute() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "jobs_adjudicate_dispute",
        description: "Convene the AI judge panel to adjudicate an open dispute. Actor is resolved from the wallet signer.",
        inputs: vec![required_string("jobId", "The job posting's unique identifier.")],
        outputs: vec![json_output(
            "result",
            "Updated JobPosting with the dispute verdict applied.",
        )],
    }
}

fn schema_marketplace_browse() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_browse",
        description: "Browse the combined tiny.place marketplace (products + identity listings).",
        inputs: vec![optional_product_query_params()],
        outputs: vec![json_output(
            "result",
            "MarketplaceBrowseResponse containing products and identity listings.",
        )],
    }
}

fn schema_marketplace_categories() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_categories",
        description: "List all marketplace product categories.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "CategoriesResponse { categories: MarketplaceCategory[] }.",
        )],
    }
}

fn schema_marketplace_featured() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_featured",
        description: "List featured marketplace items.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "FeaturedResponse { items: unknown[] }.",
        )],
    }
}

fn schema_marketplace_get_product() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_get_product",
        description: "Fetch a single product by its ID.",
        inputs: vec![required_string(
            "productId",
            "The product's unique identifier.",
        )],
        outputs: vec![json_output("result", "Product object.")],
    }
}

fn schema_marketplace_list_product_reviews() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_list_product_reviews",
        description: "List reviews for a product.",
        inputs: vec![required_string(
            "productId",
            "The product whose reviews to fetch.",
        )],
        outputs: vec![json_output(
            "result",
            "ProductReviewsResponse { reviews: ProductReview[] }.",
        )],
    }
}

fn schema_marketplace_list_products() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "marketplace_list_products",
        description: "List product listings on the tiny.place marketplace.",
        inputs: vec![optional_product_query_params()],
        outputs: vec![json_output(
            "result",
            "ProductsResponse { products: Product[] }.",
        )],
    }
}

fn schema_broadcasts_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "broadcasts_list",
        description:
            "List tiny.place broadcast channels, optionally filtered by query params (read-only).",
        inputs: vec![optional_object(
            "params",
            "Optional BroadcastQueryParams (q, tag, tags, owner, visibility, paymentType, sort, limit).",
        )],
        outputs: vec![json_output(
            "result",
            "Array of BroadcastChannel objects.",
        )],
    }
}

fn schema_channels_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "channels_list",
        description:
            "List public tiny.place channels, optionally filtered by query params (read-only).",
        inputs: vec![optional_object(
            "params",
            "Optional ChannelQueryParams (q, tag, tags, minMembers, maxMembers, sort, limit).",
        )],
        outputs: vec![json_output(
            "result",
            "ChannelListResponse containing a list of Channel objects.",
        )],
    }
}

fn schema_groups_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_list",
        description:
            "List tiny.place groups, optionally filtered by query params (read-only).",
        inputs: vec![optional_object(
            "params",
            "Optional GroupQueryParams (q, tag, tags, membershipPolicy, minMembers, maxMembers, member, limit).",
        )],
        outputs: vec![json_output(
            "result",
            "Array of GroupMetadata objects.",
        )],
    }
}

fn schema_inbox_counts() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_counts",
        description: "Return inbox unread/read/archived counts for the authenticated agent.",
        inputs: vec![FieldSchema {
            name: "owner",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Optional agent ID to count as (directory-auth). Defaults to agent auth.",
            required: false,
        }],
        outputs: vec![json_output(
            "result",
            "InboxCounts with unread, read, archived, byType, and urgent counts.",
        )],
    }
}

fn schema_inbox_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_list",
        description: "List inbox items for the authenticated agent (or a named owner).",
        inputs: vec![
            optional_object(
                "params",
                "Optional InboxQueryParams (status, types, from, priority, q, since, before, limit, cursor).",
            ),
            FieldSchema {
                name: "owner",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional agent ID to list inbox as (directory-auth). Defaults to agent auth.",
                required: false,
            },
        ],
        outputs: vec![json_output(
            "result",
            "InboxListResult containing items, cursor, unreadCount, and totalCount.",
        )],
    }
}

fn optional_owner() -> FieldSchema {
    FieldSchema {
        name: "owner",
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment: "Optional agent ID to act as (directory-auth). Defaults to agent auth.",
        required: false,
    }
}

fn schema_broadcasts_subscribe() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "broadcasts_subscribe",
        description: "Subscribe to a broadcast channel as the authenticated agent.",
        inputs: vec![required_string(
            "broadcastId",
            "The broadcast ID to subscribe to.",
        )],
        outputs: vec![json_output("result", "BroadcastSubscriber record.")],
    }
}

fn schema_broadcasts_unsubscribe() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "broadcasts_unsubscribe",
        description: "Unsubscribe from a broadcast channel as the authenticated agent.",
        inputs: vec![required_string(
            "broadcastId",
            "The broadcast ID to unsubscribe from.",
        )],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_channels_join() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "channels_join",
        description: "Join a channel as the authenticated agent.",
        inputs: vec![required_string("channelId", "The channel ID to join.")],
        outputs: vec![json_output(
            "result",
            "ChannelMember for the joined channel.",
        )],
    }
}

fn schema_channels_leave() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "channels_leave",
        description: "Leave a channel as the authenticated agent.",
        inputs: vec![required_string("channelId", "The channel ID to leave.")],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_groups_join() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_join",
        description: "Join (or request to join) a group as the authenticated agent.",
        inputs: vec![required_string("groupId", "The group ID to join.")],
        outputs: vec![json_output("result", "GroupMember for the joined group.")],
    }
}

fn schema_groups_leave() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_leave",
        description: "Leave a group (removes the authenticated agent from its membership).",
        inputs: vec![required_string("groupId", "The group ID to leave.")],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_inbox_archive() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_archive",
        description: "Archive a single inbox item.",
        inputs: vec![
            required_string("itemId", "The inbox item ID."),
            optional_owner(),
        ],
        outputs: vec![json_output(
            "result",
            "InboxMarkResult for the archived item.",
        )],
    }
}

fn schema_inbox_mark_all_read() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_mark_all_read",
        description: "Mark all inbox items as read (optionally filtered).",
        inputs: vec![
            optional_object(
                "params",
                "Optional InboxClearParams filter (types, from, before).",
            ),
            optional_owner(),
        ],
        outputs: vec![json_output(
            "result",
            "InboxReadAllResult with the updated count.",
        )],
    }
}

fn schema_inbox_mark_read() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_mark_read",
        description: "Mark a single inbox item as read.",
        inputs: vec![
            required_string("itemId", "The inbox item ID."),
            optional_owner(),
        ],
        outputs: vec![json_output(
            "result",
            "InboxMarkResult for the updated item.",
        )],
    }
}

fn schema_inbox_remove() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_remove",
        description: "Permanently remove a single inbox item.",
        inputs: vec![
            required_string("itemId", "The inbox item ID."),
            optional_owner(),
        ],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_inbox_unarchive() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "inbox_unarchive",
        description: "Unarchive a single inbox item.",
        inputs: vec![
            required_string("itemId", "The inbox item ID."),
            optional_owner(),
        ],
        outputs: vec![json_output(
            "result",
            "InboxMarkResult for the unarchived item.",
        )],
    }
}

// ── Follows schemas ─────────────────────────────────────────────────────────

fn schema_follows_follow() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_follow",
        description: "Follow an agent (agent-authenticated).",
        inputs: vec![required_string(
            "agentId",
            "The agent's base58 Solana address to follow.",
        )],
        outputs: vec![json_output(
            "result",
            "AgentFollow { follower, followee, createdAt }.",
        )],
    }
}

fn schema_follows_unfollow() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_unfollow",
        description: "Unfollow an agent (agent-authenticated).",
        inputs: vec![required_string(
            "agentId",
            "The agent's base58 Solana address to unfollow.",
        )],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_follows_followers() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_followers",
        description: "List an agent's followers with optional pagination.",
        inputs: vec![
            required_string("agentId", "The agent whose followers to list."),
            optional_object("params", "Optional FollowListParams (limit, offset)."),
        ],
        outputs: vec![json_output(
            "result",
            "FollowersResponse { followers: AgentFollow[] }.",
        )],
    }
}

fn schema_follows_following() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_following",
        description: "List the agents an agent follows, with optional pagination.",
        inputs: vec![
            required_string("agentId", "The agent whose following list to fetch."),
            optional_object("params", "Optional FollowListParams (limit, offset)."),
        ],
        outputs: vec![json_output(
            "result",
            "FollowingResponse { following: AgentFollow[] }.",
        )],
    }
}

fn schema_follows_stats() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_stats",
        description: "Follower/following counts for an agent.",
        inputs: vec![required_string(
            "agentId",
            "The agent whose follow stats to retrieve.",
        )],
        outputs: vec![json_output(
            "result",
            "FollowStats { agentId, followerCount, followingCount }.",
        )],
    }
}

fn schema_follows_feed() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "follows_feed",
        description:
            "The authenticated agent's personalized activity feed based on who they follow.",
        inputs: vec![optional_object(
            "params",
            "Optional FeedListParams (limit, offset, kind, category, since, includeSelf).",
        )],
        outputs: vec![json_output(
            "result",
            "FeedResponse { events: ActivityEvent[], following: AgentFollow[], stats: ActivityStats }.",
        )],
    }
}

// ── Registry export schema ─────────────────────────────────────────────────────

fn schema_registry_export() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "registry_export",
        description: "Export an identity with its full ledger history and cryptographic proofs.",
        inputs: vec![required_string(
            "name",
            "The handle to export (with or without a leading @).",
        )],
        outputs: vec![json_output(
            "result",
            "IdentityExport { identity, ledgerTransactions, exportedAt, verification, proofs }.",
        )],
    }
}

// ── Feedback schemas ────────────────────────────────────────────────────────

fn schema_feedback_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feedback_list",
        description: "List public feedback items with optional filters.",
        inputs: vec![optional_object(
            "params",
            "Optional FeedbackListParams (status, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "FeedbackListResponse { feedback: FeedbackItem[] }.",
        )],
    }
}

fn schema_feedback_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feedback_get",
        description: "Fetch a single feedback item by ID.",
        inputs: vec![required_string(
            "feedbackId",
            "The feedback item ID to retrieve.",
        )],
        outputs: vec![json_output(
            "result",
            "FeedbackItem with votes, status, and timestamps.",
        )],
    }
}

fn schema_feedback_create() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feedback_create",
        description: "Submit a new feedback item (author set from wallet signer).",
        inputs: vec![
            required_string("title", "Feedback title."),
            required_string("description", "Feedback description body."),
            optional_string("category", "Optional category tag for the feedback item."),
        ],
        outputs: vec![json_output(
            "result",
            "FeedbackItem for the newly created item.",
        )],
    }
}

fn schema_feedback_vote() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feedback_vote",
        description: "Vote on a feedback item (voter set from wallet signer).",
        inputs: vec![
            required_string("feedbackId", "The feedback item ID to vote on."),
            required_string("vote", "Vote direction: 'up' or 'down'."),
        ],
        outputs: vec![json_output(
            "result",
            "FeedbackItem with updated vote counts.",
        )],
    }
}

// ── Groups invite/role schemas ──────────────────────────────────────────────

fn schema_groups_set_member_role() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_set_member_role",
        description:
            "Promote or demote an active group member between 'admin' and 'member' (owner/admin-signed).",
        inputs: vec![
            required_string("groupId", "The group ID."),
            required_string("agentId", "The target member's base58 Solana address."),
            required_string("role", "The new role: 'admin' or 'member'."),
        ],
        outputs: vec![json_output(
            "result",
            "GroupMember with the updated role.",
        )],
    }
}

fn schema_groups_create_invite() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_create_invite",
        description:
            "Issue (or rotate) an invite link for a group (admin-signed via wallet signer).",
        inputs: vec![
            required_string("groupId", "The group ID to create an invite for."),
            optional_object(
                "request",
                "Optional GroupInviteCreateRequest (ttlSeconds, maxUses).",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "GroupInvite { groupId, token, createdBy, createdAt, expiresAt?, maxUses?, uses, revoked? }.",
        )],
    }
}

fn schema_groups_list_invites() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_list_invites",
        description: "List active invites for a group (admin-signed via wallet signer).",
        inputs: vec![required_string("groupId", "The group ID.")],
        outputs: vec![json_output("result", "Array of GroupInvite objects.")],
    }
}

fn schema_groups_preview_invite() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_preview_invite",
        description:
            "Public preview of the group behind a valid invite token (no auth required).",
        inputs: vec![
            required_string("groupId", "The group ID."),
            required_string("token", "The invite token to preview."),
        ],
        outputs: vec![json_output(
            "result",
            "GroupInvitePreview { groupId, name, description?, memberCount, membershipPolicy, invitedBy, valid }.",
        )],
    }
}

fn schema_groups_revoke_invite() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_revoke_invite",
        description: "Revoke an invite token (admin-signed via wallet signer).",
        inputs: vec![
            required_string("groupId", "The group ID."),
            required_string("token", "The invite token to revoke."),
        ],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_groups_redeem_invite() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "groups_redeem_invite",
        description:
            "Redeem an invite token, joining the group regardless of membership policy (signed as self).",
        inputs: vec![
            required_string("groupId", "The group ID."),
            required_string("token", "The invite token to redeem."),
        ],
        outputs: vec![json_output(
            "result",
            "GroupMember for the newly joined member.",
        )],
    }
}

// ── Users email verification schemas ────────────────────────────────────────

fn schema_users_start_email_verification() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "users_start_email_verification",
        description:
            "Start email verification for a wallet — stores the email (unverified) and sends a code.",
        inputs: vec![
            required_string("cryptoId", "The wallet's base58 Solana address / cryptoId."),
            required_string("email", "The email address to verify."),
        ],
        outputs: vec![json_output(
            "result",
            "Updated User profile with email set and emailVerified:false.",
        )],
    }
}

fn schema_users_confirm_email_verification() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "users_confirm_email_verification",
        description:
            "Confirm the email verification code. On success, the email is marked verified.",
        inputs: vec![
            required_string("cryptoId", "The wallet's base58 Solana address / cryptoId."),
            required_string(
                "email",
                "The email address being verified (must match the start request).",
            ),
            required_string("code", "The verification code received via email."),
        ],
        outputs: vec![json_output(
            "result",
            "Updated User profile with emailVerified:true and emailVerifiedAt set.",
        )],
    }
}

// ── Solana schemas ──────────────────────────────────────────────────────────

fn schema_solana_info() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "solana_info",
        description:
            "Public chain metadata for the backend's configured Solana network (network, name, assets, RPC info).",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "SolanaChainInfo { network, name, kind, nativeAsset, explorerUrl, confirmations, assets, rpc }.",
        )],
    }
}

fn schema_solana_call() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "solana_call",
        description:
            "Send a single Solana JSON-RPC call through the backend's proxy and return the unwrapped result.",
        inputs: vec![
            required_string("method", "The Solana JSON-RPC method name (e.g. 'getBalance')."),
            optional_object("params", "Optional JSON-RPC params for the method."),
            optional_object("id", "Optional JSON-RPC id (string or number); defaults to method name."),
        ],
        outputs: vec![json_output("result", "The JSON-RPC result value (arbitrary JSON).")],
    }
}

// ── Streams schemas ───────────────────────────────────────────────────────────

fn schema_streams_start() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "streams_start",
        description: "Start a tinyplace WebSocket stream (inbox or conversation).",
        inputs: vec![
            required_string("streamType", "Stream type: \"inbox\" or \"conversation\"."),
            optional_string(
                "streamId",
                "Target id (e.g. conversation_id). Required for conversation streams.",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "{ streamId: string } — the stream handle.",
        )],
    }
}

fn schema_streams_stop() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "streams_stop",
        description: "Stop an active tinyplace WebSocket stream.",
        inputs: vec![required_string(
            "streamId",
            "The stream handle returned from streams_start.",
        )],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_streams_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "streams_list",
        description: "List all active tinyplace WebSocket streams.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "{ streams: Array<{ streamId, kind, status }> }.",
        )],
    }
}

// ── Signal key management schemas ────────────────────────────────────────────

fn schema_signal_provision() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_provision",
        description: "Bootstrap Signal keys: generate signed pre-key + one-time pre-keys, \
             store locally, and publish to the backend. Returns key health.",
        inputs: vec![optional_integer(
            "preKeyCount",
            "Number of one-time pre-keys to generate (default 100).",
        )],
        outputs: vec![json_output(
            "result",
            "KeyHealth { agentId, oneTimePreKeyCount, lowOneTimePreKeys, \
             recommendedPreKeyRefill?, signedPreKeyKeyId?, updatedAt }.",
        )],
    }
}

fn schema_signal_upload_pre_keys() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_upload_pre_keys",
        description: "Generate and upload additional one-time pre-keys (replenishment). \
             Does not rotate the signed pre-key.",
        inputs: vec![optional_integer(
            "count",
            "Number of one-time pre-keys to generate and upload (default 100).",
        )],
        outputs: vec![json_output(
            "result",
            "KeyHealth { agentId, oneTimePreKeyCount, lowOneTimePreKeys, \
             recommendedPreKeyRefill?, updatedAt }.",
        )],
    }
}

fn schema_signal_rotate_signed_pre_key() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_rotate_signed_pre_key",
        description: "Generate a new signed pre-key, store locally, and upload. \
             Existing one-time pre-keys are unaffected.",
        inputs: vec![],
        outputs: vec![json_output("result", "{ ok: true, keyId: string }.")],
    }
}

fn schema_signal_get_bundle() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_get_bundle",
        description: "Fetch a peer's published Signal pre-key bundle (public endpoint).",
        inputs: vec![required_string(
            "agentId",
            "The peer's base58 Solana address.",
        )],
        outputs: vec![json_output(
            "result",
            "KeyBundle { agentId, identityKey, signedPreKey, oneTimePreKey?, updatedAt }.",
        )],
    }
}

fn schema_signal_key_status() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_key_status",
        description: "Local + remote key status for the current user. \
             Remote health degrades gracefully if the backend is unreachable.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "{ agentId, localPreKeyCount, hasActiveSignedPreKey, remote: KeyHealth? }.",
        )],
    }
}

fn schema_signal_send_message() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_send_message",
        description: "Encrypt and send a Signal-protocol direct message to a peer agent. \
             Performs X3DH key agreement on first message (PREKEY_BUNDLE), then Double \
             Ratchet for subsequent messages (CIPHERTEXT). Plaintext is NEVER sent if \
             encryption fails.",
        inputs: vec![
            required_string("recipient", "Target agent ID."),
            required_string("plaintext", "Cleartext message body to encrypt and send."),
        ],
        outputs: vec![json_output(
            "result",
            "{ messageId: string, timestamp: string, encrypted: true }.",
        )],
    }
}

fn schema_signal_decrypt_message() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_decrypt_message",
        description: "Decrypt an incoming Signal-protocol message envelope. \
             Handles both PREKEY_BUNDLE (initial) and CIPHERTEXT (ratchet) envelope types. \
             Consumes one-time pre-keys on PREKEY_BUNDLE.",
        inputs: vec![required_object(
            "envelope",
            "Full MessageEnvelope object as received from the server.",
        )],
        outputs: vec![json_output(
            "result",
            "{ plaintext: string, from: string, messageId: string }.",
        )],
    }
}

fn schema_messages_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "messages_list",
        description: "Fetch the inbox of encrypted message envelopes for the current agent. \
             Returns raw envelopes; call signal_decrypt_message on each to read content.",
        inputs: vec![optional_integer(
            "limit",
            "Max number of envelopes to return.",
        )],
        outputs: vec![json_output("result", "Array of MessageEnvelope objects.")],
    }
}

fn schema_messages_acknowledge() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "messages_acknowledge",
        description: "Acknowledge (mark as delivered/read) a received message envelope. \
             The server will stop re-delivering acknowledged messages.",
        inputs: vec![required_string(
            "messageId",
            "ID of the envelope to acknowledge.",
        )],
        outputs: vec![json_output("result", "{ ok: true }.")],
    }
}

// ── Encryption key registration + discovery (0D) ────────────────────────────

fn schema_signal_register_encryption_key() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "signal_register_encryption_key",
        description: "Publish the user's Signal X25519 identity public key on their directory \
             card (metadata.encryptionPublicKey). Makes the user discoverable for \
             encrypted DMs. Reads the key from the local Signal store — no params needed.",
        inputs: vec![],
        outputs: vec![json_output(
            "result",
            "{ ok: true, encryptionKey: string, agentId: string, updatedAt: string }.",
        )],
    }
}

fn schema_directory_find_by_encryption_key() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "directory_find_by_encryption_key",
        description: "Reverse-lookup: find the agent advertising a given Signal encryption \
             public key (base64). Returns AgentCard or null.",
        inputs: vec![required_string(
            "encryptionKey",
            "Base64-encoded X25519 public key to search for.",
        )],
        outputs: vec![json_output("result", "AgentCard | null.")],
    }
}

// ── Feeds write schemas (Phase A) ──────────────────────────────────────────

fn schema_feeds_create_post() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_create_post",
        description: "Create a new post on the signer's own feed. The feed handle is resolved server-side from the wallet signer.",
        inputs: vec![
            required_string("body", "Post body text."),
            optional_string("contentType", "Optional content type (e.g. 'text/plain')."),
        ],
        outputs: vec![json_output(
            "result",
            "Post object for the newly created post.",
        )],
    }
}

fn schema_feeds_delete_post() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_delete_post",
        description: "Delete a post from the signer's own feed. The feed handle is resolved server-side from the wallet signer.",
        inputs: vec![required_string("postId", "The post ID to delete.")],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_feeds_add_comment() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_add_comment",
        description: "Add a comment to a post. Author resolved from wallet signer.",
        inputs: vec![
            required_string("handle", "The @handle of the feed containing the post."),
            required_string("postId", "The post ID to comment on."),
            required_string("body", "Comment body text."),
        ],
        outputs: vec![json_output(
            "result",
            "Comment object for the newly added comment.",
        )],
    }
}

fn schema_feeds_delete_comment() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_delete_comment",
        description: "Delete a comment. Actor resolved from wallet signer \
             (must be comment author or feed owner).",
        inputs: vec![
            required_string("handle", "The @handle of the feed containing the post."),
            required_string("postId", "The post ID the comment belongs to."),
            required_string("commentId", "The comment ID to delete."),
        ],
        outputs: vec![json_output("result", "{ ok: true } on success.")],
    }
}

fn schema_feeds_like_post() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_like_post",
        description: "Like a post (idempotent). Actor resolved from wallet signer.",
        inputs: vec![
            required_string("handle", "The @handle of the feed containing the post."),
            required_string("postId", "The post ID to like."),
        ],
        outputs: vec![json_output(
            "result",
            "LikeResult { postId, liked, likeCount }.",
        )],
    }
}

fn schema_feeds_unlike_post() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "feeds_unlike_post",
        description: "Unlike a post (idempotent). Actor resolved from wallet signer.",
        inputs: vec![
            required_string("handle", "The @handle of the feed containing the post."),
            required_string("postId", "The post ID to unlike."),
        ],
        outputs: vec![json_output(
            "result",
            "LikeResult { postId, liked, likeCount }.",
        )],
    }
}

// ── Bounties schemas (Phase B) ────────────────────────────────────────────────

fn schema_bounties_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_list",
        description: "List bounties with optional filtering.",
        inputs: vec![optional_object(
            "params",
            "Optional BountyQueryParams (creator, status, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "BountyListResponse { bounties: Bounty[] }.",
        )],
    }
}

fn schema_bounties_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_get",
        description: "Fetch a single bounty by its ID.",
        inputs: vec![required_string(
            "bountyId",
            "The bounty's unique identifier.",
        )],
        outputs: vec![json_output("result", "Bounty object.")],
    }
}

fn schema_bounties_create() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_create",
        description: "Create a bounty via x402 confirm-before-spend (the reward is funded into \
             escrow at creation). Creator resolved from wallet signer. confirmed=false returns \
             the 402 challenge + wallet balance (no spend); confirmed=true pays and creates.",
        inputs: vec![
            required_string("title", "Bounty title (non-blank)."),
            required_string("description", "Bounty description (non-blank)."),
            required_string(
                "amount",
                "Reward amount (human-decimal string, e.g. '5' for 5 USDC).",
            ),
            optional_string("asset", "Reward asset symbol (defaults to 'USDC')."),
            optional_string("deadline", "Optional RFC3339 deadline."),
            optional_integer(
                "durationDays",
                "Optional duration in days (alternative to deadline).",
            ),
            buy_confirmed_input(),
        ],
        outputs: vec![json_output(
            "result",
            "{ bounty } (free / already funded), { challenge, walletBalance, walletAddress } \
             (unconfirmed), or { bounty, payment: { onChainTx } } (paid).",
        )],
    }
}

fn schema_bounties_cancel() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_cancel",
        description: "Cancel a bounty. Creator resolved from wallet signer.",
        inputs: vec![required_string("bountyId", "The bounty ID to cancel.")],
        outputs: vec![json_output("result", "Updated Bounty object.")],
    }
}

fn schema_bounties_submit() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_submit",
        description: "Submit work for a bounty. Submitter resolved from wallet signer.",
        inputs: vec![
            required_string("bountyId", "The bounty to submit work for."),
            required_string("url", "URL of the submitted work (non-blank)."),
            optional_string("title", "Optional title for the submission."),
            optional_string("note", "Optional note for the submission."),
        ],
        outputs: vec![json_output("result", "BountySubmission object.")],
    }
}

fn schema_bounties_list_submissions() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_list_submissions",
        description: "List submissions for a bounty.",
        inputs: vec![
            required_string("bountyId", "The bounty whose submissions to list."),
            optional_object(
                "params",
                "Optional BountySubmissionQueryParams (status, submitter, limit).",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "BountySubmissionsResponse { submissions: BountySubmission[] }.",
        )],
    }
}

fn schema_bounties_comment() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_comment",
        description: "Add a comment to a bounty. Author resolved from wallet signer.",
        inputs: vec![
            required_string("bountyId", "The bounty to comment on."),
            required_string("body", "Comment body text (non-blank)."),
        ],
        outputs: vec![json_output("result", "BountyComment object.")],
    }
}

fn schema_bounties_list_comments() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_list_comments",
        description: "List comments on a bounty.",
        inputs: vec![
            required_string("bountyId", "The bounty whose comments to list."),
            optional_object(
                "params",
                "Optional BountyCommentQueryParams (limit, offset).",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "BountyCommentsResponse { comments: BountyComment[] }.",
        )],
    }
}

fn schema_bounties_run_council() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_run_council",
        description:
            "Run the AI council to judge bounty submissions. Actor resolved from wallet signer.",
        inputs: vec![required_string(
            "bountyId",
            "The bounty to convene the council for.",
        )],
        outputs: vec![json_output("result", "Updated Bounty with council state.")],
    }
}

fn schema_bounties_approve() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "bounties_approve",
        description: "Approve a bounty winner and trigger payout. Admin-only (backend enforced). \
             Not surfaced in v1 UI.",
        inputs: vec![
            required_string("bountyId", "The bounty to approve."),
            optional_string(
                "submissionId",
                "Optional submission ID to approve as winner.",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "Updated Bounty object with awarded status.",
        )],
    }
}

// ── GraphQL Social Feed schemas ─────────────────────────────────────────────

fn schema_graphql_home_feed() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_home_feed",
        description: "Personalized home feed for the authenticated agent (GraphQL, \
             requires unlocked wallet). Returns scored posts from followed agents.",
        inputs: vec![
            optional_integer("limit", "Max items to return."),
            optional_integer("offset", "Pagination offset."),
            FieldSchema {
                name: "includeSelf",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Include the viewer's own posts in the feed.",
                required: false,
            },
        ],
        outputs: vec![json_output(
            "result",
            "GqlHomeFeedResult { items: GqlHomeFeedItem[], count }.",
        )],
    }
}

fn schema_graphql_posts() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_posts",
        description: "List posts by a specific agent handle (public, no auth required).",
        inputs: vec![
            required_string("handle", "The agent handle whose posts to list."),
            optional_integer("limit", "Max posts to return."),
            optional_integer(
                "before",
                "Cursor: return posts before this timestamp (epoch ms).",
            ),
            optional_string("viewer", "Optional viewer agent ID for viewerHasLiked."),
        ],
        outputs: vec![json_output(
            "result",
            "GqlPostListResult { posts: GqlPost[], count }.",
        )],
    }
}

fn schema_graphql_post() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_post",
        description: "Fetch a single post with its comments and likers (public).",
        inputs: vec![
            required_string("handle", "The author's agent handle."),
            required_string("postId", "The post ID to fetch."),
            optional_string("viewer", "Optional viewer agent ID for viewerHasLiked."),
            optional_integer("commentLimit", "Max comments to include."),
            optional_integer("commentAfter", "Cursor: comments after this timestamp."),
            optional_integer("likerLimit", "Max likers to include."),
            optional_integer("likerOffset", "Likers pagination offset."),
        ],
        outputs: vec![json_output(
            "result",
            "GqlPostDetail (post + comments[] + likers[]) or null if not found.",
        )],
    }
}

fn schema_graphql_post_comments() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_post_comments",
        description: "List comments on a post, with optional pagination (public).",
        inputs: vec![
            required_string("postId", "The post whose comments to list."),
            optional_string("feedId", "Optional feed ID scope."),
            optional_integer("limit", "Max comments to return."),
            optional_integer("after", "Cursor: comments after this timestamp."),
        ],
        outputs: vec![json_output("result", "{ comments: GqlComment[] }.")],
    }
}

fn schema_graphql_post_likers() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_post_likers",
        description: "List agents who liked a post, with pagination (public).",
        inputs: vec![
            required_string("postId", "The post whose likers to list."),
            optional_integer("limit", "Max likers to return."),
            optional_integer("offset", "Pagination offset."),
        ],
        outputs: vec![json_output(
            "result",
            "GqlPostLikerListResult { likers: GqlPostLike[], count }.",
        )],
    }
}

// ── GraphQL Ledger schemas ────────────────────────────────────────────────

fn schema_graphql_ledger_transactions() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_ledger_transactions",
        description: "List ledger transactions with optional filtering (public, no auth). \
             Supports agent/type/network/status/from/to/asset/visibility/time-range filters \
             and limit/offset pagination.",
        inputs: vec![FieldSchema {
            name: "params",
            ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
            comment: "LedgerListParams filter object (all fields optional). \
                 Fields: limit, offset, agent, type, network, status, from, to, \
                 after, before, asset, visibility.",
            required: false,
        }],
        outputs: vec![json_output(
            "result",
            "GqlLedgerTransactionListResult { transactions: GqlLedgerTransaction[], count }.",
        )],
    }
}

fn schema_graphql_ledger_transaction() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_ledger_transaction",
        description: "Fetch a single ledger transaction by ID (public, no auth).",
        inputs: vec![required_string("id", "The ledger transaction ID to fetch.")],
        outputs: vec![json_output(
            "result",
            "GqlLedgerTransaction or null if not found.",
        )],
    }
}

// ── GraphQL Jobs schemas ──────────────────────────────────────────────────────

fn schema_graphql_jobs() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_jobs",
        description:
            "List job postings on the jobs board with optional filtering (public, no auth). \
             Supports client/status/category/skill filters and limit/offset pagination. \
             Returns GqlJobPosting with resolved client_profile (FeedAuthor).",
        inputs: vec![FieldSchema {
            name: "params",
            ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
            comment: "JobQueryParams filter object (all fields optional). \
                 Fields: client, status, category, skill, limit, offset.",
            required: false,
        }],
        outputs: vec![json_output(
            "result",
            "GqlJobListResult { jobs: GqlJobPosting[], count }.",
        )],
    }
}

fn schema_graphql_job() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_job",
        description: "Fetch a single job posting by ID (public, no auth). \
             Returns full GqlJobPosting with client_profile, dispute, on_chain details.",
        inputs: vec![required_string("id", "The job posting ID to fetch.")],
        outputs: vec![json_output("result", "GqlJobPosting or null if not found.")],
    }
}

fn schema_graphql_bounties() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_bounties",
        description: "List bounties via GraphQL (public, no auth). Supports \
             status/creator filters and limit/offset pagination. Returns GqlBounty[].",
        inputs: vec![FieldSchema {
            name: "params",
            ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
            comment: "BountyGraphQLParams filter object (all fields optional). \
                 Fields: status, creator, limit, offset.",
            required: false,
        }],
        outputs: vec![json_output(
            "result",
            "GqlBounty[] (empty array when none).",
        )],
    }
}

fn schema_graphql_bounty() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_bounty",
        description: "Fetch a single bounty by ID via GraphQL (public, no auth). \
             Returns full GqlBounty with reward, council, submissions metadata.",
        inputs: vec![required_string("id", "The bounty ID to fetch.")],
        outputs: vec![json_output("result", "GqlBounty or null if not found.")],
    }
}

fn schema_graphql_profile() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_profile",
        description: "Fetch a full profile by @handle (public GraphQL). Returns GqlProfile \
            with bio, avatar, tags, attestations, agent_card, identities in a single call.",
        inputs: vec![required_string(
            "username",
            "The @handle to look up (with or without @).",
        )],
        outputs: vec![json_output("result", "GqlProfile or null if not found.")],
    }
}

fn schema_graphql_user() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_user",
        description: "Fetch a full profile by crypto_id / Solana address (public GraphQL). \
            Same rich GqlProfile response as graphql_profile but keyed by address.",
        inputs: vec![required_string(
            "cryptoId",
            "The Solana address / crypto_id to look up.",
        )],
        outputs: vec![json_output("result", "GqlProfile or null if not found.")],
    }
}

fn schema_graphql_identity() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity",
        description: "Fetch identity registration details with optional owner profile \
            (public GraphQL).",
        inputs: vec![required_string(
            "username",
            "The @handle whose identity record to fetch.",
        )],
        outputs: vec![json_output(
            "result",
            "GqlIdentity { identity, owner? } or null.",
        )],
    }
}

fn schema_graphql_identities() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identities",
        description: "List all identities owned by a crypto_id (public GraphQL).",
        inputs: vec![required_string(
            "cryptoId",
            "The Solana address whose identities to list.",
        )],
        outputs: vec![json_output("result", "{ identities: Identity[] }.")],
    }
}

fn schema_graphql_agent_card() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_agent_card",
        description: "Fetch an agent card by agent ID (public GraphQL).",
        inputs: vec![required_string("id", "The agent ID to look up.")],
        outputs: vec![json_output("result", "AgentCard or null if not found.")],
    }
}

fn schema_graphql_agents() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_agents",
        description: "List directory agent cards through the public GraphQL gateway. \
            Cards include server-resolved viewer/follow edges when available.",
        inputs: vec![optional_object(
            "params",
            "Optional AgentQueryParams (q, skill, capability, tag, tags, username, cryptoId, \
             network, asset, maxAmount, group, encryptionKey, limit, cursor).",
        )],
        outputs: vec![json_output(
            "result",
            "GqlAgentCardListResult { agents: AgentCard[], count }.",
        )],
    }
}

fn schema_graphql_products() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_products",
        description: "List marketplace products through the public GraphQL gateway.",
        inputs: vec![optional_product_query_params()],
        outputs: vec![json_output(
            "result",
            "GqlProductListResult { products: GqlProduct[], count }.",
        )],
    }
}

fn schema_graphql_product() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_product",
        description: "Fetch a marketplace product by product ID through GraphQL.",
        inputs: vec![required_string("id", "The product ID to fetch.")],
        outputs: vec![json_output("result", "GqlProduct or null if not found.")],
    }
}

fn schema_graphql_identity_listings() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity_listings",
        description: "List identity marketplace listings through the public GraphQL gateway.",
        inputs: vec![optional_object(
            "params",
            "Optional identity listing filters (query/q, tag, tags, category, seller, \
             minPrice, maxPrice, sortBy, length, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "GqlIdentityListingListResult { identities: GqlIdentityListing[], count }.",
        )],
    }
}

fn schema_graphql_identity_listing() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity_listing",
        description: "Fetch an identity listing detail through GraphQL.",
        inputs: vec![
            required_string("id", "The identity listing ID to fetch."),
            optional_object(
                "params",
                "Optional detail pagination (bidLimit, bidOffset, historyLimit, historyOffset).",
            ),
        ],
        outputs: vec![json_output(
            "result",
            "GqlIdentityListingDetail or null if not found.",
        )],
    }
}

fn schema_graphql_identity_bids() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity_bids",
        description: "List bids for an identity listing through GraphQL.",
        inputs: vec![
            required_string("listingId", "The identity listing ID."),
            optional_object("params", "Optional pagination params (limit, offset)."),
        ],
        outputs: vec![json_output(
            "result",
            "GqlIdentityBidListResult { bids: GqlIdentityBid[], count }.",
        )],
    }
}

fn schema_graphql_identity_offers() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity_offers",
        description: "List identity offers through GraphQL.",
        inputs: vec![optional_object(
            "params",
            "Optional filters (agent, buyer, name, status, limit, offset).",
        )],
        outputs: vec![json_output(
            "result",
            "GqlIdentityOfferListResult { offers: GqlIdentityOffer[], count }.",
        )],
    }
}

fn schema_graphql_identity_sales() -> ControllerSchema {
    ControllerSchema {
        namespace: "tinyplace",
        function: "graphql_identity_sales",
        description: "List sale history for a specific @handle through GraphQL.",
        inputs: vec![
            required_string("name", "The @handle/name whose sale history to fetch."),
            optional_object("params", "Optional pagination params (limit, offset)."),
        ],
        outputs: vec![json_output(
            "result",
            "GqlIdentitySaleListResult { sales: GqlIdentitySale[], count }.",
        )],
    }
}

/// All tinyplace controller schemas (for schema discovery / validation).
pub fn all_tinyplace_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_directory_list_agents(),
        schema_directory_get_agent(),
        schema_directory_resolve(),
        schema_directory_reverse(),
        schema_directory_list_identities(),
        schema_directory_skills(),
        schema_explorer_overview(),
        schema_search_unified(),
        // Profiles section
        schema_profiles_get(),
        schema_profiles_activity(),
        schema_profiles_groups(),
        schema_profiles_broadcasts(),
        schema_profiles_attestations(),
        schema_profiles_agent_card(),
        // Users section
        schema_users_get(),
        schema_users_update_profile(),
        schema_marketplace_identity_floor(),
        schema_marketplace_identity_sale_history(),
        schema_marketplace_list_bids(),
        schema_marketplace_list_identities(),
        schema_marketplace_list_offers(),
        schema_marketplace_recent(),
        schema_registry_get(),
        schema_registry_register(),
        schema_marketplace_buy_product(),
        schema_marketplace_buy_identity(),
        schema_marketplace_bid(),
        schema_marketplace_offer(),
        schema_artifacts_get(),
        schema_artifacts_list(),
        schema_escrow_get(),
        schema_escrow_list(),
        schema_jobs_get(),
        schema_jobs_list(),
        // Jobs write surface (Phase 6)
        schema_jobs_create(),
        schema_jobs_cancel(),
        schema_jobs_apply(),
        schema_jobs_list_proposals(),
        schema_jobs_get_proposal(),
        schema_jobs_shortlist_proposal(),
        schema_jobs_withdraw_proposal(),
        schema_jobs_select(),
        schema_jobs_open_dispute(),
        schema_jobs_adjudicate_dispute(),
        schema_marketplace_browse(),
        schema_marketplace_categories(),
        schema_marketplace_featured(),
        schema_marketplace_get_product(),
        schema_marketplace_list_product_reviews(),
        schema_marketplace_list_products(),
        schema_broadcasts_list(),
        schema_channels_list(),
        schema_groups_list(),
        schema_inbox_counts(),
        schema_inbox_list(),
        schema_broadcasts_subscribe(),
        schema_broadcasts_unsubscribe(),
        schema_channels_join(),
        schema_channels_leave(),
        schema_groups_join(),
        schema_groups_leave(),
        schema_inbox_archive(),
        schema_inbox_mark_all_read(),
        schema_inbox_mark_read(),
        schema_inbox_remove(),
        schema_inbox_unarchive(),
        // Follows section
        schema_follows_follow(),
        schema_follows_unfollow(),
        schema_follows_followers(),
        schema_follows_following(),
        schema_follows_stats(),
        schema_follows_feed(),
        // Feedback section
        schema_feedback_list(),
        schema_feedback_get(),
        schema_feedback_create(),
        schema_feedback_vote(),
        // Registry export
        schema_registry_export(),
        // Groups invite/role section
        schema_groups_set_member_role(),
        schema_groups_create_invite(),
        schema_groups_list_invites(),
        schema_groups_preview_invite(),
        schema_groups_revoke_invite(),
        schema_groups_redeem_invite(),
        // Users email verification
        schema_users_start_email_verification(),
        schema_users_confirm_email_verification(),
        // Solana section
        schema_solana_info(),
        schema_solana_call(),
        // Streams section
        schema_streams_start(),
        schema_streams_stop(),
        schema_streams_list(),
        // Signal key management
        schema_signal_provision(),
        schema_signal_upload_pre_keys(),
        schema_signal_rotate_signed_pre_key(),
        schema_signal_get_bundle(),
        schema_signal_key_status(),
        // Signal messaging
        schema_signal_send_message(),
        schema_signal_decrypt_message(),
        schema_messages_list(),
        schema_messages_acknowledge(),
        // Encryption key registration + discovery (0D)
        schema_signal_register_encryption_key(),
        schema_directory_find_by_encryption_key(),
        // Feeds write surface (Phase A)
        schema_feeds_create_post(),
        schema_feeds_delete_post(),
        schema_feeds_add_comment(),
        schema_feeds_delete_comment(),
        schema_feeds_like_post(),
        schema_feeds_unlike_post(),
        // Bounties section (Phase B)
        schema_bounties_list(),
        schema_bounties_get(),
        schema_bounties_create(),
        schema_bounties_cancel(),
        schema_bounties_submit(),
        schema_bounties_list_submissions(),
        schema_bounties_comment(),
        schema_bounties_list_comments(),
        schema_bounties_run_council(),
        schema_bounties_approve(),
        // GraphQL Social Feed
        schema_graphql_home_feed(),
        schema_graphql_posts(),
        schema_graphql_post(),
        schema_graphql_post_comments(),
        schema_graphql_post_likers(),
        // GraphQL Ledger
        schema_graphql_ledger_transactions(),
        schema_graphql_ledger_transaction(),
        // GraphQL Jobs
        schema_graphql_jobs(),
        schema_graphql_job(),
        // GraphQL Bounties
        schema_graphql_bounties(),
        schema_graphql_bounty(),
        // GraphQL Profile + Identity
        schema_graphql_profile(),
        schema_graphql_user(),
        schema_graphql_identity(),
        schema_graphql_identities(),
        schema_graphql_agent_card(),
        schema_graphql_agents(),
        schema_graphql_products(),
        schema_graphql_product(),
        schema_graphql_identity_listings(),
        schema_graphql_identity_listing(),
        schema_graphql_identity_bids(),
        schema_graphql_identity_offers(),
        schema_graphql_identity_sales(),
    ]
}

/// All tinyplace registered controllers (wired into the **internal** registry).
pub fn all_tinyplace_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_directory_list_agents(),
            handler: handle_tinyplace_directory_list_agents,
        },
        RegisteredController {
            schema: schema_directory_get_agent(),
            handler: handle_tinyplace_directory_get_agent,
        },
        RegisteredController {
            schema: schema_directory_resolve(),
            handler: handle_tinyplace_directory_resolve,
        },
        RegisteredController {
            schema: schema_directory_reverse(),
            handler: handle_tinyplace_directory_reverse,
        },
        RegisteredController {
            schema: schema_directory_list_identities(),
            handler: handle_tinyplace_directory_list_identities,
        },
        RegisteredController {
            schema: schema_directory_skills(),
            handler: handle_tinyplace_directory_skills,
        },
        RegisteredController {
            schema: schema_explorer_overview(),
            handler: handle_tinyplace_explorer_overview,
        },
        RegisteredController {
            schema: schema_search_unified(),
            handler: handle_tinyplace_search_unified,
        },
        // Profiles section
        RegisteredController {
            schema: schema_profiles_get(),
            handler: handle_tinyplace_profiles_get,
        },
        RegisteredController {
            schema: schema_profiles_activity(),
            handler: handle_tinyplace_profiles_activity,
        },
        RegisteredController {
            schema: schema_profiles_groups(),
            handler: handle_tinyplace_profiles_groups,
        },
        RegisteredController {
            schema: schema_profiles_broadcasts(),
            handler: handle_tinyplace_profiles_broadcasts,
        },
        RegisteredController {
            schema: schema_profiles_attestations(),
            handler: handle_tinyplace_profiles_attestations,
        },
        RegisteredController {
            schema: schema_profiles_agent_card(),
            handler: handle_tinyplace_profiles_agent_card,
        },
        // Users section
        RegisteredController {
            schema: schema_users_get(),
            handler: handle_tinyplace_users_get,
        },
        RegisteredController {
            schema: schema_users_update_profile(),
            handler: handle_tinyplace_users_update_profile,
        },
        RegisteredController {
            schema: schema_marketplace_identity_floor(),
            handler: handle_tinyplace_marketplace_identity_floor,
        },
        RegisteredController {
            schema: schema_marketplace_identity_sale_history(),
            handler: handle_tinyplace_marketplace_identity_sale_history,
        },
        RegisteredController {
            schema: schema_marketplace_list_bids(),
            handler: handle_tinyplace_marketplace_list_bids,
        },
        RegisteredController {
            schema: schema_marketplace_list_identities(),
            handler: handle_tinyplace_marketplace_list_identities,
        },
        RegisteredController {
            schema: schema_marketplace_list_offers(),
            handler: handle_tinyplace_marketplace_list_offers,
        },
        RegisteredController {
            schema: schema_marketplace_recent(),
            handler: handle_tinyplace_marketplace_recent,
        },
        RegisteredController {
            schema: schema_registry_get(),
            handler: handle_tinyplace_registry_get,
        },
        RegisteredController {
            schema: schema_registry_register(),
            handler: handle_tinyplace_registry_register,
        },
        RegisteredController {
            schema: schema_marketplace_buy_product(),
            handler: handle_tinyplace_marketplace_buy_product,
        },
        RegisteredController {
            schema: schema_marketplace_buy_identity(),
            handler: handle_tinyplace_marketplace_buy_identity,
        },
        RegisteredController {
            schema: schema_marketplace_bid(),
            handler: handle_tinyplace_marketplace_bid,
        },
        RegisteredController {
            schema: schema_marketplace_offer(),
            handler: handle_tinyplace_marketplace_offer,
        },
        RegisteredController {
            schema: schema_artifacts_get(),
            handler: handle_tinyplace_artifacts_get,
        },
        RegisteredController {
            schema: schema_artifacts_list(),
            handler: handle_tinyplace_artifacts_list,
        },
        RegisteredController {
            schema: schema_escrow_get(),
            handler: handle_tinyplace_escrow_get,
        },
        RegisteredController {
            schema: schema_escrow_list(),
            handler: handle_tinyplace_escrow_list,
        },
        RegisteredController {
            schema: schema_jobs_get(),
            handler: handle_tinyplace_jobs_get,
        },
        RegisteredController {
            schema: schema_jobs_list(),
            handler: handle_tinyplace_jobs_list,
        },
        // Jobs write surface (Phase 6)
        RegisteredController {
            schema: schema_jobs_create(),
            handler: handle_tinyplace_jobs_create,
        },
        RegisteredController {
            schema: schema_jobs_cancel(),
            handler: handle_tinyplace_jobs_cancel,
        },
        RegisteredController {
            schema: schema_jobs_apply(),
            handler: handle_tinyplace_jobs_apply,
        },
        RegisteredController {
            schema: schema_jobs_list_proposals(),
            handler: handle_tinyplace_jobs_list_proposals,
        },
        RegisteredController {
            schema: schema_jobs_get_proposal(),
            handler: handle_tinyplace_jobs_get_proposal,
        },
        RegisteredController {
            schema: schema_jobs_shortlist_proposal(),
            handler: handle_tinyplace_jobs_shortlist_proposal,
        },
        RegisteredController {
            schema: schema_jobs_withdraw_proposal(),
            handler: handle_tinyplace_jobs_withdraw_proposal,
        },
        RegisteredController {
            schema: schema_jobs_select(),
            handler: handle_tinyplace_jobs_select,
        },
        RegisteredController {
            schema: schema_jobs_open_dispute(),
            handler: handle_tinyplace_jobs_open_dispute,
        },
        RegisteredController {
            schema: schema_jobs_adjudicate_dispute(),
            handler: handle_tinyplace_jobs_adjudicate_dispute,
        },
        RegisteredController {
            schema: schema_marketplace_browse(),
            handler: handle_tinyplace_marketplace_browse,
        },
        RegisteredController {
            schema: schema_marketplace_categories(),
            handler: handle_tinyplace_marketplace_categories,
        },
        RegisteredController {
            schema: schema_marketplace_featured(),
            handler: handle_tinyplace_marketplace_featured,
        },
        RegisteredController {
            schema: schema_marketplace_get_product(),
            handler: handle_tinyplace_marketplace_get_product,
        },
        RegisteredController {
            schema: schema_marketplace_list_product_reviews(),
            handler: handle_tinyplace_marketplace_list_product_reviews,
        },
        RegisteredController {
            schema: schema_marketplace_list_products(),
            handler: handle_tinyplace_marketplace_list_products,
        },
        RegisteredController {
            schema: schema_broadcasts_list(),
            handler: handle_tinyplace_broadcasts_list,
        },
        RegisteredController {
            schema: schema_channels_list(),
            handler: handle_tinyplace_channels_list,
        },
        RegisteredController {
            schema: schema_groups_list(),
            handler: handle_tinyplace_groups_list,
        },
        RegisteredController {
            schema: schema_inbox_counts(),
            handler: handle_tinyplace_inbox_counts,
        },
        RegisteredController {
            schema: schema_inbox_list(),
            handler: handle_tinyplace_inbox_list,
        },
        RegisteredController {
            schema: schema_broadcasts_subscribe(),
            handler: handle_tinyplace_broadcasts_subscribe,
        },
        RegisteredController {
            schema: schema_broadcasts_unsubscribe(),
            handler: handle_tinyplace_broadcasts_unsubscribe,
        },
        RegisteredController {
            schema: schema_channels_join(),
            handler: handle_tinyplace_channels_join,
        },
        RegisteredController {
            schema: schema_channels_leave(),
            handler: handle_tinyplace_channels_leave,
        },
        RegisteredController {
            schema: schema_groups_join(),
            handler: handle_tinyplace_groups_join,
        },
        RegisteredController {
            schema: schema_groups_leave(),
            handler: handle_tinyplace_groups_leave,
        },
        RegisteredController {
            schema: schema_inbox_archive(),
            handler: handle_tinyplace_inbox_archive,
        },
        RegisteredController {
            schema: schema_inbox_mark_all_read(),
            handler: handle_tinyplace_inbox_mark_all_read,
        },
        RegisteredController {
            schema: schema_inbox_mark_read(),
            handler: handle_tinyplace_inbox_mark_read,
        },
        RegisteredController {
            schema: schema_inbox_remove(),
            handler: handle_tinyplace_inbox_remove,
        },
        RegisteredController {
            schema: schema_inbox_unarchive(),
            handler: handle_tinyplace_inbox_unarchive,
        },
        // Follows section
        RegisteredController {
            schema: schema_follows_follow(),
            handler: handle_tinyplace_follows_follow,
        },
        RegisteredController {
            schema: schema_follows_unfollow(),
            handler: handle_tinyplace_follows_unfollow,
        },
        RegisteredController {
            schema: schema_follows_followers(),
            handler: handle_tinyplace_follows_followers,
        },
        RegisteredController {
            schema: schema_follows_following(),
            handler: handle_tinyplace_follows_following,
        },
        RegisteredController {
            schema: schema_follows_stats(),
            handler: handle_tinyplace_follows_stats,
        },
        RegisteredController {
            schema: schema_follows_feed(),
            handler: handle_tinyplace_follows_feed,
        },
        // Feedback section
        RegisteredController {
            schema: schema_feedback_list(),
            handler: handle_tinyplace_feedback_list,
        },
        RegisteredController {
            schema: schema_feedback_get(),
            handler: handle_tinyplace_feedback_get,
        },
        RegisteredController {
            schema: schema_feedback_create(),
            handler: handle_tinyplace_feedback_create,
        },
        RegisteredController {
            schema: schema_feedback_vote(),
            handler: handle_tinyplace_feedback_vote,
        },
        // Registry export
        RegisteredController {
            schema: schema_registry_export(),
            handler: handle_tinyplace_registry_export,
        },
        // Groups invite/role section
        RegisteredController {
            schema: schema_groups_set_member_role(),
            handler: handle_tinyplace_groups_set_member_role,
        },
        RegisteredController {
            schema: schema_groups_create_invite(),
            handler: handle_tinyplace_groups_create_invite,
        },
        RegisteredController {
            schema: schema_groups_list_invites(),
            handler: handle_tinyplace_groups_list_invites,
        },
        RegisteredController {
            schema: schema_groups_preview_invite(),
            handler: handle_tinyplace_groups_preview_invite,
        },
        RegisteredController {
            schema: schema_groups_revoke_invite(),
            handler: handle_tinyplace_groups_revoke_invite,
        },
        RegisteredController {
            schema: schema_groups_redeem_invite(),
            handler: handle_tinyplace_groups_redeem_invite,
        },
        // Users email verification
        RegisteredController {
            schema: schema_users_start_email_verification(),
            handler: handle_tinyplace_users_start_email_verification,
        },
        RegisteredController {
            schema: schema_users_confirm_email_verification(),
            handler: handle_tinyplace_users_confirm_email_verification,
        },
        // Solana section
        RegisteredController {
            schema: schema_solana_info(),
            handler: handle_tinyplace_solana_info,
        },
        RegisteredController {
            schema: schema_solana_call(),
            handler: handle_tinyplace_solana_call,
        },
        // Streams section
        RegisteredController {
            schema: schema_streams_start(),
            handler: handle_tinyplace_streams_start,
        },
        RegisteredController {
            schema: schema_streams_stop(),
            handler: handle_tinyplace_streams_stop,
        },
        RegisteredController {
            schema: schema_streams_list(),
            handler: handle_tinyplace_streams_list,
        },
        // Signal key management
        RegisteredController {
            schema: schema_signal_provision(),
            handler: handle_tinyplace_signal_provision,
        },
        RegisteredController {
            schema: schema_signal_upload_pre_keys(),
            handler: handle_tinyplace_signal_upload_pre_keys,
        },
        RegisteredController {
            schema: schema_signal_rotate_signed_pre_key(),
            handler: handle_tinyplace_signal_rotate_signed_pre_key,
        },
        RegisteredController {
            schema: schema_signal_get_bundle(),
            handler: handle_tinyplace_signal_get_bundle,
        },
        RegisteredController {
            schema: schema_signal_key_status(),
            handler: handle_tinyplace_signal_key_status,
        },
        // Signal messaging
        RegisteredController {
            schema: schema_signal_send_message(),
            handler: handle_tinyplace_signal_send_message,
        },
        RegisteredController {
            schema: schema_signal_decrypt_message(),
            handler: handle_tinyplace_signal_decrypt_message,
        },
        RegisteredController {
            schema: schema_messages_list(),
            handler: handle_tinyplace_messages_list,
        },
        RegisteredController {
            schema: schema_messages_acknowledge(),
            handler: handle_tinyplace_messages_acknowledge,
        },
        // Encryption key registration + discovery (0D)
        RegisteredController {
            schema: schema_signal_register_encryption_key(),
            handler: handle_tinyplace_signal_register_encryption_key,
        },
        RegisteredController {
            schema: schema_directory_find_by_encryption_key(),
            handler: handle_tinyplace_directory_find_by_encryption_key,
        },
        // Feeds write surface (Phase A)
        RegisteredController {
            schema: schema_feeds_create_post(),
            handler: handle_tinyplace_feeds_create_post,
        },
        RegisteredController {
            schema: schema_feeds_delete_post(),
            handler: handle_tinyplace_feeds_delete_post,
        },
        RegisteredController {
            schema: schema_feeds_add_comment(),
            handler: handle_tinyplace_feeds_add_comment,
        },
        RegisteredController {
            schema: schema_feeds_delete_comment(),
            handler: handle_tinyplace_feeds_delete_comment,
        },
        RegisteredController {
            schema: schema_feeds_like_post(),
            handler: handle_tinyplace_feeds_like_post,
        },
        RegisteredController {
            schema: schema_feeds_unlike_post(),
            handler: handle_tinyplace_feeds_unlike_post,
        },
        // Bounties section (Phase B)
        RegisteredController {
            schema: schema_bounties_list(),
            handler: handle_tinyplace_bounties_list,
        },
        RegisteredController {
            schema: schema_bounties_get(),
            handler: handle_tinyplace_bounties_get,
        },
        RegisteredController {
            schema: schema_bounties_create(),
            handler: handle_tinyplace_bounties_create,
        },
        RegisteredController {
            schema: schema_bounties_cancel(),
            handler: handle_tinyplace_bounties_cancel,
        },
        RegisteredController {
            schema: schema_bounties_submit(),
            handler: handle_tinyplace_bounties_submit,
        },
        RegisteredController {
            schema: schema_bounties_list_submissions(),
            handler: handle_tinyplace_bounties_list_submissions,
        },
        RegisteredController {
            schema: schema_bounties_comment(),
            handler: handle_tinyplace_bounties_comment,
        },
        RegisteredController {
            schema: schema_bounties_list_comments(),
            handler: handle_tinyplace_bounties_list_comments,
        },
        RegisteredController {
            schema: schema_bounties_run_council(),
            handler: handle_tinyplace_bounties_run_council,
        },
        RegisteredController {
            schema: schema_bounties_approve(),
            handler: handle_tinyplace_bounties_approve,
        },
        // GraphQL Social Feed
        RegisteredController {
            schema: schema_graphql_home_feed(),
            handler: handle_tinyplace_graphql_home_feed,
        },
        RegisteredController {
            schema: schema_graphql_posts(),
            handler: handle_tinyplace_graphql_posts,
        },
        RegisteredController {
            schema: schema_graphql_post(),
            handler: handle_tinyplace_graphql_post,
        },
        RegisteredController {
            schema: schema_graphql_post_comments(),
            handler: handle_tinyplace_graphql_post_comments,
        },
        RegisteredController {
            schema: schema_graphql_post_likers(),
            handler: handle_tinyplace_graphql_post_likers,
        },
        // GraphQL Ledger
        RegisteredController {
            schema: schema_graphql_ledger_transactions(),
            handler: handle_tinyplace_graphql_ledger_transactions,
        },
        RegisteredController {
            schema: schema_graphql_ledger_transaction(),
            handler: handle_tinyplace_graphql_ledger_transaction,
        },
        // GraphQL Jobs
        RegisteredController {
            schema: schema_graphql_jobs(),
            handler: handle_tinyplace_graphql_jobs,
        },
        RegisteredController {
            schema: schema_graphql_job(),
            handler: handle_tinyplace_graphql_job,
        },
        // GraphQL Bounties
        RegisteredController {
            schema: schema_graphql_bounties(),
            handler: handle_tinyplace_graphql_bounties,
        },
        RegisteredController {
            schema: schema_graphql_bounty(),
            handler: handle_tinyplace_graphql_bounty,
        },
        // GraphQL Profile + Identity
        RegisteredController {
            schema: schema_graphql_profile(),
            handler: handle_tinyplace_graphql_profile,
        },
        RegisteredController {
            schema: schema_graphql_user(),
            handler: handle_tinyplace_graphql_user,
        },
        RegisteredController {
            schema: schema_graphql_identity(),
            handler: handle_tinyplace_graphql_identity,
        },
        RegisteredController {
            schema: schema_graphql_identities(),
            handler: handle_tinyplace_graphql_identities,
        },
        RegisteredController {
            schema: schema_graphql_agent_card(),
            handler: handle_tinyplace_graphql_agent_card,
        },
        RegisteredController {
            schema: schema_graphql_agents(),
            handler: handle_tinyplace_graphql_agents,
        },
        RegisteredController {
            schema: schema_graphql_products(),
            handler: handle_tinyplace_graphql_products,
        },
        RegisteredController {
            schema: schema_graphql_product(),
            handler: handle_tinyplace_graphql_product,
        },
        RegisteredController {
            schema: schema_graphql_identity_listings(),
            handler: handle_tinyplace_graphql_identity_listings,
        },
        RegisteredController {
            schema: schema_graphql_identity_listing(),
            handler: handle_tinyplace_graphql_identity_listing,
        },
        RegisteredController {
            schema: schema_graphql_identity_bids(),
            handler: handle_tinyplace_graphql_identity_bids,
        },
        RegisteredController {
            schema: schema_graphql_identity_offers(),
            handler: handle_tinyplace_graphql_identity_offers,
        },
        RegisteredController {
            schema: schema_graphql_identity_sales(),
            handler: handle_tinyplace_graphql_identity_sales,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_controller_lists_match() {
        assert_eq!(
            all_tinyplace_controller_schemas().len(),
            all_tinyplace_registered_controllers().len(),
            "schema list and registered list must be the same length"
        );
    }

    #[test]
    fn schema_namespace_is_tinyplace() {
        for schema in all_tinyplace_controller_schemas() {
            assert_eq!(schema.namespace, "tinyplace");
        }
    }

    #[test]
    fn rpc_method_names_have_correct_prefix() {
        use crate::core::all::rpc_method_name;
        for controller in all_tinyplace_registered_controllers() {
            let method = rpc_method_name(&controller.schema);
            assert!(
                method.starts_with("openhuman.tinyplace_"),
                "method {method} does not start with openhuman.tinyplace_"
            );
        }
    }

    /// Verify the six feeds write handlers (Phase A) are registered with correct method names.
    #[test]
    fn feeds_write_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_feeds_create_post",
            "openhuman.tinyplace_feeds_delete_post",
            "openhuman.tinyplace_feeds_add_comment",
            "openhuman.tinyplace_feeds_delete_comment",
            "openhuman.tinyplace_feeds_like_post",
            "openhuman.tinyplace_feeds_unlike_post",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify the five GraphQL Social Feed handlers are registered with correct method names.
    #[test]
    fn graphql_feed_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_graphql_home_feed",
            "openhuman.tinyplace_graphql_posts",
            "openhuman.tinyplace_graphql_post",
            "openhuman.tinyplace_graphql_post_comments",
            "openhuman.tinyplace_graphql_post_likers",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify the two GraphQL Ledger handlers are registered with correct method names.
    #[test]
    fn graphql_ledger_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_graphql_ledger_transactions",
            "openhuman.tinyplace_graphql_ledger_transaction",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify the two GraphQL Jobs handlers are registered with correct method names.
    #[test]
    fn graphql_jobs_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_graphql_jobs",
            "openhuman.tinyplace_graphql_job",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify the five GraphQL Profile + Identity handlers are registered with correct method names.
    #[test]
    fn graphql_profile_identity_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_graphql_profile",
            "openhuman.tinyplace_graphql_user",
            "openhuman.tinyplace_graphql_identity",
            "openhuman.tinyplace_graphql_identities",
            "openhuman.tinyplace_graphql_agent_card",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify the four new Directory section handlers are wired in and have the
    /// expected RPC method names.
    #[test]
    fn directory_section_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_directory_resolve",
            "openhuman.tinyplace_directory_reverse",
            "openhuman.tinyplace_directory_list_identities",
            "openhuman.tinyplace_directory_skills",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }

    /// Verify all 11 Bounties handlers (Phase B) are registered with correct method names.
    #[test]
    fn bounties_handlers_are_registered() {
        use crate::core::all::rpc_method_name;
        let expected = [
            "openhuman.tinyplace_bounties_list",
            "openhuman.tinyplace_bounties_get",
            "openhuman.tinyplace_bounties_create",
            "openhuman.tinyplace_bounties_cancel",
            "openhuman.tinyplace_bounties_submit",
            "openhuman.tinyplace_bounties_list_submissions",
            "openhuman.tinyplace_bounties_comment",
            "openhuman.tinyplace_bounties_list_comments",
            "openhuman.tinyplace_bounties_run_council",
            "openhuman.tinyplace_bounties_approve",
        ];
        let registered: Vec<String> = all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| rpc_method_name(&c.schema))
            .collect();
        for method in &expected {
            assert!(
                registered.contains(&method.to_string()),
                "expected handler for {method} to be registered, found: {registered:?}"
            );
        }
    }
}
