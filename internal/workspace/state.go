// Package workspace contains presentation-independent document and queue state.
package workspace

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"slices"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"
)

type Kind string

const (
	Chat     Kind = "chat"
	Rally    Kind = "rally"
	Settings Kind = "settings"
	File     Kind = "file"
	New      Kind = "new"
)

type Agent struct{ ID, Name, Status, Role, Path, Message string }

// ThreadSettings is the effective server configuration returned on start,
// resume, and fork. Keep it distinct from editable application defaults.
type ThreadSettings struct {
	Current                               bool `json:"-"`
	Provider, Approval, Reviewer, Sandbox string
	WritableRoots                         string
	Network                               bool
}

type Block struct{ ID, Kind, Role, Text, Status string }
type Draft struct {
	ID, Text      string
	Status, Error string
	Attachments   []string
}
type streamBuffer struct {
	builder strings.Builder
	text    string
}
type Conversation struct {
	SidebarHidden                                       bool   `json:"-"`
	SidebarRevision                                     uint64 `json:"-"`
	Unread                                              bool
	QueuePaused                                         bool
	EditQueue                                           string
	QueueDraft                                          Draft
	Outbox                                              []Draft
	transcriptBytes                                     int
	transcriptCount                                     int
	Ephemeral                                           bool
	EphemeralLost                                       bool
	Agents                                              []Agent
	EventSequence                                       uint64
	NoMessages                                          bool
	Settings                                            ThreadSettings
	SideParentID, SideParentTitle                       string
	streams                                             map[string]*streamBuffer
	ID, Title, Cwd, Model, Effort, Tier, TurnID, Status string
	Plan                                                bool
	Blocks                                              []Block
	Draft                                               string
	DraftAttachments                                    []string
	Queue                                               []Draft
	Updated                                             int64
	Archived                                            bool
	Tokens                                              int
	ContextTokens, ContextWindow                        int
	InputTokens, CachedTokens, OutputTokens             int
}

func (c *Conversation) Busy() bool { return c.Status == "running" || c.Status == "starting" }
func (c *Conversation) Enqueue(text string, attachments []string) string {
	id := NewID("draft")
	c.Queue = append(c.Queue, Draft{ID: id, Text: text, Attachments: slices.Clone(attachments)})
	return id
}
func (c *Conversation) EditQueued(id, text string) bool {
	for i := range c.Queue {
		if c.Queue[i].ID == id {
			c.Queue[i].Text = text
			return true
		}
	}
	return false
}
func (c *Conversation) DeleteQueued(id string) bool {
	for i, d := range c.Queue {
		if d.ID == id {
			c.Queue = slices.Delete(c.Queue, i, i+1)
			return true
		}
	}
	return false
}
func (c *Conversation) Pop() (Draft, bool) {
	if len(c.Queue) == 0 {
		return Draft{}, false
	}
	d := c.Queue[0]
	c.Queue[0] = Draft{}
	c.Queue = c.Queue[1:]
	return d, true
}

const MaxBlockBytes = 2 << 20
const MaxTranscriptBytes = 16 << 20
const MaxTranscriptBlocks = 3000

var uniqueSequence atomic.Uint64

