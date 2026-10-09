package ui

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
)

type approvalContent struct {
	Reason, Code, Caption string
	Details               []string
	Diff                  []approvalDiffLine
}
type approvalDiffLine struct{ Kind, Text string }
type approvalDiffEntry struct {
	Thread, Item string
	Lines        []approvalDiffLine
}

// Command display is descriptive only; wire decisions retain the original argv.
func approvalCommandWords(command string) []string {
	// POSIX shlex must not eat literal Windows path backslashes.
	if strings.Contains(command, `:\`) {
		return []string{command}
	}
	words, err := splitWordsForPlatform(command, false)
	if err != nil || len(words) == 0 {
		return []string{command}
	}
	return words
}
func approvalCommand(command string) string {
	words := approvalCommandWords(command)
	if len(words) < 2 {
		return command
	}
	name := strings.ToLower(filepath.Base(strings.ReplaceAll(words[0], `\`, "/")))
	name = strings.TrimSuffix(name, ".exe")
	switch name {
	case "bash", "zsh", "sh":
		if len(words) == 3 && (words[1] == "-c" || words[1] == "-lc") {
			return words[2]
		}
	case "powershell", "pwsh":
		for i := 1; i+1 < len(words); i++ {
			if strings.EqualFold(words[i], "-command") || strings.EqualFold(words[i], "-c") {
				// Only unwrap a complete script argument. Further arguments can
				// change what PowerShell runs, so retain the original command.
				if i+2 == len(words) {
					return words[i+1]
				}
				return command
			}
		}
	}
	return command
}
func approvalPrefix(value any) string {
	if m, ok := value.(map[string]any); ok {
		value = m["command"]
	}
	var words []string
	switch v := value.(type) {
	case []any:
		for _, x := range v {
			s, ok := x.(string)
			if !ok {
				return ""
			}
			words = append(words, s)
		}
	case []string:
		words = append([]string(nil), v...)
	case string:
		return v
	default:
		return ""
	}
	if len(words) == 0 {
		return ""
	}
	for i, s := range words {
		if strings.ContainsAny(s, "\r\n") {
			return ""
		}
		if strings.ContainsAny(s, " \t'\"\\$`;&|()<>") {
			words[i] = "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
		}
	}
	return strings.Join(words, " ")
}
func approvalPath(path, cwd string) string {
	if cwd != "" {
		if rel, err := filepath.Rel(cwd, path); err == nil && rel != "." && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			return rel
		}
	}
	home, _ := os.UserHomeDir()
	if home != "" {
		if rel, err := filepath.Rel(home, path); err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			if rel == "." {
				return "~"
			}
			return "~" + string(filepath.Separator) + rel
		}
	}
	return path
}
func permissionRule(permissions map[string]any) string {
	var parts []string
	network, _ := permissions["network"].(map[string]any)
	if enabled, _ := network["enabled"].(bool); enabled {
		parts = append(parts, "network")
	}
	fs, _ := permissions["fileSystem"].(map[string]any)
	if fs == nil {
		fs, _ = permissions["file_system"].(map[string]any)
	}
	groups := map[string][]string{}
	if entries, ok := fs["entries"].([]any); ok {
		for _, raw := range entries {
			entry, _ := raw.(map[string]any)
			path, _ := entry["path"].(map[string]any)
			label := ""
			switch str(path, "type") {
			case "path":
				label = approvalPath(str(path, "path"), "")
			case "glob_pattern", "globPattern", "glob-pattern":
				label = "glob " + str(path, "pattern")
			case "special":
				value, _ := path["value"].(map[string]any)
				label = fallback(str(value, "kind"), str(value, "type"))
				if label == "" {
					label = str(path, "value")
				}
				switch label {
				case "root":
					label = ":root"
				case "minimal":
					label = ":minimal"
				case "project_roots", "projectRoots", "project-roots":
					label = ":workspace_roots"
				case "tmpdir":
					label = ":tmpdir"
				case "slash_tmp", "slashTmp", "slash-tmp":
					label = "/tmp"
				case "unknown":
					label = str(value, "path")
				}
				if sub := str(value, "subpath"); sub != "" {
					label += "/" + sub
				}
			}
			if label != "" {
				groups[str(entry, "access")] = append(groups[str(entry, "access")], label)
			}
		}
	} else {
		for _, access := range []string{"read", "write"} {
			if paths, ok := fs[access].([]any); ok {
				for _, path := range paths {
					if s, ok := path.(string); ok {
						groups[access] = append(groups[access], approvalPath(s, ""))
					}
				}
			}
		}
	}
	for _, access := range []string{"read", "write", "deny"} {
		if paths := groups[access]; len(paths) > 0 {
			label := access
			if access == "deny" {
				label = "deny read"
			}
			parts = append(parts, label+" "+strings.Join(paths, ", "))
		}
	}
	return cut(strings.Join(parts, "; "), 8000)
}
func (a *App) prepareApproval(r *approval) {
	p := r.Params
	view := approvalContent{Reason: cut(str(p, "reason"), 8000)}
	cwd := str(p, "cwd")
	if cwd == "" {
		if c := a.state.Chats[r.OriginThreadID]; c != nil {
			cwd = c.Cwd
		}
	}
	switch r.Message.Method {
	case "item/commandExecution/requestApproval":
		network, _ := p["networkApprovalContext"].(map[string]any)
		switch {
		case network != nil:
			r.Title = "Allow network access to " + str(network, "host") + "?"
			scheme := str(network, "protocol")
			if scheme == "" {
				scheme = "https"
			}
			view.Code = scheme + "://" + str(network, "host")
			if command := str(p, "command"); strings.HasPrefix(command, "network-access ") {
				view.Code = strings.TrimPrefix(command, "network-access ")
			}
		case str(p, "kind") == "writeStdin":
			r.Title = "Send input to the running command?"
			words := approvalCommandWords(str(p, "command"))
			if len(words) > 0 {
				view.Code = fmt.Sprintf("%q", words[len(words)-1])
			}
		default:
			r.Title = "Run this command?"
			if p["additionalPermissions"] != nil {
				r.Title = "Run this command with additional permissions?"
			}
			if command := str(p, "command"); command != "" {
				view.Code = "$ " + approvalCommand(command)
			}
		}
		if network == nil && cwd != "" {
			view.Caption = "in " + approvalPath(cwd, "")
		}
		if permissions, ok := p["additionalPermissions"].(map[string]any); ok {
			if rule := permissionRule(permissions); rule != "" {
				view.Details = append(view.Details, "Permission rule: "+rule)
			}
		}
	case "item/permissions/requestApproval":
		r.Title = "Grant additional permissions?"
		permissions, _ := p["permissions"].(map[string]any)
		rule := permissionRule(permissions)
		if rule == "" {
			view.Details = append(view.Details, "No specific permissions were listed.")
		} else {
			view.Details = append(view.Details, "Permission rule: "+rule)
		}
		if cwd != "" {
			view.Caption = "in " + approvalPath(cwd, "")
		}
	case "item/fileChange/requestApproval":
		r.Title = "Apply these file changes?"
		view.Diff = a.cachedApprovalDiff(r.OriginThreadID, str(p, "itemId"))
		if len(view.Diff) == 0 {
			view.Details = append(view.Details, "The proposed changes are not available yet.")
		}
		if root := str(p, "grantRoot"); root != "" {
			view.Details = append(view.Details, "Codex also asks for write access under "+approvalPath(root, "")+" for the rest of this session.")
		}
	default:
		view.Details = []string{r.Details}
	}
	if env := str(p, "environmentId"); env != "" && env != "local" {
		view.Details = append(view.Details, "Environment: "+cut(env, 200))
	}
	view.Code = cut(view.Code, 32000)
	if r.Content.Code != view.Code {
		r.CodeEditor = nil
	}
	r.Content = view
}
func (a *App) cachedApprovalDiff(thread, item string) []approvalDiffLine {
	for _, e := range a.approvalDiffs {
		if e.Thread == thread && e.Item == item {
			return e.Lines
		}
	}
	if c := a.state.Chats[thread]; c != nil {
		for _, b := range c.Blocks {
			if b.ID == item && b.Kind == "fileChange" {
				var changes []any
				if json.Unmarshal([]byte(b.Text), &changes) == nil {
					return approvalDiff(changes, c.Cwd)
				}
			}
		}
	}
	return nil
}
func approvalDiff(changes []any, cwd string) []approvalDiffLine {
	lines := make([]approvalDiffLine, 0, min(len(changes)*8, 400))
	omitted := false
	appendLine := func(kind, text string) {
		if len(lines) >= 400 {
			omitted = true
			return
		}
		lines = append(lines, approvalDiffLine{kind, cut(strings.ReplaceAll(text, "\t", "    "), 400)})
	}
	for _, raw := range changes {
		change, _ := raw.(map[string]any)
		kind, _ := change["kind"].(map[string]any)
		verb := str(kind, "type")
		if verb == "" {
			verb = "update"
		}
		diff := str(change, "diff")
		if move := fallback(str(kind, "movePath"), str(kind, "move_path")); move != "" {
			diff = strings.TrimSuffix(diff, "\n\nMoved to: "+move)
		}
		// Count the complete supplied patch while retaining only bounded preview lines.
		added, removed := walkApprovalDiff(diff, verb, nil)
		path := approvalPath(str(change, "path"), cwd)
		title := map[string]string{"add": "Added ", "delete": "Deleted ", "update": "Edited "}[verb]
		if title == "" {
			title = "Edited "
		}
		if move := fallback(str(kind, "movePath"), str(kind, "move_path")); move != "" {
			title = "Moved "
			path += " → " + approvalPath(move, cwd)
		}
		appendLine("file", fmt.Sprintf("%s%s (+%d −%d)", title, path, added, removed))
		walkApprovalDiff(diff, verb, appendLine)
		if omitted {
			break
		}
	}
	if omitted {
		lines = append(lines, approvalDiffLine{"note", "Preview limited to 400 lines."})
	}
	return lines
}

