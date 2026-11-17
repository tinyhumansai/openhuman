//! Host-tool exposure checks for the standalone SaaS CLI.
//!
//! The CLI host does not install embedder-provided `ProfileCapabilities`, so
//! an operator's `host_files` allowlist cannot create a host-tool adapter by
//! itself. Verify the remote web-chat surface stays free of those tools.

use super::mock_llm::mock_llm;
use super::world::{credential, USERS};
use super::*;

#[test]
fn saas_cli_does_not_expose_host_files_without_a_host_adapter() {
    let deployment = deployment(true);
    let llm = mock_llm();
    let node = start_node(
        &deployment,
        "host-files",
        None,
        "tool_allowlist = [\"host_files\"]\n",
        Some(llm.port),
    );

    provision(&client(), &node.base, USERS[0]);
    let provisioned = operator(
        &node,
        "openhuman.profiles_set_credential",
        json!({
            "profile_id": USERS[0],
            "kind": "session",
            "token": credential(0),
        }),
    );
    assert!(provisioned.get("result").is_some(), "{provisioned}");

    let (status, _, body) = call(
        &node,
        USERS[0],
        "openhuman.channel_web_chat",
        json!({
            "client_id": "host-files-client",
            "thread_id": "host-files-thread",
            "message": "What tools can you use?",
        }),
    );
    assert_eq!(status, 200, "{body}");
    assert!(body.get("result").is_some(), "{body}");

    wait_until(
        "host-files inference request",
        &node,
        Duration::from_secs(60),
        || llm.recorded().iter().any(|request| request.is_inference()),
    );
    let requests = llm.recorded();
    assert!(
        requests.iter().any(|request| request.is_inference()),
        "the tool exposure assertion requires an inference request"
    );
    let offered: Vec<_> = requests
        .iter()
        .filter(|request| request.is_inference())
        .flat_map(|request| request.offered_tools())
        .collect();
    for tool in [
        "file_read",
        "file_write",
        "apply_patch",
        "grep",
        "glob",
        "list",
    ] {
        assert!(
            !offered.iter().any(|offered| offered == tool),
            "the CLI has no host-tool adapter, so {tool} must not be offered: {offered:?}"
        );
    }
}
