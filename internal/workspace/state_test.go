package workspace

import (
	"fmt"
	"strings"
	"testing"
	"unicode/utf8"
)

func TestTabsAndSidebarStaySeparate(t *testing.T) {
	s := NewState()
	s.Chats["c"] = &Conversation{ID: "c", Title: "A chat", Updated: 2}
	s.Open(Chat, "A chat", "c", "")
	r := s.Open(Rally, "Team board", "", "teamboard")
	s.Open(Settings, "Settings", "", "")
	if len(s.Sidebar("", false)) != 1 {
		t.Fatal("non-chat in sidebar")
	}
	s.Close(r)
	if len(s.Tabs) != 2 {
		t.Fatal("close failed")
	}
	s.Open(Chat, "A chat", "c", "")
	if len(s.Tabs) != 2 {
		t.Fatal("duplicate chat tab")
	}
}
func TestQueueEditDeleteAndIsolation(t *testing.T) {
	a := &Conversation{}
	b := &Conversation{}
	id := a.Enqueue("first", []string{"image.png"})
	b.Enqueue("other", nil)
	if !a.EditQueued(id, "changed") || !a.DeleteQueued(id) {
		t.Fatal("edit/delete failed")
	}
	a.Enqueue("one", nil)
	a.Enqueue("two", nil)
	d, ok := a.Pop()
	if !ok || d.Text != "one" || b.Queue[0].Text != "other" {
		t.Fatal("queue lost ordering or isolation")
	}
}
func TestIndependentThreadsAndActivityDates(t *testing.T) {
	s := NewState()
	s.Chats["a"] = &Conversation{ID: "a", Updated: 5}
	s.Chats["b"] = &Conversation{ID: "b", Updated: 10}
	if s.Sidebar("", false)[0].ID != "b" {
		t.Fatal("not activity sorted")
	}
	s.Chats["b"].Archived = true
	if len(s.Sidebar("", false)) != 1 {
		t.Fatal("archive ignored")
	}
	s.Chats["a"].Append("m", "agentMessage", "assistant", "hello ")
	s.Chats["a"].Append("m", "agentMessage", "assistant", "world")
	if len(s.Chats["a"].Blocks) != 1 || s.Chats["a"].Blocks[0].Text != "hello world" {
		t.Fatal("delta duplication")
	}
}

func TestTranscriptBoundsEveryInsertAndReplacement(t *testing.T) {
	c := &Conversation{}
	huge := strings.Repeat("🙂", 2<<20)
	c.Append("one", "agentMessage", "assistant", huge)
	if len(c.Blocks[0].Text) > MaxBlockBytes || !utf8.ValidString(c.Blocks[0].Text) {
		t.Fatal("unbounded or invalid UTF-8 initial block")
	}
	c.Append("one", "agentMessage", "assistant", " more")
	c.ReplaceBlock("one", huge, "completed")
	if len(c.streams) != 0 || len(c.Blocks[0].Text) > MaxBlockBytes {
		t.Fatal("completion retained streaming text")
	}
	for i := 0; i < MaxTranscriptBlocks+10; i++ {
		c.Append(fmt.Sprint(i), "message", "assistant", "x")
	}
	if len(c.Blocks) > MaxTranscriptBlocks {
		t.Fatal("block limit exceeded")
	}
	c.ReleaseTranscript()
	if c.transcriptBytes != 0 || len(c.streams) != 0 {
		t.Fatal("retained released transcript")
	}
}
