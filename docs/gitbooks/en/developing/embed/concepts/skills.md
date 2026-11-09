---
description: "Skills are copied bundles of instructions and resources discovered inside an agent\u2019s own home."
---

# Skills

A skill bundle contains `SKILL.md` plus any referenced resources. `AgentSpec::skills_dir` copies bundles into the agent's `agents/<id>/skills/` tree. Copying is intentional: discovery rejects symlinked bundles, so linking a shared directory does not install it.

Enable the named Embed `skills` feature for skill setters and registry access. Library agents hide the operator's user skill directory unless `include_user_skills(true)` is explicitly selected. Treat skill installation as part of the agent's configuration, rather than reading arbitrary operator files into every tenant.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Skill registry seam and default user-skill policy | Copied bundles, discovery context and explicit user-skill opt-in |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/skills.rs#skills -->

```rust
    let skills = tempfile::tempdir()?;
    std::fs::create_dir(skills.path().join("summarize"))?;
    std::fs::write(skills.path().join("summarize/SKILL.md"),
        "---\nname: summarize\ndescription: Summarize documents clearly.\n---\nInclude the main points.\n")?;
    let agent = runtime.agent(AgentSpec::new("skilled").skills_dir(skills.path()))?;
    let copied = agent
        .workspace_dir()
        .join("agents/skilled/skills/summarize/SKILL.md");
    assert_eq!(
        std::fs::read_to_string(&copied)?,
        std::fs::read_to_string(skills.path().join("summarize/SKILL.md"))?
    );
    assert!(!std::fs::symlink_metadata(copied)?.file_type().is_symlink());
    assert!(!agent.run("Hello").await?.reply.is_empty());
    println!("skill bundle copied without symlinks");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/skills.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example writes a bundle, registers an agent, checks the copied contents and verifies that the destination is not a symlink. Use the same directory layout for production bundles; do not depend on the fixture's temporary directory lifetime.

A resumed session's prompt is frozen, so adding or editing a bundle does not rewrite an existing conversation's recorded prefix. Start a new session when you want updated instructions to become visible, or build the agent with `AgentDefinitionSpec::refresh_prompt_on_resume(true)` so a resumed session is sent its current prompt. See [turns and sessions](turns-sessions.md) and the generated [builder setters](../builder-setters.md) for runtime policy knobs.
