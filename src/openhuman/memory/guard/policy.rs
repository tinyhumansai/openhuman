//! [`GuardPolicy`] — the resolved policy bundle [`MemoryGuard`] and all ten
//! family decorators share.
//!
//! [`MemoryGuard`]: super::MemoryGuard
//!
//! ## What is cached here, and what deliberately is not
//!
//! The *binding* facts — driver id, [`DriverClass`], the hook budgets, and the
//! configured `trust_state` — are resolved once, at bind time, and stored. They
//! cannot change without a rebind, and the binding is what constructs this
//! type.
//!
//! The [`SecurityPolicy`] is **not** cached. It is read from
//! [`live_policy::current`] on every call. That is a deliberate departure from
//! the obvious design: `MemoryBinding` is cached in a process-global,
//! workspace-keyed map for the life of the process, and `live_policy` is
//! installed *after* the first bind and hot-swapped on every autonomy change
//! (`reload_from`, `update_action_dir`). Caching an `Option<Arc<SecurityPolicy>>`
//! at construction would freeze whatever was current at first bind — in
//! practice the pre-boot `None` — and the tier check would then never fire
//! again for the rest of the process. A `RwLock` read per call is cheap and is
//! the only thing that stays correct across a hot swap.
//!
//! ## `None` policy means "no tier enforcement", never "deny"
//!
//! [`live_policy::current`] returns `None` before a session runtime has
//! installed one, which is the state roughly four thousand pre-boot unit tests
//! run in. Denying there would fail all of them at once, for the same reason
//! [`unbound_default_capabilities`] returns the full set and `core::all`'s
//! `group_allowed` returns `true` with no ambient context: denying is only ever
//! correct *after* something has actually answered.
//!
//! [`unbound_default_capabilities`]: crate::openhuman::memory::binding::unbound_default_capabilities

use std::borrow::Cow;
use std::sync::Arc;

use crate::openhuman::memory::api::capabilities::Capability;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::types::SourceScope;
use crate::openhuman::memory::api::types::MemoryTaint;

use crate::core::subsystem::DriverClass;
use crate::openhuman::config::schema::MemoryHooksConfig;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::security::egress::emit_external_transfer;
use crate::openhuman::security::egress::types::{DataKind, EgressDescriptor, EgressReason};
use crate::openhuman::security::live_policy;
use crate::openhuman::security::policy::ToolOperation;

/// Prefix on every guard-authored error message, so a refusal that surfaces to
/// a caller is attributable to the guard rather than to the driver underneath.
pub const GUARD_DENIED_PREFIX: &str = "memory guard: ";

/// The trust state an external driver must carry before the guard will let a
/// call reach it. Matches the value `binding::admit` requires at bind time.
pub const TRUSTED: &str = "trusted";

/// The policy bundle every guarded call consults.
///
/// Cheap to clone-by-`Arc`: the guard holds one, and each of the ten family
/// decorators holds an `Arc` of the same value.
pub struct GuardPolicy {
    driver_id: String,
    class: DriverClass,
    hooks: MemoryHooksConfig,
    trust_state: String,
}

impl GuardPolicy {
    /// Build the policy for a bound driver.
    pub fn new(
        driver_id: impl Into<String>,
        class: DriverClass,
        hooks: MemoryHooksConfig,
        trust_state: impl Into<String>,
    ) -> Self {
        Self {
            driver_id: driver_id.into(),
            class,
            hooks,
            trust_state: trust_state.into(),
        }
    }

    /// The bound driver's stable id — appears in every span and audit event.
    pub fn driver_id(&self) -> &str {
        &self.driver_id
    }

    /// How the driver was reached. A host fact, never self-reported.
    pub fn class(&self) -> DriverClass {
        self.class
    }

    /// The configured hook budgets.
    pub fn hooks(&self) -> MemoryHooksConfig {
        self.hooks
    }

    /// The configured trust state for this driver binding.
    pub fn trust_state(&self) -> &str {
        &self.trust_state
    }

    // ── The admission gate: steps 1, 5 and 7 in one call ─────────────────────

    /// Enter the guard's tracing span, run the tier (step 1) and egress
    /// (step 5) checks inside it, and leave — all before the caller awaits
    /// anything.
    ///
    /// The span is entered and exited **within this synchronous call** on
    /// purpose. [`tracing::span::EnteredSpan`] is `!Send`, and every method on
    /// the driver contract is an `#[async_trait]` method whose future must be
    /// `Send`; holding an entered span across the `.await` of the forwarded
    /// driver call makes the whole future `!Send` and fails to compile. Keeping
    /// the span around the *decision* is also where it earns its keep — the
    /// driver below emits its own instrumented line for the work itself.
    ///
    /// # Errors
    ///
    /// The first refusal, tier or egress.
    pub fn admit_read(
        &self,
        capability: Capability,
        method: &str,
        namespace: &str,
        carries_content: bool,
    ) -> Result<(), MemoryError> {
        let span = super::audit::guard_span(self, capability, method, namespace);
        let _enter = span.enter();
        self.enforce_read(method)?;
        self.check_egress(method, carries_content)
    }