func NewID(prefix string) string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err == nil {
		return prefix + "-" + hex.EncodeToString(b[:])
	}
	return fmt.Sprintf("%s-%d-%d", prefix, time.Now().UnixNano(), uniqueSequence.Add(1))
}
func boundedOutput(value string) string {
	if len(value) <= MaxBlockBytes {
		return value
	}
	start := len(value) - (1 << 20)
	for start < len(value) && !utf8.RuneStart(value[start]) {
		start++
	}
	return "[Earlier output omitted; read the full history from Codex.]\n" + strings.Clone(value[start:])
}
func (c *Conversation) Append(id, kind, role, delta string) {
	c.ensureTranscriptAccounting()
	for i := len(c.Blocks) - 1; i >= 0; i-- {
		if c.Blocks[i].ID != id {
			continue
		}
		if c.streams == nil {
			c.streams = map[string]*streamBuffer{}
		}
		stream := c.streams[id]
		if stream == nil || stream.text != c.Blocks[i].Text {
			stream = &streamBuffer{}
			stream.builder.WriteString(c.Blocks[i].Text)
			c.streams[id] = stream
		}
		stream.builder.WriteString(delta)
		stream.text = stream.builder.String()
		next := boundedOutput(stream.text)
		c.transcriptBytes += len(next) - len(c.Blocks[i].Text)
		c.Blocks[i].Text = next
		if len(stream.text) > MaxBlockBytes {
			delete(c.streams, id)
		}
		c.trimAccounted()
		return
	}
	delta = boundedOutput(delta)
	c.Blocks = append(c.Blocks, Block{ID: id, Kind: kind, Role: role, Text: delta})
	c.transcriptBytes += len(delta)
	c.transcriptCount = len(c.Blocks)
	c.trimAccounted()
}
func (c *Conversation) ReplaceBlock(id, body, status string) bool {
	c.ensureTranscriptAccounting()
	for i := range c.Blocks {
		if c.Blocks[i].ID == id {
			if body != "" {
				body = boundedOutput(body)
				c.transcriptBytes += len(body) - len(c.Blocks[i].Text)
				c.Blocks[i].Text = body
			}
			c.Blocks[i].Status = status
			delete(c.streams, id)
			c.trimAccounted()
			return true
		}
	}
	return false
}
func (c *Conversation) FinishBlock(id string) { delete(c.streams, id) }
func (c *Conversation) ReleaseTranscript() {
	c.Blocks = nil
	c.streams = nil
	c.transcriptBytes = 0
	c.transcriptCount = 0
}
func (c *Conversation) ensureTranscriptAccounting() {
	if c.transcriptCount == len(c.Blocks) {
		return
	}
	c.transcriptBytes = 0
	for i := range c.Blocks {
		c.Blocks[i].Text = boundedOutput(c.Blocks[i].Text)
		c.transcriptBytes += len(c.Blocks[i].Text)
	}
	c.transcriptCount = len(c.Blocks)
}
func (c *Conversation) TrimTranscript() {
	c.transcriptCount = -1
	c.ensureTranscriptAccounting()
	c.trimAccounted()
}
func (c *Conversation) trimAccounted() {
	drop := 0
	for drop < len(c.Blocks)-1 && (len(c.Blocks)-drop > MaxTranscriptBlocks || c.transcriptBytes > MaxTranscriptBytes) {
		c.transcriptBytes -= len(c.Blocks[drop].Text)
		delete(c.streams, c.Blocks[drop].ID)
		drop++
	}
	if drop > 0 {
		clear(c.Blocks[:drop])
		c.Blocks = slices.Clone(c.Blocks[drop:])
	}
	c.transcriptCount = len(c.Blocks)
}

type Tab struct {
	ID                  string
	Kind                Kind
	Title, Target, Page string
}
type State struct {
	Tabs        []Tab
	Active      string
	Chats       map[string]*Conversation
	Counter     int
	TabRevision uint64 `json:"-"`
}

func NewState() *State { return &State{Chats: map[string]*Conversation{}} }
func (s *State) Open(kind Kind, title, target, page string) string {
	for _, t := range s.Tabs {
		if t.Kind == kind && t.Target == target && t.Page == page && kind != New {
			s.Active = t.ID
			return t.ID
		}
	}
	s.Counter++
	id := NewID("tab")
	s.Tabs = append(s.Tabs, Tab{ID: id, Kind: kind, Title: title, Target: target, Page: page})
	s.TabRevision++
	s.Active = id
	return id
}
func (s *State) Close(id string) {
	for i, t := range s.Tabs {
		if t.ID == id {
			s.Tabs = slices.Delete(s.Tabs, i, i+1)
			s.TabRevision++
			if s.Active == id {
				s.Active = ""
				if len(s.Tabs) > 0 {
					s.Active = s.Tabs[min(i, len(s.Tabs)-1)].ID
				}
			}
			return
		}
	}
}
func (s *State) Move(id string, offset int) {
	for i, t := range s.Tabs {
		if t.ID == id {
			j := i + offset
			if j >= 0 && j < len(s.Tabs) {
				if i < j {
					copy(s.Tabs[i:j], s.Tabs[i+1:j+1])
				} else {
					copy(s.Tabs[j+1:i+1], s.Tabs[j:i])
				}
				s.Tabs[j] = t
			}
			return
		}
	}
}
func (s *State) Current() *Tab {
	for i := range s.Tabs {
		if s.Tabs[i].ID == s.Active {
			return &s.Tabs[i]
		}
	}
	return nil
}
func (s *State) Sidebar(search string, archived bool) []*Conversation {
	var rows []*Conversation
	search = strings.ToLower(search)
	for _, c := range s.Chats {
		if !c.SidebarHidden && c.Archived == archived && strings.Contains(strings.ToLower(c.Title+" "+c.Cwd), search) {
			rows = append(rows, c)
		}
	}
	slices.SortFunc(rows, func(a, b *Conversation) int {
		if a.Updated > b.Updated {
			return -1
		}
		if a.Updated < b.Updated {
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
	return rows
}
