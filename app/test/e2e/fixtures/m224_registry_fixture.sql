-- M2.2.4 Task 6 disposable live-Core registry fixture.
-- Non-secret only. Seeds one active Tenant, >50 rows for every cursor-backed
-- registry collection, and deterministic primary records for the desktop flow.
-- Equivalent collection cardinality target: generate_series(1, 52) after the
-- explicit primary lifecycle rows are included.

INSERT INTO kernel_tenants (id, tenant_key, lifecycle_state, created_at, updated_at)
VALUES (
  '10000000-0000-4000-8000-000000000001',
  'tenant.registry.primary',
  'active',
  '2026-09-01T12:00:00Z',
  '2026-09-01T12:00:00Z'
);

INSERT INTO kernel_agents (
  id,
  tenant_id,
  agent_key,
  version,
  lifecycle_state,
  configuration_fingerprint,
  owner_actor_id,
  created_at,
  configuration,
  owner_actor_type
)
VALUES (
  '20000000-0000-4000-8000-000000000001',
  '10000000-0000-4000-8000-000000000001',
  'agent.registry.001-primary',
  1,
  'active',
  '1111111111111111111111111111111111111111111111111111111111111111',
  'registry-reader',
  '2026-09-01T12:01:00Z',
  '{
    "schema_version": 1,
    "domain_key": "registry",
    "owner": { "actor_type": "service", "actor_id": "registry-reader" },
    "allowed_tool_refs": [{ "tool_key": "tool.registry.reader", "version": 1 }],
    "knowledge_scope_refs": [{ "source_key": "docs.registry", "trust_version": "2026-09", "access_scope": "read" }],
    "risk_policy_ref": { "policy_id": "policy.registry", "policy_version": "v1" }
  }'::jsonb,
  'service'
);

INSERT INTO kernel_agents (
  id,
  tenant_id,
  agent_key,
  version,
  lifecycle_state,
  configuration_fingerprint,
  owner_actor_id,
  created_at,
  configuration,
  owner_actor_type
)
SELECT
  format('20000000-0000-4000-8000-%012s', lpad((n + 100)::text, 12, '0'))::uuid,
  '10000000-0000-4000-8000-000000000001'::uuid,
  format('agent.registry.zzz-%03s', n),
  1,
  'active',
  md5(format('agent-fingerprint:%s:a', n)) || md5(format('agent-fingerprint:%s:b', n)),
  format('registry-filler-%03s', n),
  '2026-09-01T12:02:00Z'::timestamptz + make_interval(secs => n),
  jsonb_build_object(
    'schema_version',
    1,
    'domain_key',
    'registry',
    'owner',
    jsonb_build_object('actor_type', 'service', 'actor_id', format('registry-filler-%03s', n)),
    'allowed_tool_refs',
    jsonb_build_array(jsonb_build_object('tool_key', format('tool.registry.zzz.%03s', n), 'version', 1)),
    'knowledge_scope_refs',
    jsonb_build_array(
      jsonb_build_object(
        'source_key',
        format('docs.registry.%03s', n),
        'trust_version',
        '2026-09',
        'access_scope',
        'read'
      )
    ),
    'risk_policy_ref',
    jsonb_build_object('policy_id', 'policy.registry', 'policy_version', 'v1')
  ),
  'service'
FROM generate_series(1, 51) AS n;

INSERT INTO kernel_tool_definitions (
  id,
  tool_key,
  version,
  lifecycle_state,
  input_schema,
  output_schema,
  created_at,
  definition_fingerprint,
  schema_version,
  display_name,
  description,
  tool_effect_class,
  abstract_auth_scopes_json,
  timeout_defaults_json,
  retry_contract_json,
  audit_contract_json
)
VALUES
  (
    '30000000-0000-4000-8000-000000000001',
    'tool.registry.reader',
    1,
    'active',
    '{"type":"object","properties":{"registry_key":{"type":"string"}},"required":["registry_key"]}'::jsonb,
    '{"type":"object","properties":{"rows":{"type":"array"}}}'::jsonb,
    '2026-09-01T12:10:00Z',
    '2222222222222222222222222222222222222222222222222222222222222222',
    1,
    'Registry Reader',
    'Reads published registry rows.',
    'read_only',
    '["registry.read"]'::jsonb,
    '{"timeout_ms":3000}'::jsonb,
    '{"attempts":1}'::jsonb,
    '{"mode":"metadata_only"}'::jsonb
  ),
  (
    '30000000-0000-4000-8000-000000000002',
    'tool.registry.guard',
    1,
    'active',
    '{"type":"object"}'::jsonb,
    '{"type":"object"}'::jsonb,
    '2026-09-01T12:10:01Z',
    '3333333333333333333333333333333333333333333333333333333333333333',
    1,
    'Registry Guard',
    'Describes approval-gated registry posture.',
    'effectful',
    '["registry.guard"]'::jsonb,
    '{"timeout_ms":5000}'::jsonb,
    '{"attempts":2}'::jsonb,
    '{"mode":"redacted_io"}'::jsonb
  ),
  (
    '30000000-0000-4000-8000-000000000003',
    'tool.registry.shadow',
    1,
    'active',
    '{"type":"object"}'::jsonb,
    '{"type":"object"}'::jsonb,
    '2026-09-01T12:10:02Z',
    '4444444444444444444444444444444444444444444444444444444444444444',
    1,
    'Registry Shadow',
    'Exists without a tenant enablement.',
    'read_only',
    '["registry.shadow"]'::jsonb,
    '{"timeout_ms":2000}'::jsonb,
    '{"attempts":1}'::jsonb,
    '{"mode":"metadata_only"}'::jsonb
  );

