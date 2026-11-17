use super::*;

/// Every group must be a decision in `StoreInitPlan`: either it owns a store
/// field, or it is explicitly declared store-less here. A new family that owns
/// a store but is not keyed will fail this until it is listed.
#[test]
fn every_domain_group_is_accounted_for_in_store_init_plan() {
    use crate::core::runtime::context::StoreInitPlan;

    // Groups that own a store field in StoreInitPlan.
    const OWNS_STORE: &[DomainGroup] =
        &[DomainGroup::Memory, DomainGroup::Agent, DomainGroup::Skills];
    // Groups with no store of their own. Adding a variant forces a choice
    // between these two lists — that is the point.
    const STORELESS: &[DomainGroup] = &[
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Flows,
        DomainGroup::Mcp,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Inference,
        DomainGroup::Integrations,
        DomainGroup::Automation,
        DomainGroup::Runtimes,
        DomainGroup::Desktop,
        DomainGroup::Hosted,
        // The registry is a compiled-in `const` table and the loaded-module set
        // lives in tinybus's own `ModuleHost`, so there is nothing for
        // `init_stores` to stand up.
        DomainGroup::Modules,
        DomainGroup::Platform,
        // User agents live on disk under the SaaS root and are opened by the
        // agent host on demand; nothing is stood up at boot.
        DomainGroup::Operator,
    ];

    for g in DomainGroup::ALL {
        let owns = OWNS_STORE.contains(g);
        let storeless = STORELESS.contains(g);
        assert!(
            owns ^ storeless,
            "{g:?} is in neither (or both) of OWNS_STORE / STORELESS — decide \
             whether it needs a StoreInitPlan field and list it in exactly one"
        );
    }

    // And the owning groups actually gate their field: turning the group off
    // must turn the store off.
    let mut only_memory = crate::core::runtime::DomainSet::none();
    only_memory.memory = true;
    let plan = StoreInitPlan::for_domains(only_memory);
    assert!(plan.memory, "Memory on ⇒ memory store initialized");
    assert!(!plan.agent_attachments, "Agent off ⇒ attachments store off");
    assert!(!plan.skills_prune, "Skills off ⇒ skills prune off");
}

/// Same contract for `DomainSubscriberPlan`: every group either registers
/// subscribers or is declared subscriber-less.
#[test]
fn every_domain_group_is_accounted_for_in_subscriber_plan() {
    use crate::core::runtime::subscribers::DomainSubscriberPlan;

    const REGISTERS: &[DomainGroup] = &[
        DomainGroup::Platform,
        DomainGroup::Channels,
        DomainGroup::Flows,
        DomainGroup::Memory,
        DomainGroup::Agent,
        DomainGroup::Mcp,
        DomainGroup::Integrations,
        DomainGroup::Security,
        DomainGroup::Desktop,
        DomainGroup::Skills,
    ];
    const NO_SUBSCRIBERS: &[DomainGroup] = &[
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Inference,
        DomainGroup::Automation,
        DomainGroup::Runtimes,
        DomainGroup::Hosted,
        // Modules run on their own in-process broker, so they cannot publish a
        // `DomainEvent` and there is nothing on the core bus to subscribe to.
        DomainGroup::Modules,
        // The operator plane only answers provisioning calls.
        DomainGroup::Operator,
    ];

    for g in DomainGroup::ALL {
        assert!(
            REGISTERS.contains(g) ^ NO_SUBSCRIBERS.contains(g),
            "{g:?} is in neither (or both) of REGISTERS / NO_SUBSCRIBERS — decide \
             whether it registers event-bus subscribers and list it in exactly one"
        );
    }

    // full() must enable every registering group; none() must enable none.
    let full = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::full());
    let none = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::none());
    assert_ne!(full, none, "full() and none() must differ");
}
