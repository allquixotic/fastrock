// Package workspace contains presentation-independent document and queue state.
package workspace

import (
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

type Block struct{ ID, Kind, Role, Text, Status string }
type Draft struct {
	ID, Text    string
	Attachments []string
}
type Conversation struct {
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
	c.Queue = c.Queue[1:]
	return d, true
}
func (c *Conversation) Append(id, kind, role, delta string) {
	for i := len(c.Blocks) - 1; i >= 0; i-- {
		if c.Blocks[i].ID == id {
			c.Blocks[i].Text += delta
			return
		}
	}
	c.Blocks = append(c.Blocks, Block{ID: id, Kind: kind, Role: role, Text: delta})
	if len(c.Blocks) > 3000 {
		c.Blocks = c.Blocks[len(c.Blocks)-3000:]
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
	id := fmt.Sprintf("tab-%d", s.Counter)
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
				s.Tabs[i], s.Tabs[j] = s.Tabs[j], s.Tabs[i]
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