// Stream the patch without allocating a row for every source line. Counted
// hunks distinguish source lines beginning with ---/+++ from file headers.
func walkApprovalDiff(diff, verb string, visit func(string, string)) (added, removed int) {
	oldLeft, newLeft := 0, 0
	for diff != "" {
		line, rest, _ := strings.Cut(diff, "\n")
		diff = rest
		line = strings.TrimSuffix(line, "\r")
		kind := "context"
		switch verb {
		case "add":
			kind, line = "add", "+"+line
		case "delete":
			kind, line = "remove", "-"+line
		default:
			inHunk := false
			switch {
			case strings.HasPrefix(line, "+") && newLeft > 0:
				newLeft--
				kind, inHunk = "add", true
			case strings.HasPrefix(line, "-") && oldLeft > 0:
				oldLeft--
				kind, inHunk = "remove", true
			case (line == "" || strings.HasPrefix(line, " ")) && oldLeft > 0 && newLeft > 0:
				oldLeft--
				newLeft--
				inHunk = true
			case strings.HasPrefix(line, "\\"):
				kind, inHunk = "note", true
			}
			if !inHunk {
				oldLeft, newLeft = 0, 0
				if _, old, _, next, ok := diffHunkHeader(line); ok {
					oldLeft, newLeft = old, next
					kind = "hunk"
				} else if strings.HasPrefix(line, "@@") {
					kind = "hunk"
				} else if strings.HasPrefix(line, "--- ") {
					newHeader, after, _ := strings.Cut(diff, "\n")
					if strings.HasPrefix(newHeader, "+++ ") && strings.HasPrefix(after, "@@") {
						diff = after
						continue
					}
					kind = "remove"
				} else if strings.HasPrefix(line, "+") {
					kind = "add"
				} else if strings.HasPrefix(line, "-") {
					kind = "remove"
				} else if approvalDiffMetadata(line) {
					continue
				}
			}
		}
		if kind == "add" {
			added++
		} else if kind == "remove" {
			removed++
		}
		if visit != nil {
			visit(kind, line)
		}
	}
	return
}
func approvalDiffMetadata(line string) bool {
	for _, prefix := range []string{"diff ", "index ", "old mode ", "new mode ", "deleted file mode ", "new file mode ", "similarity index ", "dissimilarity index ", "rename from ", "rename to ", "copy from ", "copy to "} {
		if strings.HasPrefix(line, prefix) {
			return true
		}
	}
	return false
}
func (a *App) cacheApprovalDiff(thread, item string, changes []any) {
	cwd := ""
	if c := a.state.Chats[thread]; c != nil {
		cwd = c.Cwd
	}
	entry := approvalDiffEntry{thread, item, approvalDiff(changes, cwd)}
	for i, e := range a.approvalDiffs {
		if e.Thread == thread && e.Item == item {
			a.approvalDiffs = append(a.approvalDiffs[:i], a.approvalDiffs[i+1:]...)
			break
		}
	}
	a.approvalDiffs = append(a.approvalDiffs, entry)
	if len(a.approvalDiffs) > 64 {
		a.approvalDiffs = append([]approvalDiffEntry(nil), a.approvalDiffs[len(a.approvalDiffs)-64:]...)
	}
	for i := range a.approvals {
		r := &a.approvals[i]
		if r.OriginThreadID == thread && str(r.Params, "itemId") == item {
			a.prepareApproval(r)
			r.Armed = time.Now().Add(300 * time.Millisecond)
		}
	}
}
func (a *App) approvalEvent(method string, p map[string]any) {
	thread := str(p, "threadId")
	switch method {
	case "item/fileChange/patchUpdated":
		changes, _ := p["changes"].([]any)
		a.cacheApprovalDiff(thread, str(p, "itemId"), changes)
	case "item/started", "item/completed":
		item, _ := p["item"].(map[string]any)
		if str(item, "type") == "fileChange" {
			changes, _ := item["changes"].([]any)
			a.cacheApprovalDiff(thread, str(item, "id"), changes)
		}
	case "turn/completed", "thread/closed":
		keep := a.approvalDiffs[:0]
		for _, e := range a.approvalDiffs {
			if e.Thread != thread {
				keep = append(keep, e)
			}
		}
		clear(a.approvalDiffs[len(keep):])
		a.approvalDiffs = keep
	}
}
func (a *App) fetchApprovalDiff(r approval) {
	if r.Message.Method != "item/fileChange/requestApproval" || len(r.Content.Diff) > 0 || str(r.Params, "turnId") == "" {
		return
	}
	client := a.client
	generation := a.serverGeneration
	a.rpcInline("thread/items/list", map[string]any{"threadId": r.OriginThreadID, "turnId": str(r.Params, "turnId"), "limit": 100, "sortDirection": "desc"}, func(raw json.RawMessage) {
		if client != a.client || generation != a.serverGeneration || len(a.cachedApprovalDiff(r.OriginThreadID, str(r.Params, "itemId"))) > 0 {
			return
		}
		live := false
		for _, pending := range a.approvals {
			if pending.Message.Origin == r.Message.Origin && string(pending.Message.ID) == string(r.Message.ID) {
				live = true
			}
		}
		if !live {
			return
		}
		data, _ := codexJSON(raw)["data"].([]any)
		for _, raw := range data {
			entry, _ := raw.(map[string]any)
			item, _ := entry["item"].(map[string]any)
			if str(item, "id") == str(r.Params, "itemId") && str(item, "type") == "fileChange" {
				changes, _ := item["changes"].([]any)
				a.cacheApprovalDiff(r.OriginThreadID, str(item, "id"), changes)
				return
			}
		}
	}, func(error) {})
}
func codexJSON(raw json.RawMessage) map[string]any {
	var m map[string]any
	_ = json.Unmarshal(raw, &m)
	return m
}
func approvalCodeEditor(r *approval) *desktop.TextEditor {
	if r.CodeEditor == nil {
		r.CodeEditor = textEditor(r.Content.Code, true)
		r.CodeEditor.Flags |= desktop.EditReadOnly
		r.CodeEditor.Flags &^= desktop.EditSoftWrap | desktop.EditNoHorizontalScroll
	}
	return r.CodeEditor
}
