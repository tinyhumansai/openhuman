use crate::openhuman::agent::host_runtime;
use crate::openhuman::approval::ApprovalManager;
use crate::openhuman::config::Config;
use crate::openhuman::memory::{self, Memory, MemoryCategory};
use crate::openhuman::providers::{self, ChatMessage, Provider};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::io::Write as _;
use std::sync::Arc;

use super::history::{auto_compact_history, autosave_memory_key, trim_history};
use super::instructions::build_tool_instructions;
use super::memory_context::build_context;
use super::tool_loop::{agent_turn, run_tool_call_loop};

#[allow(clippy::too_many_lines)]
pub async fn run(
    config: Config,
    message: Option<String>,
    model_override: Option<String>,
    temperature: f64,
    peripheral_overrides: Vec<String>,
) -> Result<String> {
    // ── Wire up agnostic subsystems ──────────────────────────────
    let runtime: Arc<dyn host_runtime::RuntimeAdapter> =
        Arc::from(host_runtime::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));

    // ── Memory (the brain) ────────────────────────────────────────
    let mem: Arc<dyn Memory> = Arc::from(memory::create_memory_with_storage(
        &config.memory,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        config.api_key.as_deref(),
    )?);
    tracing::info!(backend = mem.name(), "Memory initialized");

    // ── Peripherals (merge peripheral tools into registry) ─
    if !peripheral_overrides.is_empty() {
        tracing::info!(
            peripherals = ?peripheral_overrides,
            "Peripheral overrides from runtime flags (config boards take precedence)"
        );
    }

    // ── Tools (including memory tools and peripherals) ────────────
    let (composio_key, composio_entity_id) = if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    };
    let mut tools_registry = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        mem.clone(),
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &config.workspace_dir,
        &config.agents,
        config.api_key.as_deref(),
        &config,
    );

    let peripheral_tools: Vec<Box<dyn Tool>> =
        tools::create_peripheral_tools(&config.peripherals).await?;
    if !peripheral_tools.is_empty() {
        tracing::info!(count = peripheral_tools.len(), "Peripheral tools added");
        tools_registry.extend(peripheral_tools);
    }

    // ── Inference (OpenHuman backend only) ───────────────────────
    let provider_name = providers::INFERENCE_BACKEND_ID;

    let model_name = model_override
        .as_deref()
        .or(config.default_model.as_deref())
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    let provider_runtime_options = providers::ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };

    let provider: Box<dyn Provider> = providers::create_routed_provider_with_options(
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &config.model_routes,
        model_name,
        &provider_runtime_options,
    )?;

    // ── Build system prompt from workspace MD files (OpenClaw framework) ──
    let skills = crate::openhuman::skills::load_skills(&config.workspace_dir);
    let mut tool_descs: Vec<(&str, &str)> = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory_store",
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.",
        ),
        (
            "memory_recall",
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.",
        ),
        (
            "memory_forget",
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.",
        ),
    ];
    tool_descs.push((
        "cron_add",
        "Create a cron job. Supports schedule kinds: cron, at, every; and job types: shell or agent.",
    ));
    tool_descs.push((
        "cron_list",
        "List all cron jobs with schedule, status, and metadata.",
    ));
    tool_descs.push(("cron_remove", "Remove a cron job by job_id."));
    tool_descs.push((
        "cron_update",
        "Patch a cron job (schedule, enabled, command/prompt, model, delivery, session_target).",
    ));
    tool_descs.push((
        "cron_run",
        "Force-run a cron job immediately and record a run history entry.",
    ));
    tool_descs.push(("cron_runs", "Show recent run history for a cron job."));
    tool_descs.push((
        "screenshot",
        "Capture a screenshot of the current screen. Returns file path and base64-encoded PNG. Use when: visual verification, UI inspection, debugging displays.",
    ));
    tool_descs.push((
        "image_info",
        "Read image file metadata (format, dimensions, size) and optionally base64-encode it. Use when: inspecting images, preparing visual data for analysis.",
    ));
    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in Brave Browser (allowlist-only, no scraping)",
        ));
    }
    if config.composio.enabled {
        tool_descs.push((
            "composio",
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover, 'execute' to run (optionally with connected_account_id), 'connect' to OAuth.",
        ));
    }
    tool_descs.push((
        "schedule",
        "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a sub-task to a specialized agent. Use when: task needs different model/capability, or to parallelize work.",
        ));
    }
    if config.peripherals.enabled && !config.peripherals.boards.is_empty() {
        tool_descs.push((
            "gpio_read",
            "Read GPIO pin value (0 or 1) on connected hardware (STM32, Arduino). Use when: checking sensor/button state, LED status.",
        ));
        tool_descs.push((
            "gpio_write",
            "Set GPIO pin high (1) or low (0) on connected hardware. Use when: turning LED on/off, controlling actuators.",
        ));
        tool_descs.push((
            "arduino_upload",
            "Upload agent-generated Arduino sketch. Use when: user asks for 'make a heart', 'blink pattern', or custom LED behavior on Arduino. You write the full .ino code; OpenHuman compiles and uploads it. Pin 13 = built-in LED on Uno.",
        ));
        tool_descs.push((
            "hardware_memory_map",
            "Return flash and RAM address ranges for connected hardware. Use when: user asks for 'upper and lower memory addresses', 'memory map', or 'readable addresses'.",
        ));
        tool_descs.push((
            "hardware_board_info",
            "Return full board info (chip, architecture, memory map) for connected hardware. Use when: user asks for 'board info', 'what board do I have', 'connected hardware', 'chip info', or 'what hardware'.",
        ));
        tool_descs.push((
            "hardware_memory_read",
            "Read actual memory/register values from Nucleo via USB. Use when: user asks to 'read register values', 'read memory', 'dump lower memory 0-126', 'give address and value'. Params: address (hex, default 0x20000000), length (bytes, default 128).",
        ));
        tool_descs.push((
            "hardware_capabilities",
            "Query connected hardware for reported GPIO pins and LED pin. Use when: user asks what pins are available.",
        ));
    }
    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let mut system_prompt = crate::openhuman::channels::build_system_prompt(
        &config.workspace_dir,
        model_name,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
    );

    // Append structured tool-use instructions with schemas
    system_prompt.push_str(&build_tool_instructions(&tools_registry));

    // ── Approval manager (supervised mode) ───────────────────────
    let approval_manager = ApprovalManager::from_config(&config.autonomy);

    // ── Execute ──────────────────────────────────────────────────
    let mut final_output = String::new();

    if let Some(msg) = message {
        // Auto-save user message to memory
        if config.memory.auto_save {
            let user_key = autosave_memory_key("user_msg");
            let _ = mem
                .store(&user_key, &msg, MemoryCategory::Conversation, None)
                .await;
        }

        // Inject memory context into user message
        let mem_context =
            build_context(mem.as_ref(), &msg, config.memory.min_relevance_score).await;
        let context = mem_context;
        let enriched = if context.is_empty() {
            msg.clone()
        } else {
            format!("{context}{msg}")
        };

        let mut history = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&enriched),
        ];

        let response = run_tool_call_loop(
            provider.as_ref(),
            &mut history,
            &tools_registry,
            provider_name,
            model_name,
            temperature,
            false,
            Some(&approval_manager),
            "cli",
            &config.multimodal,
            config.agent.max_tool_iterations,
            None,
        )
        .await?;
        final_output = response.clone();
        println!("{response}");

        // Auto-save assistant response to daily log
        if config.memory.auto_save {
            let summary = truncate_with_ellipsis(&response, 100);
            let response_key = autosave_memory_key("assistant_resp");
            let _ = mem
                .store(&response_key, &summary, MemoryCategory::Daily, None)
                .await;
        }
    } else {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /help for commands.\n");
        let cli = crate::openhuman::channels::CliChannel::new();

        // Persistent conversation history across turns
        let mut history = vec![ChatMessage::system(&system_prompt)];

        loop {
            print!("> ");
            let _ = std::io::stdout().flush();

            let mut input = String::new();
            match std::io::stdin().read_line(&mut input) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("\nError reading input: {e}\n");
                    break;
                }
            }

            let user_input = input.trim().to_string();
            if user_input.is_empty() {
                continue;
            }
            match user_input.as_str() {
                "/quit" | "/exit" => break,
                "/help" => {
                    println!("Available commands:");
                    println!("  /help        Show this help message");
                    println!("  /clear /new  Clear conversation history");
                    println!("  /quit /exit  Exit interactive mode\n");
                    continue;
                }
                "/clear" | "/new" => {
                    println!(
                        "This will clear the current conversation and delete all session memory."
                    );
                    println!("Core memories (long-term facts/preferences) will be preserved.");
                    print!("Continue? [y/N] ");
                    let _ = std::io::stdout().flush();

                    let mut confirm = String::new();
                    if std::io::stdin().read_line(&mut confirm).is_err() {
                        continue;
                    }
                    if !matches!(confirm.trim().to_lowercase().as_str(), "y" | "yes") {
                        println!("Cancelled.\n");
                        continue;
                    }

                    history.clear();
                    history.push(ChatMessage::system(&system_prompt));
                    // Clear conversation and daily memory
                    let mut cleared = 0;
                    for category in [MemoryCategory::Conversation, MemoryCategory::Daily] {
                        let entries = mem.list(Some(&category), None).await.unwrap_or_default();
                        for entry in entries {
                            if mem.forget(&entry.key).await.unwrap_or(false) {
                                cleared += 1;
                            }
                        }
                    }
                    if cleared > 0 {
                        println!("Conversation cleared ({cleared} memory entries removed).\n");
                    } else {
                        println!("Conversation cleared.\n");
                    }
                    continue;
                }
                _ => {}
            }

            // Auto-save conversation turns
            if config.memory.auto_save {
                let user_key = autosave_memory_key("user_msg");
                let _ = mem
                    .store(&user_key, &user_input, MemoryCategory::Conversation, None)
                    .await;
            }

            // Inject memory context into user message
            let mem_context =
                build_context(mem.as_ref(), &user_input, config.memory.min_relevance_score).await;
            let context = mem_context;
            let enriched = if context.is_empty() {
                user_input.clone()
            } else {
                format!("{context}{user_input}")
            };

            history.push(ChatMessage::user(&enriched));

            let response = match run_tool_call_loop(
                provider.as_ref(),
                &mut history,
                &tools_registry,
                provider_name,
                model_name,
                temperature,
                false,
                Some(&approval_manager),
                "cli",
                &config.multimodal,
                config.agent.max_tool_iterations,
                None,
            )
            .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            };
            final_output = response.clone();
            if let Err(e) = crate::openhuman::channels::Channel::send(
                &cli,
                &crate::openhuman::channels::traits::SendMessage::new(
                    format!("\n{response}\n"),
                    "user",
                ),
            )
            .await
            {
                eprintln!("\nError sending console response: {e}\n");
            }

            // Auto-compaction before hard trimming to preserve long-context signal.
            if let Ok(compacted) = auto_compact_history(
                &mut history,
                provider.as_ref(),
                model_name,
                config.agent.max_history_messages,
                &config,
            )
            .await
            {
                if compacted {
                    println!("🧹 Auto-compaction complete");
                }
            }

            // Hard cap as a safety net.
            trim_history(&mut history, config.agent.max_history_messages);

            if config.memory.auto_save {
                let summary = truncate_with_ellipsis(&response, 100);
                let response_key = autosave_memory_key("assistant_resp");
                let _ = mem
                    .store(&response_key, &summary, MemoryCategory::Daily, None)
                    .await;
            }
        }
    }

    Ok(final_output)
}

