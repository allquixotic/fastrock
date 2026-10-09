package update

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func archive(t *testing.T, entries map[string]string) []byte {
	t.Helper()
	var b bytes.Buffer
	z := zip.NewWriter(&b)
	for name, data := range entries {
		w, e := z.Create(name)
		if e != nil {
			t.Fatal(e)
		}
		w.Write([]byte(data))
	}
	if e := z.Close(); e != nil {
		t.Fatal(e)
	}
	return b.Bytes()
}
func TestV15VerifiedDownloadAndCoalescing(t *testing.T) {
	data := archive(t, map[string]string{"fastrock.exe": "MZ\x00\x00fixture"})
	sum := sha256.Sum256(data)
	calls := 0
	var server *httptest.Server
	server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/release" {
			calls++
			json.NewEncoder(w).Encode(Release{Tag: "v1.2.0", Assets: []Asset{{Name: "fastrock-windows-amd64.zip", URL: server.URL + "/binary", Size: int64(len(data)), Digest: "sha256:" + hex.EncodeToString(sum[:])}}})
			return
		}
		w.Write(data)
	}))
	defer server.Close()
	m := New("1.1.0", t.TempDir(), filepath.Join(t.TempDir(), "fastrock.exe"), nil)
	m.endpoint = server.URL + "/release"
	m.client = server.Client()
	m.verify = func(string, string) error { return nil }
	m.goos = "windows"
	m.arch = "amd64"
	if e := m.check(context.Background()); e != nil {
		t.Fatal(e)
	}
	if m.Status().State != "ready" || m.Pending() == "" {
		t.Fatal(m.Status())
	}
	m.Check(context.Background()) // ready means no second network request
	if calls != 1 {
		t.Fatal("duplicate request")
	}
	var j Job
	b, e := os.ReadFile(m.Pending())
	if e != nil {
		t.Fatal(e)
	}
	json.Unmarshal(b, &j)
	if _, e = os.Stat(j.Staged); e != nil {
		t.Fatal(e)
	}
}
func TestV15BadDownloadsDoNotStage(t *testing.T) {
	for _, mode := range []string{"hash", "truncated", "missing-platform", "oversized"} {
		t.Run(mode, func(t *testing.T) {
			data := archive(t, map[string]string{"fastrock.exe": "MZ\x00\x00fixture"})
			sum := sha256.Sum256(data)
			var server *httptest.Server
			server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path == "/release" {
					a := Asset{Name: "fastrock-windows-amd64.zip", URL: server.URL + "/binary", Size: int64(len(data)), Digest: "sha256:" + hex.EncodeToString(sum[:])}
					switch mode {
					case "hash":
						a.Digest = "sha256:" + strings.Repeat("0", 64)
					case "truncated":
						a.Size++
					case "missing-platform":
						a.Name = "other.zip"
					case "oversized":
						a.Size = maxArchive + 1
					}
					json.NewEncoder(w).Encode(Release{Tag: "v2.0.0", Assets: []Asset{a}})
					return
				}
				w.Write(data)
			}))
			defer server.Close()
			cache := t.TempDir()
			target := filepath.Join(t.TempDir(), "fastrock.exe")
			os.WriteFile(target, []byte("old"), 0600)
			m := New("1.0.0", cache, target, nil)
			m.endpoint = server.URL + "/release"
			m.client = server.Client()
			m.verify = func(string, string) error { return nil }
			m.goos = "windows"
			m.arch = "amd64"
			if e := m.check(context.Background()); e == nil {
				t.Fatal("accepted bad release")
			}
			b, _ := os.ReadFile(target)
			if string(b) != "old" || m.Pending() != "" {
				t.Fatal("modified old executable")
			}
			entries, _ := os.ReadDir(cache)
			if len(entries) > 0 {
				t.Fatal("left partial stage")
			}
		})
	}
}

func TestNativeSignatureRequiredBeforeReady(t *testing.T) {
	data := archive(t, map[string]string{"fastrock.exe": "MZ\x00\x00untrusted"})
	sum := sha256.Sum256(data)
	var server *httptest.Server
	server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/release" {
			json.NewEncoder(w).Encode(Release{Tag: "v2.0.0", Assets: []Asset{{Name: "fastrock-windows-amd64.zip", URL: server.URL + "/binary", Size: int64(len(data)), Digest: "sha256:" + hex.EncodeToString(sum[:])}}})
			return
		}
		w.Write(data)
	}))
	defer server.Close()
	cache := t.TempDir()
	m := New("1.0.0", cache, filepath.Join(t.TempDir(), "fastrock.exe"), nil)
	m.client, m.endpoint, m.goos, m.arch = server.Client(), server.URL+"/release", "windows", "amd64"
	checked := false
	m.verify = func(path, version string) error {
		checked = true
		if _, err := os.Stat(path); err != nil {
			t.Fatal(err)
		}
		return errors.New("untrusted publisher")
	}
	if err := m.check(context.Background()); err == nil || !strings.Contains(err.Error(), "untrusted publisher") {
		t.Fatal(err)
	}
	if !checked || m.Pending() != "" || m.Status().State == "ready" {
		t.Fatal("untrusted payload became ready")
	}
	entries, _ := os.ReadDir(cache)
	if len(entries) != 0 {
		t.Fatal("untrusted payload was retained")
	}
}
func TestV15ArchiveContainment(t *testing.T) {
	for _, name := range []string{"../escape", "/absolute", "C:/escape", "fastrock.exe/../../escape", "fastrock.exe\\escape"} {
		t.Run(name, func(t *testing.T) {
			p := filepath.Join(t.TempDir(), "bad.zip")
			os.WriteFile(p, archive(t, map[string]string{name: "bad"}), 0600)
			if e := extract(p, t.TempDir(), "windows"); e == nil {
				t.Fatalf("accepted %q", name)
			}
		})
	}
}
func TestV15ReplacementRollback(t *testing.T) {
	dir := t.TempDir()
	old := filepath.Join(dir, "old")
	backup := filepath.Join(dir, "backup")
	os.WriteFile(old, []byte("old"), 0600)
	if e := replace(old, filepath.Join(dir, "missing"), backup); e == nil {
		t.Fatal("replacement should fail")
	}
	b, e := os.ReadFile(old)
	if e != nil || string(b) != "old" {
		t.Fatal("old binary not restored")
	}
}
func TestStableVersions(t *testing.T) {
	for _, s := range []string{"dev", "1.2.3-rc1", "1.2", "01.2.3", "1.2.-1"} {
		if IsRelease(s) {
			t.Fatal(s)
		}
	}
	if !newer("v1.10.0", "1.9.9") || newer("1.0.0", "1.0.0") || newer("1.0.0", "dev") {
		t.Fatal("version order")
	}
}
func TestV15CancelledDownload(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	m := New("1.0.0", t.TempDir(), "unused", nil)
	if e := m.check(ctx); e == nil {
		t.Fatal("ignored cancellation")
	}
}

func TestV15ShutdownCancelsNetworkWork(t *testing.T) {
	entered := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { close(entered); <-r.Context().Done() }))
	defer server.Close()
	m := New("1.0.0", t.TempDir(), filepath.Join(t.TempDir(), "fastrock.exe"), nil)
	m.endpoint = server.URL
	m.client = server.Client()
	m.Check(context.Background())
	<-entered
	m.Close()
	if m.Pending() != "" {
		t.Fatal("cancelled request staged an update")
	}
}
