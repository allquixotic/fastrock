#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() -> anyhow::Result<()> {
    codex_gui::prepare_process_env();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 * 1024 * 1024)
        .build()?;
    let result = codex_gui::run_main(Default::default(), runtime.handle().clone());
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    result
}