/// Process a single message through the full agent (with tools, peripherals, memory).
/// Used by channels (Telegram, Discord, etc.) to enable hardware and tool use.
pub async fn process_message(config: Config, message: &str) -> Result<String> {
    let runtime: Arc<dyn host_runtime::RuntimeAdapter> =
        Arc::from(host_runtime::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let mem: Arc<dyn Memory> = Arc::from(memory::create_memory_with_storage(
        &config.memory,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        config.api_key.as_deref(),
    )?);

    let (composio_key, composio_entity_id) = if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    };
    let mut tools_registry = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        mem.clone(),
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &config.workspace_dir,
        &config.agents,
        config.api_key.as_deref(),
        &config,
    );
    let peripheral_tools: Vec<Box<dyn Tool>> =
        tools::create_peripheral_tools(&config.peripherals).await?;
    tools_registry.extend(peripheral_tools);

    let provider_name = providers::INFERENCE_BACKEND_ID;
    let model_name = config
        .default_model
        .clone()
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.into());
    let provider_runtime_options = providers::ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let provider: Box<dyn Provider> = providers::create_routed_provider_with_options(
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &config.model_routes,
        &model_name,
        &provider_runtime_options,
    )?;

    let skills = crate::openhuman::skills::load_skills(&config.workspace_dir);
    let mut tool_descs: Vec<(&str, &str)> = vec![
        ("shell", "Execute terminal commands."),
        ("file_read", "Read file contents."),
        ("file_write", "Write file contents."),
        ("memory_store", "Save to memory."),
        ("memory_recall", "Search memory."),
        ("memory_forget", "Delete a memory entry."),
        ("screenshot", "Capture a screenshot."),
        ("image_info", "Read image metadata."),
    ];
    if config.browser.enabled {
        tool_descs.push(("browser_open", "Open approved URLs in browser."));
    }
    if config.composio.enabled {
        tool_descs.push(("composio", "Execute actions on 1000+ apps via Composio."));
    }
    if config.peripherals.enabled && !config.peripherals.boards.is_empty() {
        tool_descs.push(("gpio_read", "Read GPIO pin value on connected hardware."));
        tool_descs.push((
            "gpio_write",
            "Set GPIO pin high or low on connected hardware.",
        ));
        tool_descs.push((
            "arduino_upload",
            "Upload Arduino sketch. Use for 'make a heart', custom patterns. You write full .ino code; OpenHuman uploads it.",
        ));
        tool_descs.push((
            "hardware_memory_map",
            "Return flash and RAM address ranges. Use when user asks for memory addresses or memory map.",
        ));
        tool_descs.push((
            "hardware_board_info",
            "Return full board info (chip, architecture, memory map). Use when user asks for board info, what board, connected hardware, or chip info.",
        ));
        tool_descs.push((
            "hardware_memory_read",
            "Read actual memory/register values from Nucleo. Use when user asks to read registers, read memory, dump lower memory 0-126, or give address and value.",
        ));
        tool_descs.push((
            "hardware_capabilities",
            "Query connected hardware for reported GPIO pins and LED pin. Use when user asks what pins are available.",
        ));
    }
    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let mut system_prompt = crate::openhuman::channels::build_system_prompt(
        &config.workspace_dir,
        &model_name,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
    );
    system_prompt.push_str(&build_tool_instructions(&tools_registry));

    let mem_context = build_context(mem.as_ref(), message, config.memory.min_relevance_score).await;
    let context = mem_context;
    let enriched = if context.is_empty() {
        message.to_string()
    } else {
        format!("{context}{message}")
    };

    let mut history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&enriched),
    ];

    agent_turn(
        provider.as_ref(),
        &mut history,
        &tools_registry,
        provider_name,
        &model_name,
        config.default_temperature,
        true,
        &config.multimodal,
        config.agent.max_tool_iterations,
    )
    .await
}
