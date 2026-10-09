package ui

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

var errFutureSession = errors.New("session uses a newer schema")

const transferSchemaVersion = 1

func validTab(t workspace.Tab) bool {
	if t.ID == "" {
		return false
	}
	switch t.Kind {
	case workspace.Chat:
		return t.Target != ""
	case workspace.Rally:
		return t.Page != ""
	case workspace.File:
		return t.Target != ""
	case workspace.New, workspace.Settings:
		return true
	}
	return false
}

func validateTransfer(p tabTransfer) error {
	if p.SchemaVersion < 0 || p.SchemaVersion > transferSchemaVersion {
		return fmt.Errorf("unsupported document schema %d", p.SchemaVersion)
	}
	if !validTab(p.Tab) && !(p.Tab.ID != "" && p.Tab.Kind == workspace.File && p.File != nil && p.File.Virtual) {
		return fmt.Errorf("invalid document identity or kind")
	}
	if p.Chat != nil && (p.Tab.Kind != workspace.Chat || p.Chat.ID == "" || p.Chat.ID != p.Tab.Target) {
		return fmt.Errorf("conversation does not match document")
	}
	if p.Rally != nil && p.Tab.Kind != workspace.Rally || p.File != nil && p.Tab.Kind != workspace.File || p.ChatView != nil && p.Tab.Kind != workspace.Chat {
		return fmt.Errorf("document content does not match kind")
	}
	return nil
}

// Decode records independently: one damaged editor must not hide other drafts.
// Callers preserve the original file whenever issues are returned.
func decodeSession(data []byte) (session, []string, error) {
	var wire struct {
		SchemaVersion int
		Mailbox       map[string]json.RawMessage
		Documents     []json.RawMessage
		Tabs          []json.RawMessage
		Active        string
		Counter       int
		Chats         map[string]json.RawMessage
	}
	var s session
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return s, nil, errors.New("empty session")
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		return s, nil, err
	}
	if wire.SchemaVersion > sessionSchemaVersion {
		return s, nil, fmt.Errorf("%w: %d", errFutureSession, wire.SchemaVersion)
	}
	if wire.SchemaVersion < 0 {
		return s, nil, errors.New("invalid session schema")
	}
	s = session{SchemaVersion: wire.SchemaVersion, Active: wire.Active, Counter: max(0, wire.Counter), Chats: map[string]*workspace.Conversation{}, Mailbox: map[string][]mailMessage{}}
	var issues []string
	for id, raw := range wire.Chats {
		var c *workspace.Conversation
		if err := json.Unmarshal(raw, &c); err != nil || c == nil || id == "" || c.ID != id {
			issues = append(issues, "conversation "+id)
			continue
		}
		s.Chats[id] = c
	}
	tabs := map[string]bool{}
	for i, raw := range wire.Tabs {
		var t workspace.Tab
		if err := json.Unmarshal(raw, &t); err != nil || !validTab(t) || tabs[t.ID] {
			issues = append(issues, fmt.Sprintf("tab %d", i+1))
			continue
		}
		if t.Kind == workspace.Chat && s.Chats[t.Target] == nil {
			issues = append(issues, "missing conversation for "+t.ID)
			continue
		}
		tabs[t.ID] = true
		s.Tabs = append(s.Tabs, t)
	}
	docs := map[string]bool{}
	for i, raw := range wire.Documents {
		var p tabTransfer
		if err := json.Unmarshal(raw, &p); err != nil {
			issues = append(issues, fmt.Sprintf("document %d: %s", i+1, err))
			continue
		}
		if err := validateTransfer(p); err != nil || docs[p.Tab.ID] {
			issues = append(issues, fmt.Sprintf("document %d has an invalid identity, schema or content", i+1))
			continue
		}
		docs[p.Tab.ID] = true
		s.Documents = append(s.Documents, p)
		// Old snapshots can contain a document omitted from Tabs. Recover it.
		if !tabs[p.Tab.ID] {
			s.Tabs = append(s.Tabs, p.Tab)
			tabs[p.Tab.ID] = true
		}
	}
	for id, raw := range wire.Mailbox {
		var messages []mailMessage
		if err := json.Unmarshal(raw, &messages); err != nil {
			issues = append(issues, "mailbox "+id)
			continue
		}
		s.Mailbox[id] = messages
	}
	migrateSession(&s)
	if !tabs[s.Active] {
		s.Active = ""
		if len(s.Tabs) > 0 {
			s.Active = s.Tabs[0].ID
		}
	}
	return s, issues, nil
}

func migrateSession(s *session) {
	for s.SchemaVersion < sessionSchemaVersion {
		switch s.SchemaVersion {
		case 0:
			migrateSessionV0(s)
		case 1:
			migrateSessionV1(s)
		case 2:
			migrateSessionV2(s)
		}
	}
}

func migrateSessionV0(s *session) {
	// Legacy queues had no admission state; they must never resume themselves.
	for _, c := range s.Chats {
		c.QueuePaused = true
	}
	s.SchemaVersion = 1
}
func migrateSessionV1(s *session) {
	// richtext.Saved.UnmarshalJSON converts numeric text/per-rune marks to spans.
	s.SchemaVersion = 2
}
func migrateSessionV2(s *session) {
	for i := range s.Documents {
		s.Documents[i].SchemaVersion = transferSchemaVersion
	}
	s.SchemaVersion = 3
}