INSERT INTO kernel_tool_definitions (
  id,
  tool_key,
  version,
  lifecycle_state,
  input_schema,
  output_schema,
  created_at,
  definition_fingerprint,
  schema_version,
  display_name,
  description,
  tool_effect_class,
  abstract_auth_scopes_json,
  timeout_defaults_json,
  retry_contract_json,
  audit_contract_json
)
SELECT
  format('30000000-0000-4000-8000-%012s', lpad((n + 100)::text, 12, '0'))::uuid,
  format('tool.registry.zzz.%03s', n),
  1,
  'active',
  '{"type":"object"}'::jsonb,
  '{"type":"object"}'::jsonb,
  '2026-09-01T12:11:00Z'::timestamptz + make_interval(secs => n),
  md5(format('tool-fingerprint:%s:a', n)) || md5(format('tool-fingerprint:%s:b', n)),
  1,
  format('Registry Tool %03s', n),
  format('Filler registry tool %03s.', n),
  CASE WHEN n % 3 = 0 THEN 'destructive' WHEN n % 2 = 0 THEN 'effectful' ELSE 'read_only' END,
  jsonb_build_array(format('registry.fill.%03s', n)),
  jsonb_build_object('timeout_ms', 1500 + n),
  jsonb_build_object('attempts', 1),
  jsonb_build_object('mode', 'metadata_only')
FROM generate_series(1, 49) AS n;

INSERT INTO kernel_tool_enablements (
  id,
  tenant_id,
  tool_definition_id,
  lifecycle_state,
  created_at,
  generation,
  timeout_cap_ms,
  approval_required,
  allow_ttl_seconds,
  audit_mode,
  updated_at
)
VALUES
  (
    '40000000-0000-4000-8000-000000000001',
    '10000000-0000-4000-8000-000000000001',
    '30000000-0000-4000-8000-000000000001',
    'enabled',
    '2026-09-01T12:20:00Z',
    3,
    4500,
    false,
    null,
    'metadata_only',
    '2026-09-01T12:20:00Z'
  ),
  (
    '40000000-0000-4000-8000-000000000002',
    '10000000-0000-4000-8000-000000000001',
    '30000000-0000-4000-8000-000000000002',
    'disabled',
    '2026-09-01T12:20:01Z',
    2,
    9000,
    true,
    120,
    'redacted_io',
    '2026-09-01T12:20:01Z'
  );

INSERT INTO kernel_connector_types (
  id,
  connector_key,
  version,
  lifecycle_state,
  source_type,
  connector_type_fingerprint,
  capabilities_json,
  normalization_contracts_json,
  delivery_behavior_json,
  created_at
)
VALUES
  (
    '50000000-0000-4000-8000-000000000001',
    'connector.registry.feed',
    1,
    'draft',
    'registry_feed',
    '5555555555555555555555555555555555555555555555555555555555555555',
    '["messages.read","registry.inspect"]'::jsonb,
    '[{"evidence_family":"messages","kernel_event_type":"message.created","kernel_event_schema_version":1}]'::jsonb,
    '{"mode":"push","channel":"registry"}'::jsonb,
    '2026-09-01T12:30:00Z'
  ),
  (
    '50000000-0000-4000-8000-000000000002',
    'connector.registry.feed',
    2,
    'active',
    'registry_feed',
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    '["messages.read","registry.inspect"]'::jsonb,
    '[{"evidence_family":"messages","kernel_event_type":"message.created","kernel_event_schema_version":1}]'::jsonb,
    '{"mode":"push","channel":"registry"}'::jsonb,
    '2026-09-01T12:31:00Z'
  ),
  (
    '50000000-0000-4000-8000-000000000003',
    'connector.registry.feed',
    3,
    'retired',
    'registry_feed',
    '6666666666666666666666666666666666666666666666666666666666666666',
    '["messages.read","registry.inspect"]'::jsonb,
    '[{"evidence_family":"messages","kernel_event_type":"message.created","kernel_event_schema_version":1}]'::jsonb,
    '{"mode":"push","channel":"registry"}'::jsonb,
    '2026-09-01T12:32:00Z'
  );

