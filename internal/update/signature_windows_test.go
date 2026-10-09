package update

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestV72AuthenticodeIgnoresParentPowerShellModules(t *testing.T) {
	// A PowerShell 7 parent can put incompatible modules ahead of Windows
	// PowerShell's own modules. Reproduce that without requiring PS7 installed.
	root := t.TempDir()
	module := filepath.Join(root, "Microsoft.PowerShell.Security")
	if err := os.MkdirAll(module, 0700); err != nil {
		t.Fatal(err)
	}
	manifest := `@{
ModuleVersion = '7.0.0'
GUID = 'b177703c-c37b-4f54-9d6e-0d2c0f94b188'
PowerShellVersion = '7.0'
RootModule = 'missing.dll'
CmdletsToExport = @('Get-AuthenticodeSignature')
}`
	if err := os.WriteFile(filepath.Join(module, "Microsoft.PowerShell.Security.psd1"), []byte(manifest), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PSModulePath", root)
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	_, err = signedPublisher(exe)
	if err == nil || !strings.Contains(err.Error(), "NotSigned") {
		t.Fatalf("verification must load native modules and reject the unsigned test executable: %v", err)
	}
}
