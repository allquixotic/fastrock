package settings

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type vault map[string]string

func (v vault) Get(s, u string) (string, error) { return v[s+u], nil }
func (v vault) Set(s, u, p string) error        { v[s+u] = p; return nil }
func (v vault) Delete(s, u string) error        { delete(v, s+u); return nil }
func TestDarkDefaultAndKeyringOnlySecrets(t *testing.T) {
	s := &Store{Dir: t.TempDir(), Vault: vault{}}
	p, e := s.Load()
	if e != nil || p.Theme != "dark" {
		t.Fatal(p, e)
	}
	if e = s.SetToken("https://rally.test", "super-secret"); e != nil {
		t.Fatal(e)
	}
	p.Views = []SavedView{{Name: "Mine", Page: "teamboard", Query: "(Blocked = true)"}}
	if e = s.Save(p); e != nil {
		t.Fatal(e)
	}
	b, e := os.ReadFile(filepath.Join(s.Dir, "settings.json"))
	if e != nil || strings.Contains(string(b), "super-secret") {
		t.Fatal("token leaked")
	}
	loaded, e := s.Load()
	if e != nil || len(loaded.Views) != 1 {
		t.Fatal(loaded, e)
	}
	token, _ := s.Token("https://rally.test")
	if token != "super-secret" {
		t.Fatal("token missing")
	}
}
func TestCorruptPreferencesAreReported(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	_ = os.WriteFile(filepath.Join(s.Dir, "settings.json"), []byte("not json"), 0600)
	if _, e := s.Load(); e == nil {
		t.Fatal("corruption ignored")
	}
	if e := s.SetToken("x", "secret"); e == nil {
		t.Fatal("silent insecure fallback")
	}
}
func TestAtomicSettingsWrites(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	for range 3 {
		if e := s.Save(Defaults()); e != nil {
			t.Fatal(e)
		}
	}
	files, _ := filepath.Glob(filepath.Join(s.Dir, ".fastrock-*"))
	if len(files) > 0 {
		t.Fatal("temporary files leaked")
	}
	_, e := os.Stat(filepath.Join(s.Dir, "settings.json"))
	if errors.Is(e, os.ErrNotExist) {
		t.Fatal(e)
	}
}
