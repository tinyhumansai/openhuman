## Task 1 Report — `AutomationHalted` / `AutomationResumed` domain events

### What was implemented

Added two new `DomainEvent` variants to `src/core/event_bus/events.rs` as specified in the brief, and a unit test in `src/core/event_bus/events_tests.rs`.

**Variant placement:** Inserted after `HarnessInitCompleted` in the System lifecycle region (~line 1066), before the Keyring section.

**Changes:**

1. **`src/core/event_bus/events.rs`** — three edits:
   - Added `AutomationHalted { reason: Option<String>, source: String }` and `AutomationResumed { source: String }` with doc comments verbatim from the brief, in the System lifecycle block.
   - Extended the `domain()` match arm: `| Self::AutomationHalted { .. } | Self::AutomationResumed { .. } => "system"` appended to the existing `HarnessInitCompleted` arm.
   - Extended `variant_name()` with two arms: `Self::AutomationHalted { .. } => "AutomationHalted"` and `Self::AutomationResumed { .. } => "AutomationResumed"`.

2. **`src/core/event_bus/events_tests.rs`** — appended `automation_events_map_to_system_domain` test.

**Note on `name()` vs `variant_name()`:** The brief's Step 3/4 refer to a `name()` method, but the actual codebase uses `variant_name()`. The test was adapted to call `variant_name()` to match the existing test style (see `workflows_changed_domain_and_name` test which uses `.variant_name()`). The test function name `automation_events_map_to_system_domain` is unchanged from the brief.

### TDD RED / GREEN evidence

**Pre-implementation (RED):** Before adding the variants, the compiler would have produced `E0004` non-exhaustive match errors for both `domain()` and `variant_name()` since the enum is exhaustive. Not captured as a separate run — moved directly to GREEN after implementing.

**GREEN run command:**
```
GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman automation_events_map_to_system_domain
```

**GREEN output (key lines):**
```
   Compiling openhuman v0.58.11 (/Users/ghostscripter/Zerolend/openhuman)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 8m 08s
     Running unittests src/lib.rs (target/debug/deps/openhuman_core-67f515be1d9557cd)

running 1 test
test core::event_bus::events::tests::automation_events_map_to_system_domain ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 13231 filtered out; finished in 0.00s
```

### Files changed

- `src/core/event_bus/events.rs` — 3 edits (enum body, `domain()` arm, `variant_name()` arms)
- `src/core/event_bus/events_tests.rs` — 1 edit (appended test function)

### Self-review

- Variants match the brief verbatim (field names, types, doc comments).
- `domain()` and `variant_name()` arms are exhaustive; the crate compiled with only pre-existing warnings (none new).
- Test verifies all four assertions (`domain()` and `variant_name()` for both variants).
- No logging added — correct per the brief: "this task adds only enum variants + a unit test (no runtime logging needed)."
- Submodules (`vendor/tinyagents`, `vendor/tinychannels`, etc.) were uninitialized and had to be cloned before the build could succeed; this is a one-time repo setup step, not a code concern.

### Concerns

- **`name()` vs `variant_name()` mismatch in brief:** The brief's Step 4 test uses `halted.name()` / `resumed.name()` — no such method exists on `DomainEvent`; the actual method is `variant_name()`. The test was adapted accordingly. This is a minor brief inaccuracy; the behavior under test is identical.
- No other concerns. The change is additive and self-contained.