INSERT INTO kernel_connector_types (
  id,
  connector_key,
  version,
  lifecycle_state,
  source_type,
  connector_type_fingerprint,
  capabilities_json,
  normalization_contracts_json,
  delivery_behavior_json,
  created_at
)
SELECT
  format('50000000-0000-4000-8000-%012s', lpad((n + 100)::text, 12, '0'))::uuid,
  format('connector.zzz.fill.%03s', n),
  1,
  'active',
  'registry_feed',
  md5(format('connector-type:%s:a', n)) || md5(format('connector-type:%s:b', n)),
  '["messages.read"]'::jsonb,
  '[{"evidence_family":"messages","kernel_event_type":"message.created","kernel_event_schema_version":1}]'::jsonb,
  '{"mode":"push"}'::jsonb,
  '2026-09-01T12:33:00Z'::timestamptz + make_interval(secs => n)
FROM generate_series(1, 49) AS n;

INSERT INTO kernel_connector_bindings (
  id,
  tenant_id,
  binding_key,
  version,
  connector_type_id,
  connector_type_key,
  connector_type_version,
  connector_type_fingerprint,
  lifecycle_state,
  provider_namespace,
  external_account_ref,
  config_ref,
  credential_ref,
  binding_fingerprint,
  enabled_capabilities_json,
  created_at
)
VALUES
  (
    '60000000-0000-4000-8000-000000000001',
    '10000000-0000-4000-8000-000000000001',
    'binding.registry-primary',
    1,
    '50000000-0000-4000-8000-000000000001',
    'connector.registry.feed',
    1,
    '5555555555555555555555555555555555555555555555555555555555555555',
    'draft',
    'provider.registry',
    'acct-sandbox-draft',
    'config://registry/draft',
    'credential://registry/draft',
    '7777777777777777777777777777777777777777777777777777777777777777',
    '["messages.read"]'::jsonb,
    '2026-09-01T12:40:00Z'
  ),
  (
    '60000000-0000-4000-8000-000000000002',
    '10000000-0000-4000-8000-000000000001',
    'binding.registry-primary',
    2,
    '50000000-0000-4000-8000-000000000002',
    'connector.registry.feed',
    2,
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    'active',
    'provider.registry',
    'acct-sandbox-primary',
    'config://registry/primary',
    'credential://registry/primary',
    '8888888888888888888888888888888888888888888888888888888888888888',
    '["messages.read","registry.inspect"]'::jsonb,
    '2026-09-01T12:41:00Z'
  ),
  (
    '60000000-0000-4000-8000-000000000003',
    '10000000-0000-4000-8000-000000000001',
    'binding.registry-primary',
    3,
    '50000000-0000-4000-8000-000000000003',
    'connector.registry.feed',
    3,
    '6666666666666666666666666666666666666666666666666666666666666666',
    'retired',
    'provider.registry',
    'acct-sandbox-retired',
    'config://registry/retired',
    'credential://registry/retired',
    '9999999999999999999999999999999999999999999999999999999999999999',
    '["messages.read"]'::jsonb,
    '2026-09-01T12:42:00Z'
  );

INSERT INTO kernel_connector_bindings (
  id,
  tenant_id,
  binding_key,
  version,
  connector_type_id,
  connector_type_key,
  connector_type_version,
  connector_type_fingerprint,
  lifecycle_state,
  provider_namespace,
  external_account_ref,
  config_ref,
  credential_ref,
  binding_fingerprint,
  enabled_capabilities_json,
  created_at
)
SELECT
  format('60000000-0000-4000-8000-%012s', lpad((n + 100)::text, 12, '0'))::uuid,
  '10000000-0000-4000-8000-000000000001'::uuid,
  format('binding.zzz.fill.%03s', n),
  1,
  '50000000-0000-4000-8000-000000000002'::uuid,
  'connector.registry.feed',
  2,
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'active',
  'provider.registry',
  format('acct-sandbox-%03s', n),
  format('config://registry/fill/%03s', n),
  format('credential://registry/fill/%03s', n),
  md5(format('binding:%s:a', n)) || md5(format('binding:%s:b', n)),
  '["messages.read"]'::jsonb,
  '2026-09-01T12:43:00Z'::timestamptz + make_interval(secs => n)
FROM generate_series(1, 49) AS n;
