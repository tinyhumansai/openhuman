//! Run one OpenHuman agent turn as a library call.
//!
//! The counterpart to `embed_headless.rs`: that one shows how to *build* a core
//! and dispatch plumbing methods, this one shows the thing an embedder actually
//! wants — a prompt in, a reply out, with the model, the workspace and the
//! access tier chosen in code rather than discovered from the environment.
//!
//! ```bash
//! # Against any OpenAI-compatible endpoint:
//! OPENHUMAN_EXAMPLE_BASE_URL=https://api.openai.com/v1 \
//! OPENHUMAN_EXAMPLE_API_KEY=sk-… \
//! OPENHUMAN_EXAMPLE_MODEL=gpt-5 \
//!   cargo run --example run_turn -- "What can you see in this directory?"
//!
//! # Or against the machine's own configured inference, in its real workspace:
//! OPENHUMAN_EXAMPLE_INHERIT=1 cargo run --example run_turn -- "Hello."
//! ```
//!
//! Optional: `OPENHUMAN_EXAMPLE_BACKEND_URL` points the core's non-inference
//! backend calls somewhere, and `OPENHUMAN_EXAMPLE_SKILLS_DIR` supplies skill
//! bundles.
//!
//! Note the runtime is built by hand rather than with `#[tokio::main]`. That is
//! not incidental — see [`main`].

use std::path::PathBuf;

use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use openhuman_core::{Access, Harness, Provider, Workspace};

fn main() -> anyhow::Result<()> {
    // Library embedders own logging. `RUST_LOG=debug` shows the `[embed]` and
    // `[embed][harness]` lines this crate emits around a turn.
    let _ = env_logger::builder().is_test(false).try_init();

    // A turn is a very large async state machine, and delegating to a sub-agent
    // nests another inside it. tokio's default 2 MiB worker stack overflows and
    // aborts the process — `#[tokio::main]` gives you that default, which is why
    // this example does not use it. Every host that runs a turn must set these.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()?;

    runtime.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Introduce yourself in one sentence.".to_string());

    let inherit = std::env::var_os("OPENHUMAN_EXAMPLE_INHERIT").is_some();

    let mut builder = Harness::builder()
        // Read-only by default: this example should be safe to point at any
        // directory. Swap for `Access::full()` to let the agent actually act —
        // and read that method's docs first, because it means real shell
        // commands and real file edits under `action_dir`.
        .access(Access::readonly());

    builder = if inherit {
        // Reuse the machine's real workspace and whatever inference it is
        // already configured with — sessions persist, and the agent sees the
        // memory the desktop app wrote.
        builder
            .workspace(Workspace::Inherit)
            .provider(Provider::inherit())
    } else {
        // A throwaway workspace, removed when the harness drops, and an
        // explicitly named endpoint. Nothing here touches ~/.openhuman.
        let base_url = std::env::var("OPENHUMAN_EXAMPLE_BASE_URL").map_err(|_| {
            anyhow::anyhow!(
                "set OPENHUMAN_EXAMPLE_BASE_URL + OPENHUMAN_EXAMPLE_API_KEY, \
                 or set OPENHUMAN_EXAMPLE_INHERIT=1 to use this machine's own \
                 configured inference"
            )
        })?;
        let api_key = std::env::var("OPENHUMAN_EXAMPLE_API_KEY").map_err(|_| {
            anyhow::anyhow!("OPENHUMAN_EXAMPLE_API_KEY is required with a base URL")
        })?;
        let model =
            std::env::var("OPENHUMAN_EXAMPLE_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

        let mut builder = builder
            .workspace(Workspace::Ephemeral)
            .provider(Provider::openai_compatible(base_url, api_key).model(model))
            // Let the agent's file tools look at the current directory.
            .action_dir(std::env::current_dir()?);

        // The core still makes non-inference backend calls. Signed out of the
        // real one, those are rejected — and a rejection publishes
        // Point them at your own backend if you have one. Authenticated
        // backend features additionally need a real Session::backend value;
        // caller-supplied inference itself does not.
        if let Ok(url) = std::env::var("OPENHUMAN_EXAMPLE_BACKEND_URL") {
            builder = builder.backend_url(url);
        }
        builder
    };

    // Skills are opt-in and copied into the harness's workspace; see the
    // builder method's docs for why they are copied rather than linked.
    if let Some(dir) = std::env::var_os("OPENHUMAN_EXAMPLE_SKILLS_DIR") {
        #[cfg(feature = "skills")]
        {
            builder = builder.skills_dir(PathBuf::from(dir));
        }
        #[cfg(not(feature = "skills"))]
        {
            let _ = PathBuf::from(dir);
            eprintln!("this build has no `skills` feature; ignoring the skills directory");
        }
    }

    let harness = builder.build().await?;
    println!("workspace: {}", harness.workspace_dir().display());
    println!("action dir: {}", harness.action_dir().display());

    // Stream the turn as it runs. Without a sink the call resolves to one final
    // string and shows nothing in between. The core *awaits* its sends, so this
    // channel is real backpressure — a receiver that stops draining stalls the
    // turn.
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let printer = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            eprintln!("[progress] {progress:?}");
        }
    });

    let outcome = harness.turn(&prompt).on_progress(tx).send().await?;
    // Wait for the printer to drain, but bound it. A detached async sub-agent
    // (`spawn_async_subagent`) can carry a clone of the sender past `send()`
    // returning, so waiting on sender-drop alone could hang indefinitely.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), printer).await;

    println!("\nsession: {}", outcome.session_id);
    println!("{}", outcome.reply);

    // Pass `outcome.session_id` to `.session(..)` on a later turn to continue
    // this conversation.
    Ok(())
}
