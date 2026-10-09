package ui

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func (a *App) openPath(path string, reveal bool) {
	a.work(func() {
		var cmd *exec.Cmd
		switch runtime.GOOS {
		case "darwin":
			if reveal {
				cmd = exec.Command("open", "-R", path)
			} else {
				cmd = exec.Command("open", path)
			}
		case "windows":
			if reveal {
				cmd = exec.Command("explorer.exe", "/select,"+path)
			} else {
				cmd = exec.Command("rundll32.exe", "url.dll,FileProtocolHandler", path)
			}
		default:
			cmd = exec.Command("xdg-open", path)
		}
		if err := cmd.Run(); err != nil {
			a.post(func() { a.report(err) })
		}
	})
}

const maxFileBytes = 4 << 20

// Files are paged independently of text layout. Work and UTF-8 conversion stay
// off the UI thread; only the bounded prepared editor is published.
func (a *App) reloadFile(v *fileView) { a.loadFilePage(v, false) }
func (a *App) loadFilePage(v *fileView, more bool) {
	if v.Virtual || v.Loading {
		return
	}
	v.Loading = true
	offset := int64(0)
	previous := ""
	if more {
		offset = v.Offset
		previous = text(v.Editor)
	}
	a.work(func() {
		f, err := os.Open(v.Path)
		var data []byte
		hasMore := false
		if err == nil {
			defer f.Close()
			_, err = f.Seek(offset, io.SeekStart)
			if err == nil {
				data, err = io.ReadAll(io.LimitReader(f, 256<<10))
				if err == nil {
					if info, e := f.Stat(); e == nil {
						hasMore = offset+int64(len(data)) < info.Size()
					}
				}
			}
		}
		if err == nil {
			data, err = completeUTF8Page(data, hasMore)
		}
		next := offset + int64(len(data))
		limited := hasMore && next >= maxFileBytes
		value := previous + string(data)
		editor := textEditor(value, true)
		editor.Flags |= nucular.EditReadOnly
		a.post(func() {
			v.Loading = false
			v.More = hasMore && !limited
			v.LimitReached = limited
			v.Offset = next
			if err != nil {
				v.Error = err.Error()
				return
			}
			v.Editor = editor
			if !v.Wrap {
				editor.Flags &^= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
			}
			if v.PendingPosition != nil {
				v.PendingPosition.apply(editor)
				v.PendingPosition = nil
				editor.CursorFollow = true
			}
			v.Error = ""
			if v.PendingLine > 0 {
				a.goToFileLine(v, v.PendingLine, v.PendingColumn)
			}
		})
	})
}

func fileLinePosition(buf []rune, line, column int) (int, bool) {
	line = max(1, line)
	start := 0
	for line > 1 && start < len(buf) {
		if buf[start] == '\n' {
			line--
		}
		start++
	}
	if line > 1 {
		return len(buf), false
	}
	for column > 1 && start < len(buf) && buf[start] != '\n' {
		column--
		start++
	}
	return start, true
}
func (a *App) goToFileLine(v *fileView, line, column int) {
	if v.Editor == nil || v.Loading {
		return
	}
	editor := v.Editor
	a.work(func() {
		pos, found := fileLinePosition(editor.Buffer, line, column)
		a.post(func() {
			if v.Editor != editor {
				return
			}
			if !found && v.More {
				v.PendingLine, v.PendingColumn = line, column
				a.loadFilePage(v, true)
				return
			}
			v.PendingLine, v.PendingColumn = 0, 0
			editor.Cursor, editor.SelectStart, editor.SelectEnd = pos, pos, pos
			editor.CursorFollow = true
		})
	})
}