    /// [`Self::admit_read`] for a write, taking the `Act` tier check.
    ///
    /// # Errors
    ///
    /// The first refusal, tier or egress.
    pub fn admit_write(
        &self,
        capability: Capability,
        method: &str,
        namespace: &str,
        carries_content: bool,
    ) -> Result<(), MemoryError> {
        let span = super::audit::guard_span(self, capability, method, namespace);
        let _enter = span.enter();
        self.enforce_write(method)?;
        self.check_egress(method, carries_content)
    }

    // ── Step 1: SecurityPolicy tier ─────────────────────────────────────────

    /// Tier check for a **read** operation.
    ///
    /// [`SecurityPolicy::enforce_tool_operation`] answers `Ok` unconditionally
    /// for [`ToolOperation::Read`] — reads are never gated by autonomy tier or
    /// by the action budget. The call is made anyway rather than skipped, so
    /// that a future tier which *does* gate reads (privacy mode, a
    /// read-quarantined tier) starts applying here without a new call site.
    ///
    /// # Errors
    ///
    /// Whatever the live policy refuses, prefixed with [`GUARD_DENIED_PREFIX`].
    pub fn enforce_read(&self, operation: &str) -> Result<(), MemoryError> {
        self.enforce(ToolOperation::Read, operation)
    }

    /// Tier check for a **write** operation: the `readonly`-tier refusal, and
    /// deliberately **not** the hourly action budget.
    ///
    /// M4a routed this through [`ToolOperation::Act`], which is tier *plus*
    /// `SecurityPolicy::record_action`. That was wrong in two ways, and both
    /// bite the moment a call site migrates onto the guard (M4b):
    ///
    /// - **Double-count.** `memory/tools/store.rs` and `memory/tools/forget.rs`
    ///   already spend one budget unit each on `ToolOperation::Act`.
    ///   Re-pointing them at the guard would spend a second for the same call.
    /// - **Wrong granularity.** The budget is denominated in agent *tool
    ///   calls*. The guard sits under `MemoryCore::store`, which one bulk
    ///   ingest or one sync pass calls hundreds of times — enough to exhaust a
    ///   budget with no agent having acted at all.
    ///
    /// So the guard takes the tier half only, via
    /// [`SecurityPolicy::enforce_write_tier`]. That is the same shape the ~15
    /// acting tools which gate on bare `can_act()` already use
    /// (`tools/impl/filesystem/file_write.rs`,
    /// `tools/impl/system/python_exec.rs`, `cron/scheduler.rs`, …). Budget
    /// accounting stays where it is denominated: at the tool boundary.
    ///
    /// # Errors
    ///
    /// Whatever the live policy refuses, prefixed with [`GUARD_DENIED_PREFIX`].
    pub fn enforce_write(&self, operation: &str) -> Result<(), MemoryError> {
        // Read live, never cached — see the module docs.
        let Some(policy) = live_policy::current() else {
            return Ok(());
        };
        policy
            .enforce_write_tier(operation)
            .map_err(|reason| self.denied(operation, reason))
    }

    fn enforce(&self, op: ToolOperation, operation: &str) -> Result<(), MemoryError> {
        // Read live, never cached — see the module docs.
        let Some(policy) = live_policy::current() else {
            return Ok(());
        };
        policy
            .enforce_tool_operation(op, operation)
            .map_err(|reason| self.denied(operation, reason))
    }

    /// Build the guard's canonical refusal error, publishing the audit event as
    /// a side effect. Every deny path goes through here so a refusal can never
    /// be raised without the operator seeing it.
    pub fn denied(&self, method: &str, reason: impl Into<String>) -> MemoryError {
        let reason = reason.into();
        super::audit::publish_guard_denied(self, method, &reason);
        MemoryError::Invalid(format!("{GUARD_DENIED_PREFIX}{reason}"))
    }

    // ── Step 1b: path rules — deliberately a no-op ───────────────────────────
    //
    // `validate_memory_relative_path` / `resolve_writable_memory_path` in
    // `memory/ops/helpers.rs` validate `<workspace>/memory/<relative>` *file*
    // paths against the ambient workspace. Not one method on the driver
    // contract carries a filesystem path: `MemoryCore` takes namespace/key,
    // `MemoryTree` takes namespace/node_id, `MemoryDocuments` takes a
    // `NamespaceDocumentInput`. There is nothing here to validate, and running
    // a namespace string through a path validator would invent semantics
    // neither side agreed to. The path half of step 1 belongs to whatever
    // future family actually accepts a path.

