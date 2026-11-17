//! OpenHuman's product-specific system-prompt selection and additions.

use crate::agent::prompts::types::PromptContext;
use crate::prompts::sections::*;
use anyhow::{anyhow, Result};
use std::path::Path;
use tinyagents_harness::prompt::{
    DynamicPromptSection as OwnerDynamicPromptSection, PromptComposer, PromptTier, TieredPromptPart,
};

/// Bundled seed for the workspace's editable writing-style rules.
pub const GLOBAL_STYLE_SUFFIX: &str = include_str!("../agent/prompts/STYLE.md");

fn global_style_block(workspace_dir: &Path) -> String {
    crate::prompts::sync_workspace_file(workspace_dir, "STYLE.md");
    std::fs::read_to_string(workspace_dir.join("STYLE.md")).unwrap_or_else(|error| {
        tracing::warn!(
            "[style] could not read workspace STYLE.md ({error}); falling back to the bundled copy"
        );
        GLOBAL_STYLE_SUFFIX.to_string()
    })
}

#[derive(Clone)]
enum ProductSection {
    Archetype(String),
    Dynamic(crate::agent_registry::host_definition::PromptBuilder),
    Identity,
    AgentsInstructions,
    Tools,
    Safety,
    Workspace,
    DateTime,
    Runtime,
}

impl<'a> OwnerDynamicPromptSection<PromptContext<'a>> for ProductSection {
    fn name(&self) -> &str {
        match self {
            Self::Archetype(_) => "archetype_prompt",
            Self::Dynamic(_) => "dynamic_prompt",
            Self::Identity => "identity",
            Self::AgentsInstructions => "agents_md",
            Self::Tools => "tools",
            Self::Safety => "safety",
            Self::Workspace => "workspace",
            Self::DateTime => "datetime",
            Self::Runtime => "runtime",
        }
    }

    fn render(
        &self,
        context: &PromptContext<'a>,
    ) -> tinyagents_harness::Result<Vec<TieredPromptPart>> {
        match self {
            Self::Archetype(body) => ArchetypePromptSection::new(body.clone()).render(context),
            Self::Dynamic(builder) => {
                crate::prompts::sections::DynamicPromptSection::new(*builder).render(context)
            }
            Self::Identity => IdentitySection.render(context),
            Self::AgentsInstructions => AgentsInstructionsSection.render(context),
            Self::Tools => ToolsSection.render(context),
            Self::Safety => SafetySection.render(context),
            Self::Workspace => WorkspaceSection.render(context),
            Self::DateTime => DateTimeSection.render(context),
            Self::Runtime => RuntimeSection.render(context),
        }
    }
}

struct OpenHumanPromptContent {
    sections: Vec<ProductSection>,
    verbatim: bool,
}

impl<'a> OwnerDynamicPromptSection<PromptContext<'a>> for OpenHumanPromptContent {
    fn name(&self) -> &str {
        "openhuman_prompt_content"
    }

    fn render(
        &self,
        context: &PromptContext<'a>,
    ) -> tinyagents_harness::Result<Vec<TieredPromptPart>> {
        let mut parts = Vec::new();
        let mut has_grounding = false;
        for section in &self.sections {
            for part in section.render(context)? {
                if !part.content.trim().is_empty() && part.content.contains(GROUNDING_HEADING) {
                    has_grounding = true;
                }
                parts.push(part);
            }
        }

        if !self.verbatim {
            if !has_grounding {
                parts.push(TieredPromptPart::new(
                    PromptTier::Stable,
                    GROUNDING_BODY.trim_end(),
                ));
            }
            parts.push(TieredPromptPart::new(
                PromptTier::Stable,
                global_style_block(context.workspace_dir).trim_end(),
            ));
        }
        Ok(parts)
    }
}

/// Product prompt parts returned in the shape consumed by the session host.
///
/// The underlying text and tier layout come directly from TinyAgents' prompt
/// composer. This compatibility view keeps OpenHuman's historical grouping of
/// Stable+Context into the first system message and Volatile into the second.
#[derive(Debug, Clone, Default)]
pub struct TieredPrompt {
    pub text: String,
    pub breakpoints: Vec<usize>,
    pub parts: Vec<(PromptTier, String)>,
}

impl TieredPrompt {
    #[must_use]
    pub fn system_messages(&self) -> Vec<String> {
        let mut head = String::new();
        let mut tail = String::new();
        for (tier, part) in &self.parts {
            match tier {
                PromptTier::Stable | PromptTier::Context => head.push_str(part),
                PromptTier::Volatile => tail.push_str(part),
            }
        }
        let mut messages = Vec::with_capacity(2);
        if !head.trim().is_empty() {
            messages.push(head.trim_end().to_string());
        }
        if !tail.trim().is_empty() {
            messages.push(tail.trim_end().to_string());
        }
        messages
    }
}

/// Selects OpenHuman-owned prompt content, then delegates tier assembly to
/// TinyAgents' `PromptComposer`.
#[derive(Clone)]
pub struct SystemPromptBuilder {
    sections: Vec<ProductSection>,
    verbatim: bool,
}

impl SystemPromptBuilder {
    pub fn with_defaults() -> Self {
        Self {
            sections: vec![
                ProductSection::Identity,
                ProductSection::AgentsInstructions,
                ProductSection::Tools,
                ProductSection::Safety,
                ProductSection::Workspace,
                ProductSection::DateTime,
                ProductSection::Runtime,
            ],
            verbatim: false,
        }
    }

    pub fn for_subagent(
        archetype_prompt_text: String,
        omit_identity: bool,
        omit_safety_preamble: bool,
    ) -> Self {
        let mut sections = vec![ProductSection::Archetype(archetype_prompt_text)];
        if !omit_identity {
            sections.push(ProductSection::Identity);
        }
        sections.push(ProductSection::AgentsInstructions);
        sections.push(ProductSection::Tools);
        if !omit_safety_preamble {
            sections.push(ProductSection::Safety);
        }
        sections.push(ProductSection::Workspace);
        Self {
            sections,
            verbatim: false,
        }
    }

    pub fn from_final_body(body: String) -> Self {
        Self {
            sections: vec![ProductSection::Archetype(body)],
            verbatim: false,
        }
    }

    pub fn verbatim(body: String) -> Self {
        Self {
            sections: vec![ProductSection::Archetype(body)],
            verbatim: true,
        }
    }

    pub fn from_dynamic(builder: crate::agent_registry::host_definition::PromptBuilder) -> Self {
        Self {
            sections: vec![
                ProductSection::Dynamic(builder),
                ProductSection::AgentsInstructions,
            ],
            verbatim: false,
        }
    }

    pub fn build(&self, context: &PromptContext<'_>) -> Result<String> {
        Ok(self.build_tiered(context)?.text)
    }

    pub fn build_tiered(&self, context: &PromptContext<'_>) -> Result<TieredPrompt> {
        let mut composer = PromptComposer::new();
        composer.push(OpenHumanPromptContent {
            sections: self.sections.clone(),
            verbatim: self.verbatim,
        });
        let mut rendered = composer
            .build(context)
            .map_err(|error| anyhow!(error.to_string()))?;
        // Preserve the old `verbatim("")` flattened output. Provider message
        // boundaries remain empty because the owner composer emits no parts.
        if rendered.text.is_empty() {
            rendered.text.push('\n');
        }
        Ok(TieredPrompt {
            text: rendered.text,
            breakpoints: rendered.breakpoints,
            parts: rendered.parts,
        })
    }
}
