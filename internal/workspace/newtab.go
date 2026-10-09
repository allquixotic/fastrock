package workspace

import "slices"

// OpenNew reuses the existing start page. A newly created start page belongs
// beside the document from which it was requested.
func (s *State) OpenNew() string {
	for _, t := range s.Tabs {
		if t.Kind == New {
			s.Active = t.ID
			return t.ID
		}
	}
	at := len(s.Tabs)
	for i, t := range s.Tabs {
		if t.ID == s.Active {
			at = i + 1
			break
		}
	}
	id := s.Open(New, "New tab", "", "")
	tab := s.Tabs[len(s.Tabs)-1]
	s.Tabs = slices.Delete(s.Tabs, len(s.Tabs)-1, len(s.Tabs))
	s.Tabs = slices.Insert(s.Tabs, at, tab)
	return id
}

// CompleteNew replaces only the start page that initiated the request. If the
// user has moved elsewhere while the request ran, keep their current focus.
func (s *State) CompleteNew(origin, title, thread string) string {
	for i, t := range s.Tabs {
		if t.ID == origin && t.Kind == New {
			s.Tabs[i] = Tab{ID: t.ID, Kind: Chat, Title: title, Target: thread}
			s.TabRevision++
			return t.ID
		}
	}
	return s.Open(Chat, title, thread, "")
}