    // ── Step 2: source scope as a query predicate ────────────────────────────

    /// The ambient per-turn source allowlist, in contract form.
    ///
    /// `None` is unrestricted. `Some` — including `Some` over an empty set —
    /// restricts: [`SourceScope`]'s own docs make an empty allow list deny all
    /// source-attributed content, which matches
    /// [`crate::openhuman::memory::source_scope`]'s empty-allowlist semantics
    /// exactly.
    ///
    /// This is read at the guard boundary and passed **down** into the driver
    /// so the allowlist becomes part of the query, never a post-filter over
    /// rows a `limit` already truncated.
    pub fn ambient_scope(&self) -> Option<SourceScope> {
        current_source_scope().map(SourceScope::new)
    }

    /// The scope a query actually runs under, given what the caller asked for.
    ///
    /// The ambient allowlist is an **upper bound**, never a default that an
    /// argument replaces. An earlier version returned `requested.or(ambient)`,
    /// which let a source-restricted turn widen itself back out: passing an
    /// explicit scope naming a collection the ambient allowlist did not contain
    /// made that explicit scope the sole query predicate, and the restriction
    /// the turn was running under vanished.
    ///
    /// So the two are intersected. Membership is decided by the ambient scope's
    /// own [`SourceScope::allows_source_id`] rule (equality or the engine's
    /// `mem_src:{allowed}:` prefix), so the guard and the driver's SQL agree on
    /// what "in scope" means rather than the guard inventing a second rule.
    ///
    /// An empty intersection is returned as an empty `Some`, not `None`: an
    /// empty allow list denies all source-attributed content, which is the
    /// fail-closed reading [`SourceScope`] documents. Returning `None` there
    /// would turn "you asked for nothing you are allowed to see" into
    /// "unrestricted", the exact leak this method exists to close.
    pub fn narrow_scope(&self, requested: Option<&SourceScope>) -> Option<SourceScope> {
        match (requested, self.ambient_scope()) {
            (None, ambient) => ambient,
            (Some(requested), None) => Some(requested.clone()),
            (Some(requested), Some(ambient)) => {
                let allow: Vec<String> = requested
                    .allow
                    .iter()
                    .filter(|id| ambient.allows_source_id(id))
                    .cloned()
                    .collect();
                if allow.len() != requested.allow.len() {
                    log::debug!(
                        "[memory:guard] explicit source scope narrowed by the ambient \
                         allowlist requested={} admitted={}",
                        requested.allow.len(),
                        allow.len()
                    );
                }
                Some(SourceScope { allow })
            }
        }
    }

    // ── Step 3: taint stamping ───────────────────────────────────────────────

    /// The provenance the guard stamps on a write.
    ///
    /// The contract is explicit that the driver never assigns provenance, and
    /// that the single failure mode the guard exists to prevent is *laundering*
    /// externally-sourced content into internal-trust content.
    ///
    /// So this is a monotone raise, not a plain override: the result is
    /// [`MemoryTaint::ExternalSync`] when the caller asked for it **or** when
    /// the turn is running under a source scope (a source-restricted turn is
    /// by definition handling source-attributed content), and
    /// [`MemoryTaint::Internal`] only when neither holds. A pure override would
    /// happily rewrite a caller's `ExternalSync` down to `Internal` outside a
    /// scope, which is precisely the laundering step.
    pub fn stamp_taint(&self, requested: MemoryTaint) -> MemoryTaint {
        if requested == MemoryTaint::ExternalSync || current_source_scope().is_some() {
            MemoryTaint::ExternalSync
        } else {
            MemoryTaint::Internal
        }
    }

    // ── Step 4: redaction ────────────────────────────────────────────────────

