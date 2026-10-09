// Package settings owns Fastrock preferences. Codex owns inference configuration.
package settings

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/zalando/go-keyring"
)

const Service = "fastrock"

const preferencesSchemaVersion = 1

// RallyFilter keeps the selected reference separate from its display label.
// Contains filters store text; exact Type filters store a canonical kind.
type RallyFilter struct {
	Field    string `json:"field"`
	Operator string `json:"operator"`
	Value    string `json:"value"`
	Label    string `json:"label,omitempty"`
}

type SavedView struct {
	Display          *BoardDisplay `json:"display,omitempty"`
	Filters          []RallyFilter `json:"filters,omitempty"`
	CardFields       []string      `json:"cardFields"`
	Name             string        `json:"name"`
	Page             string        `json:"page"`
	Query            string        `json:"query,omitempty"`
	Group            string        `json:"group,omitempty"`
	Mode             string        `json:"mode,omitempty"`
	Search           string        `json:"search,omitempty"`
	Timebox          string        `json:"timebox,omitempty"`
	TimeboxName      string        `json:"timeboxName,omitempty"`
	ReleaseTimebox   string        `json:"releaseTimebox,omitempty"`
	ReleaseName      string        `json:"releaseName,omitempty"`
	CurrentIteration bool          `json:"currentIteration,omitempty"`
	Owner            string        `json:"owner,omitempty"`
	State            string        `json:"state,omitempty"`
	Blocked          bool          `json:"blocked,omitempty"`
	Ready            bool          `json:"ready,omitempty"`
	Columns          []string      `json:"columns,omitempty"`
	Sort             string        `json:"sort,omitempty"`
	Descending       bool          `json:"descending,omitempty"`
}

type KeyBinding struct {
	Code int    `json:"code"`
	Mods uint32 `json:"modifiers"`
}
type Preferences struct {
	RallyDisplay     *BoardDisplay         `json:"rallyDisplay,omitempty"`
	SchemaVersion    int                   `json:"schemaVersion,omitempty"`
	RecentFolders    []string              `json:"recentFolders,omitempty"`
	Keymap           map[string]KeyBinding `json:"keymap,omitempty"`
	Theme            string                `json:"theme"`
	FontSize         int                   `json:"fontSize"`
	StatusBar        bool                  `json:"statusBar"`
	Sidebar          bool                  `json:"sidebar"`
	Info             bool                  `json:"info"`
	RallyNavHidden   bool                  `json:"rallyNavHidden"`
	RallyHiddenRows  []string              `json:"rallyHiddenRows"`
	EnterSends       bool                  `json:"enterSends"`
	BusyInput        string                `json:"busyInput,omitempty"`
	AgentMessages    bool                  `json:"agentMessages"`
	RallyEndpoint    string                `json:"rallyEndpoint"`
	RallyWorkspace   string                `json:"rallyWorkspace,omitempty"`
	RallyProject     string                `json:"rallyProject,omitempty"`
	ProjectParents   bool                  `json:"projectParents"`
	ProjectChildren  bool                  `json:"projectChildren"`
	WorkingDirectory string                `json:"workingDirectory,omitempty"`
	Views            []SavedView           `json:"views,omitempty"`
}

func Defaults() Preferences {
	cwd, _ := os.UserHomeDir()
	return Preferences{SchemaVersion: preferencesSchemaVersion, Theme: "dark", FontSize: 13, Sidebar: true, Info: true, EnterSends: true, BusyInput: "queue", AgentMessages: true,
		RallyEndpoint: "https://rally1.rallydev.com", ProjectChildren: true, WorkingDirectory: cwd}
}

type Vault interface {
	Get(string, string) (string, error)
	Set(string, string, string) error
	Delete(string, string) error
}
type SystemVault struct{}

func (SystemVault) Get(s, u string) (string, error) { return keyring.Get(s, u) }
func (SystemVault) Set(s, u, p string) error        { return keyring.Set(s, u, p) }
func (SystemVault) Delete(s, u string) error        { return keyring.Delete(s, u) }

type Store struct {
	Dir   string
	Vault Vault
}

