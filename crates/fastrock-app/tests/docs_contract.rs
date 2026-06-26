const DEVELOPMENT: &str = include_str!("../../../docs/DEVELOPMENT.md");
const AGENTS: &str = include_str!("../../../AGENTS.md");

#[test]
fn t28_development_docs_encode_required_workflow() {
    for required in [
        "rtk cargo fmt --check",
        "rtk cargo test --workspace",
        "rtk cargo clippy --workspace --all-targets -- -D warnings",
        "SPEC.md",
        "Do not commit unless the user explicitly asks",
    ] {
        assert!(DEVELOPMENT.contains(required));
    }
}

#[test]
fn t28_agents_doc_keeps_rtk_and_caveman_requirements() {
    assert!(AGENTS.contains("@/Users/sean/.codex/RTK.md"));
    assert!(AGENTS.contains("Use Caveman style"));
    assert!(AGENTS.contains("Wrap normal shell commands with `rtk`"));
}