    /// Content on its way to the driver, redacted when the driver is external.
    ///
    /// **No-op for [`DriverClass::Embedded`] and [`DriverClass::Null`]**:
    /// nothing leaves the device, and scrubbing in-process memory writes would
    /// silently destroy the user's own data. The borrowed arm is what makes
    /// that a byte-identical pass-through rather than a re-allocation that
    /// merely happens to compare equal.
    ///
    /// For [`DriverClass::External`] the content goes through the same
    /// conservative secret/PII scrubber every other host write path uses
    /// (`memory::safety::sanitize_text`).
    ///
    /// **Do not substitute `memory::util::redact::redact` here.** That function
    /// is a *log* redactor: it returns an 8-hex-character SHA-256 prefix, so
    /// using it on egress content would not redact the write, it would delete
    /// it. `redact` belongs in log lines only, and that is where
    /// [`super::audit`] uses it.
    ///
    /// The external arm is unreachable today — `binding::admit` refuses every
    /// external driver, so none can be bound — and is exercised for real in M6
    /// when the http adapter lands. It ships now so the branch exists at the
    /// same time as the class check that selects it.
    pub fn redact_outbound<'a>(&self, content: &'a str) -> Cow<'a, str> {
        match self.class {
            DriverClass::Embedded | DriverClass::Module | DriverClass::Null => {
                Cow::Borrowed(content)
            }
            DriverClass::External => {
                Cow::Owned(crate::openhuman::memory::safety::sanitize_text(content).value)
            }
        }
    }

    /// [`Self::redact_outbound`] for structured payloads (KV values, document
    /// metadata), via `memory::safety::sanitize_json`. Same class rule: an
    /// unmodified pass-through for embedded and null drivers.
    pub fn redact_outbound_json(&self, value: serde_json::Value) -> serde_json::Value {
        match self.class {
            DriverClass::Embedded | DriverClass::Module | DriverClass::Null => value,
            DriverClass::External => crate::openhuman::memory::safety::sanitize_json(&value).value,
        }
    }

    // ── Step 5: egress budget + trust state ──────────────────────────────────

    /// Per-call egress gate for an external driver.
    ///
    /// Bind-time refusal already covers the trust rule (`binding::admit` sends
    /// every `trust_state != "trusted"` external driver to the fallback), so
    /// this is the *per-call* seam and nothing more: it re-checks trust so a
    /// guard constructed some other way cannot skip it, and it discloses the
    /// transfer through the existing privacy-egress machinery.
    ///
    /// Deliberately minimal. There is no external driver in the tree yet, so
    /// building byte counters and a rolling budget here would be accounting for
    /// traffic that cannot exist; the real budget lands in M6 alongside the
    /// transport that can spend it.
    ///
    /// `carries_content` selects the disclosed [`DataKind`]:
    /// [`DataKind::FileContent`] for calls that hand raw memory bodies across
    /// the boundary, [`DataKind::Metadata`] for the rest. Neither is a perfect
    /// fit — there is no `MemoryContent` kind today — and adding one is an M6
    /// decision to take with the adapter, not a guess to bake in now.
    ///
    /// # Errors
    ///
    /// [`MemoryError::Invalid`] when the driver is external and its
    /// `trust_state` has not been explicitly raised.
    pub fn check_egress(&self, method: &str, carries_content: bool) -> Result<(), MemoryError> {
        if self.class != DriverClass::External {
            return Ok(());
        }
        if self.trust_state != TRUSTED {
            return Err(self.denied(
                method,
                format!(
                    "external driver '{}' is untrusted (trust_state = \"{}\"): \
                     set trust_state = \"{TRUSTED}\" under [subsystems.memory.drivers] \
                     before memory may cross the process boundary",
                    self.driver_id, self.trust_state
                ),
            ));
        }
        emit_external_transfer(EgressDescriptor::new(
            self.driver_id.clone(),
            method.to_string(),
            true,
            EgressReason::Integration,
            if carries_content {
                vec![DataKind::FileContent]
            } else {
                vec![DataKind::Metadata]
            },
        ));
        Ok(())
    }

    // ── Step 6: char budgets ─────────────────────────────────────────────────

    /// The recall char budget, or `None` when it is disabled.
    ///
    /// `0` reads as "no budget" rather than "return nothing": a zero-length
    /// budget that silently emptied every recall would be indistinguishable
    /// from a broken driver, and the config's own default is 1000.
    pub fn recall_budget(&self) -> Option<usize> {
        (self.hooks.recall_max_chars > 0).then_some(self.hooks.recall_max_chars)
    }

    /// The capture char budget, or `None` when it is disabled. Same zero rule
    /// as [`Self::recall_budget`].
    pub fn capture_budget(&self) -> Option<usize> {
        (self.hooks.capture_max_chars > 0).then_some(self.hooks.capture_max_chars)
    }

    // `max_context_tokens` is deliberately NOT enforced here. It is a
    // context-*assembly* budget — how much recalled text an agent turn may
    // inject into a prompt — and no method on the driver contract assembles a
    // prompt. Enforcing it in the guard would mean inventing a tokenizer and
    // applying it to a `Vec<MemoryEntry>` that has not been rendered yet. It
    // belongs to the auto-recall hook (M5), which is the thing that actually
    // builds the context block.
}

/// Wrap `policy` for sharing with the family decorators.
pub fn shared(policy: GuardPolicy) -> Arc<GuardPolicy> {
    Arc::new(policy)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