// Preserve the original bytes before a recovered snapshot can be written.
func preserveSessionFile(path string) (string, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return "", err
	}
	if !info.Mode().IsRegular() {
		return "", fmt.Errorf("session path is not a regular file")
	}
	recovery := path + ".recovery-" + time.Now().UTC().Format("20060102T150405.000000000")
	if err := os.Rename(path, recovery); err != nil {
		return "", err
	}
	return recovery, nil
}

// Read one file without recursively trying backups. The same validation and
// preservation rules apply to the primary snapshot and its backup.
func recoverSessionRecord(path string) (*session, string, bool) {
	data, err := readSessionFile(path)
	if os.IsNotExist(err) {
		return nil, "", false
	}
	var s session
	var issues []string
	if err == nil {
		s, issues, err = decodeSession(data)
	}
	if err == nil && len(issues) == 0 {
		return &s, "", false
	}
	if errors.Is(err, errFutureSession) {
		return nil, fmt.Sprintf("%s uses a newer Fastrock version; original preserved and automatic session writes disabled.", filepath.Base(path)), true
	}
	recovery, preserveErr := preserveSessionFile(path)
	if preserveErr != nil {
		return nil, fmt.Sprintf("Could not preserve %s; automatic session writes disabled: %s", filepath.Base(path), preserveErr), true
	}
	if err == nil {
		return &s, fmt.Sprintf("Recovered session with %d damaged records omitted. Original preserved at %s", len(issues), recovery), false
	}
	return nil, fmt.Sprintf("Could not recover %s: %s. Original preserved at %s", filepath.Base(path), err, recovery), false
}

func recoverSessionFile(path string) (*session, string, bool) {
	s, problem, blocked := recoverSessionRecord(path)
	if s != nil || blocked {
		return s, problem, blocked
	}
	backup, backupProblem, backupBlocked := recoverSessionRecord(path + ".bak")
	if backup != nil {
		if problem == "" {
			problem = fmt.Sprintf("Recovered missing %s from its last-good backup.", filepath.Base(path))
		} else {
			problem = fmt.Sprintf("Recovered %s from its last-good backup. %s", filepath.Base(path), problem)
		}
	}
	if backupProblem != "" {
		problem = strings.TrimSpace(problem + "\n" + backupProblem)
	}
	return backup, problem, backupBlocked
}

func readSessions(dir string) loadedSessions {
	result := loadedSessions{}
	result.primary, result.problem, result.blocked = recoverSessionFile(filepath.Join(dir, "session.json"))
	paths, err := filepath.Glob(filepath.Join(dir, "session-popout-*.json"))
	if err != nil {
		return result
	}
	backups, _ := filepath.Glob(filepath.Join(dir, "session-popout-*.json.bak"))
	for _, backup := range backups {
		paths = append(paths, strings.TrimSuffix(backup, ".bak"))
	}
	slices.Sort(paths)
	paths = slices.Compact(paths)
	for _, path := range paths {
		extra, problem, blocked := recoverSessionFile(path)
		if problem != "" {
			if result.problem != "" {
				result.problem += "\n"
			}
			result.problem += problem
		}
		result.blocked = result.blocked || blocked
		if extra != nil {
			result.extras = append(result.extras, *extra)
			result.paths = append(result.paths, path)
		}
	}
	if result.primary == nil && len(result.extras) > 0 {
		result.primary = &session{SchemaVersion: sessionSchemaVersion}
	}
	return result
}

// Keep a validated previous snapshot until its replacement is fully durable.
// Failure to write the backup must leave the current file untouched.
func writeSessionFile(store *settings.Store, name string, s session) error {
	data, err := json.Marshal(s)
	if err != nil {
		return err
	}
	if len(data) > 32<<20 {
		return fmt.Errorf("session exceeds the 32 MiB recovery limit; export large drafts before closing")
	}
	if _, issues, err := decodeSession(data); err != nil {
		return err
	} else if len(issues) > 0 {
		return fmt.Errorf("refusing invalid session snapshot: %s", issues[0])
	}
	path := filepath.Join(store.Dir, name)
	if backup, readErr := readSessionFile(path + ".bak"); readErr == nil {
		_, issues, decodeErr := decodeSession(backup)
		if errors.Is(decodeErr, errFutureSession) {
			return fmt.Errorf("preserve newer session backup: %w", decodeErr)
		}
		if decodeErr != nil || len(issues) > 0 {
			if _, err := preserveSessionFile(path + ".bak"); err != nil {
				return fmt.Errorf("preserve damaged session backup: %w", err)
			}
		}
	} else if !os.IsNotExist(readErr) {
		return fmt.Errorf("read session backup: %w", readErr)
	}
	if old, err := readSessionFile(path); err == nil {
		if _, issues, err := decodeSession(old); err != nil {
			return fmt.Errorf("existing session needs recovery: %w", err)
		} else if len(issues) > 0 {
			return fmt.Errorf("existing session contains damaged records; preserved without replacement")
		}
		if err = settings.WriteFileAtomic(path+".bak", old, 0600); err != nil {
			return fmt.Errorf("preserve previous session: %w", err)
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	return settings.WriteFileAtomic(path, data, 0600)
}
