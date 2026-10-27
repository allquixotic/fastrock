Fastrock 0.3.2 improves Windows software-rendered scrolling.

- Rendering and text dependencies use compiler optimization; application builds
  retain fast settings. LTO, including local ThinLTO, is disabled.
- A private framebuffer preserves partial rasterization while restoring complete
  Windows presentation surfaces after RDP exposure or lost pixels.
- Large tables, expanded tool output, command output and diffs use smaller native
  virtualized rows. Copy actions retain complete block content.

On games, the same prose-scrolling fixture improved from 63.9 to 7.4 ms at the
95th-percentile frame time (1440×1000, 100% scale), and from 120.5 to 13.2 ms at
2560×1600, 200% scale. Tables, code, tool output and simultaneous streaming were
also measured. These are development fixture results, not universal FPS claims.

Windows x64 ZIP with Authenticode signing, RFC 3161 timestamp and GitHub provenance.
Mac compatibility compilation is checked; no Mac package is published.
