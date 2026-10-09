package update

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/allquixotic/fastrock"
)

// Feed the actual packaged ZIP through the downloader/extractor/stager. Native
// publisher verification runs separately in the Windows packaging gate.
func TestV78PackagedWindowsUpdate(t *testing.T) {
	archive := os.Getenv("FASTROCK_WINDOWS_PACKAGE_ARCHIVE")
	version := os.Getenv("FASTROCK_PACKAGE_VERSION")
	if archive == "" || version == "" {
		t.Skip("set FASTROCK_WINDOWS_PACKAGE_ARCHIVE and FASTROCK_PACKAGE_VERSION")
	}
	data, err := os.ReadFile(archive)
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(data)
	var server *httptest.Server
	server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/release" {
			_ = json.NewEncoder(w).Encode(Release{Tag: version, Assets: []Asset{{Name: "fastrock-windows-amd64.zip", URL: server.URL + "/binary", Size: int64(len(data)), Digest: "sha256:" + hex.EncodeToString(sum[:])}}})
			return
		}
		_, _ = w.Write(data)
	}))
	defer server.Close()
	target := filepath.Join(t.TempDir(), "fastrock.exe")
	old := []byte("existing installation")
	if err := os.WriteFile(target, old, 0600); err != nil {
		t.Fatal(err)
	}
	m := New("0.0.0", t.TempDir(), target, nil)
	m.endpoint, m.client, m.goos, m.arch = server.URL+"/release", server.Client(), "windows", "amd64"
	m.verify = func(path, version string) error {
		if err := verifyBuildVersion(path, "windows", version); err != nil {
			return err
		}
		executable, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		for _, source := range []string{"LICENSE", "THIRD_PARTY_NOTICES.md", "third_party/FLTK.LICENSE", "third_party/go-fltk.LICENSE", "internal/desktop/LICENSE"} {
			notice, err := fs.ReadFile(fastrock.Licenses(), source)
			if err != nil {
				return err
			}
			// Git checkout line endings can differ between the build runner and
			// an independent verifier. Require the complete text in either form.
			notice = bytes.ReplaceAll(notice, []byte("\r\n"), []byte("\n"))
			if !bytes.Contains(executable, notice) && !bytes.Contains(executable, bytes.ReplaceAll(notice, []byte("\n"), []byte("\r\n"))) {
				t.Errorf("signed executable is missing complete notice %s", source)
			}
		}
		if verifyBuildVersion(path, "windows", "v99999.0.0") == nil {
			t.Error("accepted a relabeled release")
		}
		return nil
	}
	defer m.Close()
	if err := m.check(context.Background()); err != nil {
		t.Fatal(err)
	}
	if m.Status().State != "ready" || m.Pending() == "" {
		t.Fatalf("packaged update was not staged: %+v", m.Status())
	}
	installed, err := os.ReadFile(target)
	if err != nil || !bytes.Equal(installed, old) {
		t.Fatalf("changed installation before shutdown: %v", err)
	}
}

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
	if version := os.Getenv("FASTROCK_PACKAGE_VERSION"); version != "" {
		if e := verifyRelease(app, version); e != nil {
			t.Fatal(e)
		}
		if e := verifyRelease(app, "v99999.0.0"); e == nil {
			t.Fatal("accepted relabeled signed binary")
		}
	}
	exe := filepath.Join(app, "Contents", "MacOS", "fastrock")
	file, e := os.OpenFile(exe, os.O_WRONLY, 0)
	if e != nil {
		t.Fatal(e)
	}
	_, e = file.WriteAt([]byte("tampered"), 4096)
	file.Close()
	if e != nil {
		t.Fatal(e)
	}
	if e := verifyPlatform(app); e == nil {
		t.Fatal("accepted tampered signed binary")
	}
}
