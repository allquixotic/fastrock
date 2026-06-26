# Fastrock Packaging

Packaging targets:

- macOS: app bundle metadata from `macos/Info.plist`.
- Windows: WiX installer template from `windows/fastrock.wxs`.
- Ubuntu: desktop entry from `ubuntu/fastrock.desktop`.

All packages install the `fastrock-app` binary as the Fastrock desktop app.
Packagers must not add telemetry, login, or hosted Fastrock service endpoints.

Expected release build command:

```bash
rtk cargo build --release -p fastrock-app
```
