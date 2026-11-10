# Fastrock 0.5.0

Updates the direct dependencies, including unmodified Goose 1.58.0 and the latest
stable Codex support crates. Goose continues to run inside Fastrock; no separate
installation or service is required. Existing configuration, conversations,
virtual workspaces and credential identities are preserved.
The default search-summary model uses `us.openai.gpt-6-luna` on Bedrock Runtime.
Goose 1.58 routes bare closed-weight GPT IDs to Mantle; use a `us.` inference
profile for Runtime. Explicitly saved model IDs are retained.

- **Context window:** choose **View context window** in the conversation info
  pane to inspect the next request's token counts, component breakdown and text
  previews. Refresh after a turn or model/tool change. Counts use Goose's local
  tokenizer and may differ from provider accounting; pending compaction is shown.
  Restored conversations still require Resume before context inspection.
- **Thinking:** enable **Settings → Appearance → Collapse thinking by default**
  to start thinking blocks collapsed. This is opt-in and persists across restarts.
  Each block can still be expanded, selected and copied. Thinking text remains in
  conversation exports.

Retains the 0.4.2 workspace performance improvements and 0.4.3 accessibility
repair. Windows x64 ZIP only, built, tested, signed, timestamped and published by
GitHub Actions. macOS compilation only.

Use **Quit** in the previous version's tray menu before launching 0.5.0.
