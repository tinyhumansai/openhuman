# Review new core process statics

Run `pnpm rust:process-statics` from the repository root. The checks lane runs
the same command's script on every change. CI must fetch the base branch and
its history. A missing merge base fails the check.

The gate compares tracked production Rust sources under
`crates/openhuman-core/src/` with the base branch's merge base. In GitHub PRs,
the base is `origin/$GITHUB_BASE_REF`; locally it prefers `upstream/main`, then
`origin/main`. Supply a different base explicitly with
`node scripts/ci/check-process-statics.mjs <base-ref>`.

Every added declaration or changed declared type requires a record in
`scripts/ci/process-statics.json`. All declared types are checked, including
immutable statics and aliases: an alias can conceal a mutable container.
Existing declarations are not retrospectively approved by this gate. The
empty initial manifest does not certify their isolation.

Each record has these fields:

```json
{
  "file": "crates/openhuman-core/src/example/store.rs",
  "scope": "",
  "name": "CACHE",
  "type": "OnceLock<Mutex<Cache>>",
  "count": 1,
  "category": "B",
  "reason": "Every key includes the authenticated profile and agent scope.",
  "verification": "example/store_tests.rs: profiles_cannot_share_entries"
}
```

Use the type printed by the failure message, with redundant whitespace outside literals
removed. Separators that distinguish identifiers, literal prefixes/suffixes,
raw identifiers, numeric tokens, or joined punctuation remain (as do the `mut ` and `ref ` prefixes). ABI and other literal contents,
including their whitespace, remain significant. `count` is the exact number of matching
file/module-scope/name/type declarations. `scope` is the inline module path,
such as `outer::user`; an empty or omitted scope denotes the file root.
Swapping types between inline modules requires separate ownership review.
Duplicate records, missing fields, and records that no longer match the
source fail. Update or remove the record when its declaration changes.
Records present in the base manifest remain mandatory while their corresponding
declaration survives; deleting only an ownership record fails.

Choose the ownership category and explain why it applies:

| Category | Ownership                                                       |
| -------- | --------------------------------------------------------------- |
| A        | Process configuration or infrastructure; carries no tenant data |
| B        | Tenant data explicitly partitioned by authenticated scope       |
| C        | Durable authoritative state reached through a storage driver    |
| D        | A routing handle whose target enforces the ownership boundary   |

Reviewers must inspect the reason and verification reference. The gate
checks their presence and the source declaration; it does not execute the
referenced test or prove the category is correct.

## Bounds

This is a source declaration guard, not a Rust macro expander. It recognizes
ordinary `static`, `static mut`, and `lazy_static!`'s `static ref`, multiline
and qualified types, and ignores comments and string fixtures. Statics
generated without a literal declaration by another macro need manual review.
Literal declarations inside `macro_rules!` templates are conservatively checked
even when the template is not invoked. Excluding these bodies would allow a new
macro-defined global to bypass ownership review; the gate does not claim that
every source declaration is instantiated at runtime.
Type-position macro token trees are preserved without expanding them.
Separated punctuation is conservatively significant: harmless formatting can
require a renewed ownership record even when the effective Rust type is unchanged. Named
inline modules contribute declaration identity; anonymous blocks and function
scopes within one module still share its declaration counts.
Sibling `*_tests.rs` files are excluded; declarations behind `cfg(test)` in
production source files are conservatively included. Initializer and alias-definition
edits to existing declarations are subject to ordinary code review. Other crates and
vendored repositories are outside this core-only gate.
