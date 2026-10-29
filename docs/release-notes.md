Fastrock 0.3.3 removes repeated vwork client launches when opening conversations.

- With [vwork 0.3.3](https://github.com/allquixotic/vwork/releases/tag/v0.3.3) and
  Goose HTTP MCP support, Fastrock reuses the existing database daemon for new
  and reopened conversations. A stopped daemon starts once when needed.
- Both HTTP hops bind 127.0.0.1 only, reject browser Origin and unexpected Host
  headers, and retain bearer authentication on the daemon hop. No proxying or
  redirects to external hosts. Workspace bindings survive Fastrock restarts.
- **Compact entire database** now selects the whole redb independently of any
  workspace, including every repository and workspace. Unsaved text is preserved.

On Windows x64 `games`, installed Goose 1.57.0 plus vwork 0.3.3 produced zero
vwork client launches across nine Fastrock open/reload operations. Three-sample
median open time changed from 113.6 to 95.1 ms, and reload from 92.0 to 65.1 ms.
These isolated local-provider fixtures did not reproduce severe work-machine
latency; they verify removal of the repeated process launches. Standalone Goose
and older/customized vwork configurations retain stdio behavior.

The 0.3.2 scrolling and RDP recovery fixes remain. Application optimization stays
at 0, the rendering/text dependency allowlist at 2, and LTO stays off. Windows x64
ZIP includes Authenticode signing, RFC 3161 timestamp and GitHub provenance.
Mac compatibility compilation is checked; no Mac package is published.