func Open() (*Store, error) {
	dir := os.Getenv("FASTROCK_HOME")
	if dir == "" {
		base, e := os.UserConfigDir()
		if e != nil {
			return nil, e
		}
		dir = filepath.Join(base, "fastrock")
	}
	if e := os.MkdirAll(dir, 0700); e != nil {
		return nil, e
	}
	return &Store{Dir: dir, Vault: SystemVault{}}, nil
}
func (s *Store) Load() (Preferences, error) {
	p := Defaults()
	b, e := os.ReadFile(filepath.Join(s.Dir, "settings.json"))
	if errors.Is(e, os.ErrNotExist) {
		return p, nil
	}
	if e != nil {
		return p, e
	}
	// Start at the unversioned schema while retaining defaults for fields an
	// older app did not write. Explicit false/empty values still unmarshal.
	p.SchemaVersion = 0
	b = bytes.TrimSpace(b)
	if len(b) == 0 || b[0] != '{' {
		return p, fmt.Errorf("settings.json: expected a preferences object")
	}
	if e = json.Unmarshal(b, &p); e != nil {
		return p, fmt.Errorf("settings.json: %w", e)
	}
	return Normalize(p)
}

// Normalize is shared by disk reads and broker updates. Newer schemas must not
// be overwritten by an older application that cannot understand their fields.
func Normalize(p Preferences) (Preferences, error) {
	if p.SchemaVersion > preferencesSchemaVersion || p.SchemaVersion < 0 {
		return p, fmt.Errorf("unsupported preferences schema %d", p.SchemaVersion)
	}
	for p.SchemaVersion < preferencesSchemaVersion {
		switch p.SchemaVersion {
		case 0:
			migratePreferencesV0(&p)
		}
	}
	if p.Theme != "dark" && p.Theme != "light" && p.Theme != "system" {
		p.Theme = "dark"
	}
	if p.FontSize < 10 || p.FontSize > 24 {
		p.FontSize = 13
	}
	if p.RallyDisplay != nil {
		p.RallyDisplay = p.RallyDisplay.Copy()
	}
	p.Views = append([]SavedView(nil), p.Views...)
	p.RallyHiddenRows = append([]string(nil), p.RallyHiddenRows...)
	for i := range p.Views {
		if p.Views[i].Display != nil {
			p.Views[i].Display = p.Views[i].Display.Copy()
		}
	}
	return p, nil
}

// The original schema used the same field names. Load supplies defaults for
// missing fields before this migration; never reset user-selected false values.
func migratePreferencesV0(p *Preferences) { p.SchemaVersion = 1 }

func (s *Store) Save(p Preferences) error {
	p, err := Normalize(p)
	if err != nil {
		return err
	}
	return s.WriteJSON("settings.json", p)
}
func (s *Store) WriteJSON(name string, v any) error {
	b, e := json.Marshal(v)
	if e != nil {
		return e
	}
	if strings.HasPrefix(name, "session") && len(b) > 32<<20 {
		return fmt.Errorf("session exceeds the 32 MiB recovery limit; export large drafts before closing")
	}
	return WriteFileAtomic(filepath.Join(s.Dir, name), b, 0600)
}

// WriteFileAtomic persists a complete replacement before renaming it. Callers
// choose permissions explicitly; raw configuration preserves the existing mode.
func WriteFileAtomic(path string, b []byte, mode os.FileMode) error {
	dir := filepath.Dir(path)
	f, e := os.CreateTemp(dir, ".fastrock-*")
	if e != nil {
		return e
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if e = f.Chmod(mode); e == nil {
		_, e = f.Write(b)
	}
	if e == nil {
		e = f.Sync()
	}
	closeErr := f.Close()
	if e == nil {
		e = closeErr
	}
	if e != nil {
		return e
	}
	for attempt := 0; attempt < 4; attempt++ {
		e = os.Rename(tmp, path)
		if e == nil {
			return syncDirectory(dir)
		}
		if !retryRename(e) {
			break
		}
		time.Sleep(time.Duration(attempt+1) * 25 * time.Millisecond)
	}
	return e
}
func (s *Store) Token(endpoint string) (string, error) {
	if v := os.Getenv("FASTROCK_RALLY_TOKEN"); v != "" {
		return v, nil
	}
	if s.Vault == nil {
		return "", errors.New("OS credential store unavailable")
	}
	return s.Vault.Get(Service, endpoint)
}
func (s *Store) SetToken(endpoint, token string) error {
	if s.Vault == nil {
		return errors.New("OS credential store unavailable")
	}
	if token == "" {
		e := s.Vault.Delete(Service, endpoint)
		if errors.Is(e, keyring.ErrNotFound) {
			return nil
		}
		return e
	}
	return s.Vault.Set(Service, endpoint, token)
}
