package update

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// This validates a release archive without ever launching its executable or GUI.
func TestV15PackagedMacBundle(t *testing.T) {
	archive := os.Getenv("FASTROCK_PACKAGE_ARCHIVE")
	if archive == "" || runtime.GOOS != "darwin" {
		t.Skip("set FASTROCK_PACKAGE_ARCHIVE on macOS to verify a packaged bundle")
	}
	dir := t.TempDir()
	if e := extract(archive, dir, "darwin"); e != nil {
		t.Fatal(e)
	}
	app := filepath.Join(dir, "Fastrock.app")
	if e := validatePayload(app, "darwin"); e != nil {
		t.Fatal(e)
	}
	if e := verifyPlatform(app); e != nil {
		t.Fatal(e)
	}
}
