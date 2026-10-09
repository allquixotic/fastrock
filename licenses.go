package fastrock

import (
	"embed"
	"io/fs"
	"strings"
)

// Keep distribution notices inside the signed executable: older Windows
// updaters require a ZIP containing only fastrock.exe.
//
//go:embed LICENSE THIRD_PARTY_NOTICES.md third_party/FLTK.LICENSE third_party/go-fltk.LICENSE internal/desktop/LICENSE
var licenseFiles embed.FS

// Licenses exposes complete bundled notices as a read-only filesystem.
func Licenses() fs.FS { return licenseFiles }

// LicenseNotices returns the complete bundled license and attribution texts.
func LicenseNotices() string {
	var out strings.Builder
	for _, path := range []string{"THIRD_PARTY_NOTICES.md", "LICENSE", "third_party/FLTK.LICENSE", "third_party/go-fltk.LICENSE", "internal/desktop/LICENSE"} {
		data, err := licenseFiles.ReadFile(path)
		if err != nil {
			panic(err) // All paths are required by go:embed at compile time.
		}
		out.WriteString("===== " + path + " =====\n")
		out.Write(data)
		out.WriteString("\n\n")
	}
	return out.String()
}
