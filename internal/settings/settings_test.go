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

func TestV24PreferenceMigrationPreservesChoices(t *testing.T) {
	for _, version := range []string{"", `"schemaVersion":0,`, `"schemaVersion":1,`} {
		s := &Store{Dir: t.TempDir()}
		path := filepath.Join(s.Dir, "settings.json")
		data := []byte(`{` + version + `"theme":"light","sidebar":false,"enterSends":false,"views":[{"name":"Mine","page":"teamboard","search":"draft"}]}`)
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
		p, err := s.Load()
		if err != nil || p.SchemaVersion != preferencesSchemaVersion || p.Theme != "light" || p.Sidebar || p.EnterSends || !p.AgentMessages || p.BusyInput != "queue" || len(p.Views) != 1 || p.Views[0].Search != "draft" {
			t.Fatalf("legacy preferences changed: %+v %v", p, err)
		}
		if err := s.Save(p); err != nil {
			t.Fatal(err)
		}
		saved, _ := os.ReadFile(path)
		if !strings.Contains(string(saved), `"schemaVersion":1`) {
			t.Fatal("migration not persisted")
		}
	}
	for _, raw := range []string{"null", "[]", "", `{"schemaVersion":-1}`} {
		s := &Store{Dir: t.TempDir()}
		if err := os.WriteFile(filepath.Join(s.Dir, "settings.json"), []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := s.Load(); err == nil {
			t.Fatalf("invalid preferences accepted: %q", raw)
		}
	}
}
