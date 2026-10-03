# `integrations::composio::contract::catalogs`

Curated per-toolkit Composio action catalogs: which actions an agent may see,
how each is scoped (read / write / admin), the toolkit descriptions shown to
the model, and the per-toolkit sync-interval defaults for toolkits that have a
native provider.

These tables used to live in TinyMemory's v1 bus contract. They are host data
about Composio, not memory, so they moved here with memory v2.

Entry points (re-exported from `contract::mod`): `catalog_for_toolkit`,
`curated_scope_for`, `has_native_provider`, `is_action_visible_with_pref`,
`native_provider_sync_interval_secs`, `parse_sync_interval_override`,
`sync_interval_env_var`, `toolkit_description`, `toolkit_has_scope`,
`toolkit_result_notes`, `CAPABILITY_TOOLKITS`, `NATIVE_PROVIDERS`.
