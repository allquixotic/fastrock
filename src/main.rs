#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--vwork-hook")) {
        return fastrock_workspaces::embedded::run_hook(
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        );
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--fastrock-provider-credential"))
    {
        let id = std::env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("Credential identity required"))?;
        anyhow::ensure!(
            std::env::args_os().count() == 3,
            "Unexpected credential helper arguments"
        );
        return codex_gui::credential_helper(&id);
    }
    codex_gui::prepare_process_env();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 * 1024 * 1024)
        .build()?;
    let result = codex_gui::run_main(Default::default(), runtime.handle().clone());
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    result
}