// Exclude only an incomplete trailing rune. Invalid interior bytes identify a
// binary/non-UTF-8 file instead of silently deleting arbitrary bytes.
func completeUTF8Page(data []byte, more bool) ([]byte, error) {
	if more && len(data) > 0 {
		start := len(data) - 1
		for start > 0 && len(data)-start < utf8.UTFMax && !utf8.RuneStart(data[start]) {
			start--
		}
		if !utf8.FullRune(data[start:]) {
			data = data[:start]
		}
	}
	if !utf8.Valid(data) {
		return nil, errors.New("file is not UTF-8 text; use Open externally")
	}
	return data, nil
}
func findText(value, query string, start int, back bool) (int, int) {
	hay, needle := strings.ToLower(value), strings.ToLower(query)
	if needle == "" {
		return -1, -1
	}
	pos := 0
	for i := range hay {
		if start <= 0 {
			pos = i
			break
		}
		start--
		pos = len(hay)
	}
	at := -1
	if back {
		at = strings.LastIndex(hay[:pos], needle)
		if at < 0 {
			at = strings.LastIndex(hay, needle)
		}
	} else {
		if n := strings.Index(hay[pos:], needle); n >= 0 {
			at = pos + n
		} else {
			at = strings.Index(hay, needle)
		}
	}
	if at < 0 {
		return -1, -1
	}
	begin := utf8.RuneCountInString(hay[:at])
	return begin, begin + utf8.RuneCountInString(needle)
}
func (a *App) fileFind(v *fileView, back bool) {
	if v.Editor == nil || text(v.Find) == "" {
		return
	}
	editor, query, value := v.Editor, text(v.Find), text(v.Editor)
	start := editor.SelectEnd
	if back {
		start = editor.SelectStart
	}
	v.SearchGeneration++
	generation := v.SearchGeneration
	a.work(func() {
		begin, end := findText(value, query, start, back)
		a.post(func() {
			if v.Editor != editor || v.SearchGeneration != generation {
				return
			}
			if begin < 0 {
				a.toast = "No matches"
				return
			}
			editor.SelectStart = begin
			editor.SelectEnd = end
			editor.Cursor = end
			editor.CursorFollow = true
		})
	})
}
func (a *App) drawFile(w *nucular.Window, v *fileView) {
	if v == nil {
		return
	}
	title(w, v.Path, a.p)
	w.Row(28).Static(70, 70, 75, 85, 100, 90, 95)
	if w.ButtonText("Find") {
		v.FindOpen = !v.FindOpen
	}
	if w.ButtonText("Go to…") {
		a.inputDialog("Go to line", "1", func(value string) {
			line, err := strconv.Atoi(value)
			if err != nil || line < 1 {
				a.toast = "Enter a positive line number"
				return
			}
			a.goToFileLine(v, line, 1)
		})
	}
	if w.CheckboxText("Wrap", &v.Wrap) && v.Editor != nil {
		if v.Wrap {
			v.Editor.Flags |= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
		} else {
			v.Editor.Flags &^= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
		}
	}
	if w.ButtonText("Copy path") {
		a.copyText(v.Path)
	}
	if w.ButtonText("Reveal") {
		a.openPath(v.Path, true)
	}
	if w.ButtonText("Open externally") {
		a.openPath(v.Path, false)
	}
	if w.ButtonText("Reload") {
		a.reloadFile(v)
	}
	w.Row(27).Static(95, 100)
	if w.ButtonText("Copy all") && v.Editor != nil {
		a.copyText(text(v.Editor))
	}
	if w.ButtonText("Save as…") && v.Editor != nil {
		value := text(v.Editor)
		a.choosePath(true, false, func(path string) {
			a.work(func() { err := os.WriteFile(path, []byte(value), 0600); a.post(func() { a.report(err) }) })
		})
	}
	if v.FindOpen {
		w.Row(28).Ratio(.7, .1, .1, .1)
		v.Find.Edit(w)
		if w.ButtonText("Previous") {
			a.fileFind(v, true)
		}
		if w.ButtonText("Next") {
			a.fileFind(v, false)
		}
		if w.ButtonText("Close") {
			v.FindOpen = false
		}
	}
	if v.Error != "" {
		muted(w, v.Error, a.p)
		return
	}
	if len(v.Diff) > 0 {
		a.drawDiff(w, v)
		return
	}
	if v.LimitReached {
		muted(w, "Showing the first 4 MiB. Open externally to view the full file.", a.p)
	}
	if v.More {
		w.Row(28).Dynamic(1)
		if w.ButtonText("Load more text") {
			a.loadFilePage(v, true)
		}
	}
	w.Row(max(140, w.LayoutAvailableHeight()-8)).Dynamic(1)
	if v.Editor != nil {
		v.Editor.Edit(w)
	} else {
		w.Label("Loading…", "LC")
	}
}
func (a *App) showDiff(c *workspace.Conversation) {
	a.work(func() {
		cmd := exec.CommandContext(a.ctx, "git", "-C", c.Cwd, "diff", "--no-ext-diff", "HEAD")
		out, err := cmd.Output()
		a.post(func() {
			if err != nil {
				a.report(err)
			} else {
				a.openDiff("Changes · "+c.Title, string(out), c.Cwd)
			}
		})
	})
}
func (a *App) continueWorktree(c *workspace.Conversation, parent string) {
	a.work(func() {
		path := filepath.Join(parent, fmt.Sprintf("fastrock-%d", time.Now().Unix()))
		git := func(dir string, args ...string) ([]byte, error) {
			argv := append([]string{"-C", dir}, args...)
			return exec.CommandContext(a.ctx, "git", argv...).CombinedOutput()
		}
		patch, err := git(c.Cwd, "diff", "--binary", "HEAD")
		if err == nil {
			_, err = git(c.Cwd, "worktree", "add", "--detach", path, "HEAD")
		}
		if err == nil && len(patch) > 0 {
			cmd := exec.CommandContext(a.ctx, "git", "-C", path, "apply", "--binary")
			cmd.Stdin = bytes.NewReader(patch)
			out, e := cmd.CombinedOutput()
			if e != nil {
				err = fmt.Errorf("apply worktree changes: %s: %w", out, e)
			}
		}
		if err == nil {
			var names []byte
			names, err = git(c.Cwd, "ls-files", "--others", "--exclude-standard", "-z")
			if err == nil {
				for name := range strings.SplitSeq(string(names), "\x00") {
					if name == "" {
						continue
					}
					if err = copyWorktreeFile(c.Cwd, path, name); err != nil {
						break
					}
				}
			}
		}
		a.post(func() {
			if err != nil {
				a.report(fmt.Errorf("worktree %s: %w", path, err))
				return
			}
			a.rpc("thread/fork", map[string]any{"threadId": c.ID, "cwd": path}, func(raw json.RawMessage) {
				r := codex.Decode(raw)
				t, _ := r["thread"].(map[string]any)
				id := str(t, "id")
				if id != "" {
					a.state.Chats[id] = &workspace.Conversation{ID: id, Title: c.Title, Cwd: path}
					a.resumeThread(id)
				}
			})
		})
	})
}

func copyWorktreeFile(source, destination, name string) error {
	clean := filepath.Clean(name)
	if filepath.IsAbs(clean) || clean == ".." || strings.HasPrefix(clean, ".."+string(filepath.Separator)) {
		return errors.New("invalid worktree path")
	}
	from, to := filepath.Join(source, clean), filepath.Join(destination, clean)
	info, err := os.Lstat(from)
	if err != nil {
		return err
	}
	if err = os.MkdirAll(filepath.Dir(to), 0700); err != nil {
		return err
	}
	if info.Mode()&os.ModeSymlink != 0 {
		target, err := os.Readlink(from)
		if err != nil {
			return err
		}
		return os.Symlink(target, to)
	}
	if !info.Mode().IsRegular() {
		return nil
	}
	input, err := os.Open(from)
	if err != nil {
		return err
	}
	defer input.Close()
	output, err := os.OpenFile(to, os.O_CREATE|os.O_EXCL|os.O_WRONLY, info.Mode().Perm())
	if err != nil {
		return err
	}
	_, err = io.Copy(output, input)
	closeErr := output.Close()
	if err != nil {
		return err
	}
	return closeErr
}
