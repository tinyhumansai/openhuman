use super::*;

#[test]
fn layout_uses_agent_id_paths() {
    let ws = Path::new("/r/workspace");
    let layout = AgentLayout::resolve(ws, "alpha", PathBuf::from("/r/a"));
    assert_eq!(layout.home, Path::new("/r/workspace/agents/alpha"));
    assert_eq!(layout.skills, Path::new("/r/workspace/agents/alpha/skills"));
    assert_eq!(
        layout.transcripts,
        Path::new("/r/workspace/agents/alpha/session_raw")
    );
    assert_eq!(layout.action_dir, Path::new("/r/a"));
}

#[test]
fn default_action_dir_is_a_workspace_sibling_or_an_agent_dir() {
    assert_eq!(
        AgentLayout::default_action_dir(Path::new("/r"), Path::new("/r/action"), false, "a"),
        Path::new("/r/agents/a/action")
    );
    assert_eq!(
        AgentLayout::default_action_dir(
            Path::new("/home/u/.openhuman"),
            Path::new("/home/u/OpenHuman/projects"),
            true,
            "a"
        ),
        Path::new("/home/u/OpenHuman/projects/agents/a")
    );
}
