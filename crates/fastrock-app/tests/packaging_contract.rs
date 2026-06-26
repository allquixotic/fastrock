const MACOS_INFO_PLIST: &str = include_str!("../../../packaging/macos/Info.plist");
const WINDOWS_WXS: &str = include_str!("../../../packaging/windows/fastrock.wxs");
const UBUNTU_DESKTOP: &str = include_str!("../../../packaging/ubuntu/fastrock.desktop");
const PACKAGING_README: &str = include_str!("../../../packaging/README.md");

#[test]
fn t27_packaging_templates_reference_fastrock_app_binary() {
    assert!(MACOS_INFO_PLIST.contains("<string>fastrock-app</string>"));
    assert!(WINDOWS_WXS.contains("fastrock-app.exe"));
    assert!(UBUNTU_DESKTOP.contains("Exec=fastrock-app"));
    assert!(PACKAGING_README.contains("rtk cargo build --release -p fastrock-app"));
}

#[test]
fn t27_packaging_templates_do_not_reference_fastrock_cloud_or_telemetry() {
    for template in [
        MACOS_INFO_PLIST,
        WINDOWS_WXS,
        UBUNTU_DESKTOP,
        PACKAGING_README,
    ] {
        let lower = template.to_ascii_lowercase();
        assert!(!lower.contains("https://fastrock"));
        assert!(!lower.contains("fastrock.cloud"));
    }
}
