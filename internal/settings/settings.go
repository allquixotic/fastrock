// Package settings owns Fastrock preferences. Codex owns inference configuration.
package settings

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/zalando/go-keyring"
)

const Service = "fastrock"

type SavedView struct {
	Name  string `json:"name"`
	Page  string `json:"page"`
	Query string `json:"query,omitempty"`
	Group string `json:"group,omitempty"`
	Mode  string `json:"mode,omitempty"`
}

type KeyBinding struct {
	Code int    `json:"code"`
	Mods uint32 `json:"modifiers"`
}
type Preferences struct {
	RecentFolders    []string              `json:"recentFolders,omitempty"`
	Keymap           map[string]KeyBinding `json:"keymap,omitempty"`
	Theme            string                `json:"theme"`
	FontSize         int                   `json:"fontSize"`
	Sidebar          bool                  `json:"sidebar"`
	Info             bool                  `json:"info"`
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
	cwd, _ := os.Getwd()
	return Preferences{Theme: "dark", FontSize: 13, Sidebar: true, Info: true, EnterSends: true, BusyInput: "queue", AgentMessages: true,
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
	if e = json.Unmarshal(b, &p); e != nil {
		return p, fmt.Errorf("settings.json: %w", e)
	}
	if p.Theme != "dark" && p.Theme != "light" {
		p.Theme = "dark"
	}
	if p.FontSize < 10 || p.FontSize > 24 {
		p.FontSize = 13
	}
	return p, nil
}
func (s *Store) Save(p Preferences) error { return s.WriteJSON("settings.json", p) }
func (s *Store) WriteJSON(name string, v any) error {
	b, e := json.MarshalIndent(v, "", "  ")
	if e != nil {
		return e
	}
	f, e := os.CreateTemp(s.Dir, ".fastrock-*")
	if e != nil {
		return e
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if e = f.Chmod(0600); e == nil {
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
	return os.Rename(tmp, filepath.Join(s.Dir, name))
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
