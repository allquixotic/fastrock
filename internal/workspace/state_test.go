package workspace

import "testing"

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
