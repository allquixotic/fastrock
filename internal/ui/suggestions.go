package ui

import (
	"context"
	"strings"
	"time"
	"unicode"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type skillRef struct {
	Name, Path, Description string
	Enabled                 *bool
}

func (s skillRef) enabled() bool { return s.Enabled == nil || *s.Enabled }

type skillList struct{ Data []struct{ Skills []skillRef } }

// Scan only the token before the caret, so completions also work in the middle
// of a draft and don't replace surrounding text.
func completionToken(e *nucular.TextEditor) (string, int, int) {
	end := min(e.Cursor, len(e.Buffer))
	start := end
	for start > 0 && !unicode.IsSpace(e.Buffer[start-1]) {
		start--
	}
	if start == end {
		return "", start, end
	}
	token := e.Buffer[start:end]
	if token[0] != '@' && token[0] != '$' && !(token[0] == '/' && start == 0) {
		return "", start, end
	}
	return string(token), start, end
}
func acceptSuggestion(v *chatView) {
	if v.SuggestIndex < 0 || v.SuggestIndex >= len(v.Suggest) {
		return
	}
	query, start, end := completionToken(v.Editor)
	if query != v.SuggestQuery {
		if v.SuggestCancel != nil {
			v.SuggestCancel()
			v.SuggestCancel = nil
		}
		return
	}
	value := v.Suggest[v.SuggestIndex] + " "
	before, after := string(v.Editor.Buffer[:start]), string(v.Editor.Buffer[end:])
	setText(v.Editor, before+value+after)
	v.Editor.Cursor = start + len([]rune(value))
	v.Editor.SelectStart, v.Editor.SelectEnd = v.Editor.Cursor, v.Editor.Cursor
	v.Suggest = nil
	v.SuggestQuery = ""
}
func (a *App) suggestions(w *nucular.Window, c *workspace.Conversation, v *chatView) {
	query, _, _ := completionToken(v.Editor)
	if !v.Editor.Active {
		query = ""
	}
	if query != v.SuggestQuery {
		if v.SuggestCancel != nil {
			v.SuggestCancel()
			v.SuggestCancel = nil
		}
		v.SuggestGeneration++
		v.SuggestQuery = query
		v.SuggestIndex = 0
		v.Suggest = nil
		if strings.HasPrefix(query, "/") {
			for _, name := range slashCommands {
				if fuzzyCommand(name, query[1:]) {
					v.Suggest = append(v.Suggest, "/"+name)
				}
			}
		}
		if strings.HasPrefix(query, "@") || strings.HasPrefix(query, "$") {
			client, cwd, id := a.client, c.Cwd, c.ID
			generation := v.SuggestGeneration
			if client != nil {
				ctx, cancel := context.WithTimeout(a.ctx, 10*time.Second)
				v.SuggestCancel = cancel
				a.work(func() {
					defer cancel()
					timer := time.NewTimer(200 * time.Millisecond)
					defer timer.Stop()
					select {
					case <-timer.C:
					case <-ctx.Done():
						return
					}
					// One bounded debounce request. A newer query cancels publication, and the
					// server cancellation token also supersedes older file-search work.
					var options []string
					if query[0] == '@' {
						var r struct{ Files []struct{ Path string } }
						if client.Call(ctx, "fuzzyFileSearch", map[string]any{"query": query[1:], "roots": []string{cwd}, "cancellationToken": "fastrock-suggestions-" + id}, &r) == nil {
							for _, f := range r.Files {
								options = append(options, "@"+f.Path)
								if len(options) == 8 {
									break
								}
							}
						}
					} else {
						r, err := a.skillCache.get(ctx, a.ctx, client, cwd)
						if err == nil {
							for _, d := range r.Data {
								for _, s := range d.Skills {
									if s.enabled() && strings.Contains(strings.ToLower(s.Name), strings.ToLower(query[1:])) && len(options) < 8 {
										options = append(options, "$"+s.Name)
									}
								}
							}
						}
					}
					a.post(func() {
						if v.SuggestQuery == query && v.SuggestGeneration == generation {
							v.Suggest = options
						}
					})
				})
			}
		}
	}
	if len(v.Suggest) > 8 {
		v.Suggest = v.Suggest[:8]
	}
	v.SuggestIndex = min(v.SuggestIndex, max(0, len(v.Suggest)-1))
	for i, s := range v.Suggest {
		w.Row(24).Dynamic(1)
		if button(w, s, i == v.SuggestIndex, a.p) {
			v.SuggestIndex = i
			acceptSuggestion(v)
		}
	}
}

func fuzzyCommand(name, query string) bool {
	query = strings.ToLower(query)
	for _, ch := range strings.ToLower(name) {
		if query != "" && rune(query[0]) == ch {
			query = query[1:]
		}
	}
	return query == ""
}

func skillInputs(value string, skills skillList) []map[string]any {
	var result []map[string]any
	seen := map[string]bool{}
	for _, token := range strings.FieldsFunc(value, func(r rune) bool {
		return !(unicode.IsLetter(r) || unicode.IsNumber(r) || strings.ContainsRune("$_-:", r))
	}) {
		if !strings.HasPrefix(token, "$") {
			continue
		}
		for _, d := range skills.Data {
			for _, s := range d.Skills {
				if s.enabled() && token[1:] == s.Name && !seen[s.Path] {
					seen[s.Path] = true
					result = append(result, map[string]any{"type": "skill", "name": s.Name, "path": s.Path})
				}
			}
		}
	}
	return result
}
