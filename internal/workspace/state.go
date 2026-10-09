// Package workspace contains presentation-independent document and queue state.
package workspace

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"slices"
	"strings"
	"time"
)

type Kind string

const (
	Chat     Kind = "chat"
	Rally    Kind = "rally"
	Settings Kind = "settings"
	File     Kind = "file"
	New      Kind = "new"
)

type Agent struct{ ID, Name, Status string }

type Block struct{ ID, Kind, Role, Text, Status string }
type Draft struct {
	ID, Text    string
	Attachments []string
}
type streamBuffer struct {
	builder strings.Builder
	text    string
}
type Conversation struct {
	Ephemeral                                           bool
	EphemeralLost                                       bool
	Agents                                              []Agent
	EventSequence                                       uint64
	NoMessages                                          bool
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
}

func (c *Conversation) Busy() bool { return c.Status == "running" || c.Status == "starting" }
func (c *Conversation) Enqueue(text string, attachments []string) string {
	id := fmt.Sprintf("draft-%d", time.Now().UnixNano())
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
func (c *Conversation) Append(id, kind, role, delta string) {
	for i := len(c.Blocks) - 1; i >= 0; i-- {
		if c.Blocks[i].ID == id {
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
			c.Blocks[i].Text = stream.text
			// Full history belongs to app-server. Bound live presentation of pathological
			// command output without discarding drafts or queued messages.
			if len(stream.text) > 2<<20 {
				c.Blocks[i].Text = "[Earlier output omitted; open the saved Codex transcript for full output.]\n" + strings.Clone(stream.text[len(stream.text)-(1<<20):])
				delete(c.streams, id)
			}
			c.TrimTranscript()
			return
		}
	}
	c.Blocks = append(c.Blocks, Block{ID: id, Kind: kind, Role: role, Text: delta})
	if len(c.Blocks) > 3000 {
		clear(c.Blocks[:len(c.Blocks)-3000])
		c.Blocks = slices.Clone(c.Blocks[len(c.Blocks)-3000:])
		clear(c.streams)
	}
	c.TrimTranscript()
}
func (c *Conversation) ReleaseTranscript() { c.Blocks = nil; clear(c.streams) }
func (c *Conversation) TrimTranscript() {
	size := 0
	keep := len(c.Blocks)
	for i := len(c.Blocks) - 1; i >= 0; i-- {
		size += len(c.Blocks[i].Text)
		if size > 16<<20 {
			break
		}
		keep = i
	}
	if keep > 0 && keep < len(c.Blocks) {
		for _, b := range c.Blocks[:keep] {
			delete(c.streams, b.ID)
		}
		clear(c.Blocks[:keep])
		c.Blocks = slices.Clone(c.Blocks[keep:])
	}
}

type Tab struct {
	ID                  string
	Kind                Kind
	Title, Target, Page string
}
type State struct {
	Tabs    []Tab
	Active  string
	Chats   map[string]*Conversation
	Counter int
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
	var unique [8]byte
	if _, err := rand.Read(unique[:]); err != nil {
		panic(err)
	}
	id := fmt.Sprintf("tab-%s-%d", hex.EncodeToString(unique[:]), s.Counter)
	s.Tabs = append(s.Tabs, Tab{ID: id, Kind: kind, Title: title, Target: target, Page: page})
	s.Active = id
	return id
}
func (s *State) Close(id string) {
	for i, t := range s.Tabs {
		if t.ID == id {
			s.Tabs = slices.Delete(s.Tabs, i, i+1)
			if s.Active == id {
				s.Active = ""
				if len(s.Tabs) > 0 {
					s.Active = s.Tabs[max(0, i-1)].ID
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
	for _, c := range s.Chats {
		if c.Archived == archived && strings.Contains(strings.ToLower(c.Title+" "+c.Cwd), strings.ToLower(search)) {
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
