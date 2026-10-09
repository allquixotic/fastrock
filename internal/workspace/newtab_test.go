package workspace

import "testing"

func TestV48NewTabPlacementAndCompletion(t *testing.T) {
	s := NewState()
	left := s.Open(File, "left", "left", "")
	right := s.Open(File, "right", "right", "")
	s.Active = left
	start := s.OpenNew()
	if len(s.Tabs) != 3 || s.Tabs[1].ID != start || s.Tabs[2].ID != right {
		t.Fatal("start page was not inserted beside the active document", s.Tabs)
	}
	s.Active = right
	if s.OpenNew() != start || len(s.Tabs) != 3 || s.Active != start {
		t.Fatal("start page was duplicated")
	}
	s.Active = right
	if id := s.CompleteNew(start, "New conversation", "thread"); id != start || s.Tabs[1].Kind != Chat || s.Tabs[1].Target != "thread" || s.Active != right {
		t.Fatal("completion lost placement, identity, or newer focus", s.Tabs, s.Active)
	}
	s.Close(right)
	s.Close(start)
	if s.Active != left {
		t.Fatal("closing last/right tab lost the remaining document")
	}
	start = s.OpenNew()
	s.Close(start)
	id := s.CompleteNew(start, "completed after close", "late")
	if s.Current().ID != id || s.Current().Target != "late" || s.Current().Kind != Chat {
		t.Fatal("late completed conversation lost after start page close")
	}
}
