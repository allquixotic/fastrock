# Fastrock 0.4.3

Fixes a crash in Slint's accessibility integration when keyboard focus moves to a
newly created control while Windows UI Automation or assistive technology is
active. This defect remains present in 0.4.2. Structural changes now retain a
deferred accessibility-tree rebuild, and focus updates reference only published
controls until that rebuild completes. Accessibility remains enabled.

Includes the workspace performance improvements from 0.4.2. Goose and virtual
workspace storage are unchanged. Windows x64 ZIP only; built, tested, signed,
timestamped and published by GitHub Actions. macOS compilation only.

Use **Quit** in the previous version's tray menu before launching 0.4.3.
