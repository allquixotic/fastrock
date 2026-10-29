Fastrock 0.3.4 makes conversation text easier to select and copy.

- Start a drag in the padding beside a message or code block, without aiming
  directly at its text. The text layout stays unchanged.
- Select tool results, including literal markup and Unicode, and copy through
  Ctrl/Cmd+C or the native context menu. Selection spans visible virtual rows.
- Expand a tool call to see its full arguments, or a command to see its complete
  displayed command, even when there is no output. Copy All includes the call
  details and complete retained output.

This update changes Fastrock only; no vwork upgrade or configuration change is
required. Shared localhost MCP, scrolling virtualization and RDP recovery remain.
Application optimization stays at 0, the rendering/text dependency allowlist at
2, and LTO stays off. The Windows x64 ZIP is signed and timestamped through CI
with GitHub provenance. Mac compatibility is compiled; no Mac package is published.
