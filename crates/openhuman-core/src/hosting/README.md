# Hosting adapter

OpenHuman exposes TinyHosts provider operations through the compiled TinyHosts
TinyBus module. This domain owns the host-specific policy: resolving the
Vercel credential from `[hosting]` or the documented environment variables,
scoping a deployable path to the configured workspace, and presenting the
model-facing tool declarations. It does not link the provider implementation.

The lazy module loader is first used when a hosting tool runs. The adapter
queries `Providers`, then sends the existing `Execute` envelope. Credential
bearing requests use confidential TinyBus delivery. The portable request and
tool vocabulary is defined in `vendor/tinyhosts/crates/tinyhosts-bus`; provider
API calls, source preparation, and deployment sequencing live in the released
TinyHosts module.

All ten names, descriptions, schemas, and approval classifications are read
from the release contract's `TOOL_DECLARATIONS_JSON`. OpenHuman keeps the
workspace containment check and checks that a rollback target is ready before
it asks the module to promote it. The module itself enforces preparation limits
and validates the uploaded snapshot.

`hosting::Account::from_config` returns no account when hosting is disabled or
no credential resolves. Invalid providers and blank explicit credentials are
configuration errors. `Account::connect` is the embedding seam for hosts that
resolve credentials in their own secret store; it uses the supplied workspace
for module artifact discovery as well as deployable paths. Embedders with a
custom module policy or artifact root can use `Account::connect_with_config`.

`Account::host()` was removed with the provider implementation dependency. Its
return type exposed TinyHosts' concrete implementation trait, which cannot be
kept in the host crate without restoring that dependency. Callers should use
the stable `Account::tools()` surface or the TinyHosts bus contract directly.

The ignored `released_artifact_serves_providers_and_confidential_local_preparation`
test exercises the pinned artifact through OpenHuman's configured module loader
and real private TinyBus. It queries `Providers`, then sends a confidential
`PrepareBundle` for a temporary local HTML file and checks the returned typed
snapshot. This operation is local and does not contact a hosting provider. Run
it with the released library at hand:

```bash
OPENHUMAN_TINYHOSTS_TEST_MODULE=/path/to/libtinyhosts.so \
  scripts/ci-cancel-aware.sh cargo +stable test --locked \
    --manifest-path Cargo.toml -p openhuman --no-default-features \
    --features 'hosting,crash-reporting' \
    hosting::test::released_artifact_serves_providers_and_confidential_local_preparation \
    -- --ignored --exact
```
