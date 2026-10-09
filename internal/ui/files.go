package ui

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/allquixotic/fastrock/internal/platform"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func (a *App) openPath(path string, reveal bool) {
	if !reveal && executablePath(path) {
		a.confirm("Run this file?", "Opening this file can execute code: "+path, func() { a.openPathNow(path, false) })
		return
	}
	a.openPathNow(path, reveal)
}
func executablePath(path string) bool {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".exe", ".com", ".bat", ".cmd", ".ps1", ".vbs", ".js", ".wsf", ".scr", ".msi", ".app", ".command", ".sh", ".lnk", ".url", ".desktop":
		return true
	}
	return false
}
func (a *App) openPathNow(path string, reveal bool) {
	a.work(func() {
		var cmd *exec.Cmd
		switch runtime.GOOS {
		case "darwin":
			if reveal {
				cmd = platform.Command("open", "-R", path)
			} else {
				cmd = platform.Command("open", path)
			}
		case "windows":
			if reveal {
				cmd = platform.Command("explorer.exe", "/select,"+path)
			} else {
				cmd = platform.Command("rundll32.exe", "url.dll,FileProtocolHandler", path)
			}
		default:
			cmd = platform.Command("xdg-open", path)
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

func (a *App) ensureFileEditor(v *fileView) {
	if !v.Loaded {
		a.reloadFile(v)
		return
	}
	if v.Loading || v.Closed {
		return
	}
	v.Loading = true
	v.LoadGeneration++
	generation, value := v.LoadGeneration, v.LoadedText
	a.work(func() {
		editor := loadedFileEditor(value)
		a.post(func() {
			if v.Closed || generation != v.LoadGeneration {
				return
			}
			v.Loading = false
			v.installEditor(editor)
		})
	}, func() { v.Loading = false })
}

func loadedFileEditor(value string) *nucular.TextEditor {
	editor := textEditor(value, true)
	editor.Maxlen = maxFileBytes + 1
	editor.Flags |= nucular.EditReadOnly
	return editor
}

func (v *fileView) installEditor(editor *nucular.TextEditor) {
	v.resetFileSearch()
	v.Editor = editor
	if !v.Wrap {
		editor.Flags &^= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
	}
	if v.PendingPosition != nil {
		v.PendingPosition.apply(editor)
		v.PendingPosition = nil
		editor.CursorFollow = true
	}
}

type filePage struct {
	value string
	stamp fileStamp
	next  int64
	more  bool
	limit bool
}

var errFileChanged = errors.New("file changed while loading; reload to read the current version")
var errFileNotUTF8 = errors.New("file is not UTF-8 text; use Show as text or Open externally")

func readFilePage(path string, offset int64, previous string, expected fileStamp, lossy bool) (filePage, error) {
	f, err := os.Open(path)
	if err != nil {
		return filePage{}, err
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		return filePage{}, err
	}
	if !info.Mode().IsRegular() {
		return filePage{}, errors.New("only regular text files can be opened")
	}
	stamp := stampFile(info)
	if expected.Valid && !expected.matches(stamp) {
		return filePage{}, errFileChanged
	}
	if offset < 0 || offset >= maxFileBytes {
		return filePage{}, errors.New("file preview limit reached; open the full file externally")
	}
	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return filePage{}, err
	}
	data, err := io.ReadAll(io.LimitReader(f, min(256<<10, maxFileBytes-offset)))
	if err != nil {
		return filePage{}, err
	}
	// Check both the open handle and the path: an atomic replacement leaves the
	// old handle readable but must not become a new baseline for later pages.
	for _, stat := range []func() (os.FileInfo, error){f.Stat, func() (os.FileInfo, error) { return os.Stat(path) }} {
		after, err := stat()
		if err != nil {
			return filePage{}, err
		}
		if !stamp.matches(stampFile(after)) {
			return filePage{}, errFileChanged
		}
	}
	rawNext := offset + int64(len(data))
	hasMore := rawNext < info.Size()
	data = utf8PagePrefix(data, hasMore)
	next := offset + int64(len(data))
	if !utf8.Valid(data) {
		if !lossy {
			return filePage{}, errFileNotUTF8
		}
		data = bytes.ToValidUTF8(data, []byte("�"))
	}
	limited := hasMore && rawNext >= maxFileBytes
	return filePage{value: previous + string(data), stamp: stamp, next: next, more: hasMore && !limited, limit: limited}, nil
}

func (a *App) loadFilePage(v *fileView, more bool) {
	if v.Virtual || v.Loading || v.Closed {
		return
	}
	v.Loading = true
	v.LoadGeneration++
	generation, path, lossy := v.LoadGeneration, v.Path, v.Lossy
	offset := int64(0)
	previous := ""
	var expected fileStamp
	if more {
		offset = v.Offset
		previous, expected = v.fileText(), v.Stamp
	}
	if v.Editor != nil {
		p := position(v.Editor)
		v.PendingPosition = &p
	}
	a.work(func() {
		page, err := readFilePage(path, offset, previous, expected, lossy)
		var editor *nucular.TextEditor
		if err == nil {
			editor = loadedFileEditor(page.value)
		}
		a.post(func() {
			if v.Closed || generation != v.LoadGeneration {
				return
			}
			v.Loading = false
			if err != nil {
				v.Error = err.Error()
				v.NonUTF8 = errors.Is(err, errFileNotUTF8)
				if errors.Is(err, os.ErrNotExist) {
					v.FileNotice = fileNotice(v.Stamp, nil, err)
				} else if errors.Is(err, errFileChanged) {
					v.FileNotice = fileChangedNotice
				}
				return
			}
			v.More, v.LimitReached, v.Offset = page.more, page.limit, page.next
			v.Loaded, v.LoadedText, v.Stamp = true, page.value, page.stamp
			v.installEditor(editor)
			v.Error = ""
			v.NonUTF8 = false
			v.FileNotice = ""
			a.watchFile(v)
			if v.PendingLine > 0 {
				a.goToFileLine(v, v.PendingLine, v.PendingColumn)
			}
		})
	}, func() { v.Loading = false; v.Error = errWorkQueueFull.Error() })
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
			if !found && !v.More {
				a.toast = "That line is beyond the loaded file; open the full file externally"
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
	data = utf8PagePrefix(data, more)
	if !utf8.Valid(data) {
		return nil, errFileNotUTF8
	}
	return data, nil
}

func utf8PagePrefix(data []byte, more bool) []byte {
	if more && len(data) > 0 {
		start := len(data) - 1
		for start > 0 && len(data)-start < utf8.UTFMax && !utf8.RuneStart(data[start]) {
			start--
		}
		if !utf8.FullRune(data[start:]) {
			data = data[:start]
		}
	}
	return data
}
func findText(value, query string, start int, back bool) (int, int) {
	hay, needle := []rune(value), []rune(query)
	if len(needle) == 0 {
		return -1, -1
	}
	fold := func(r rune) rune {
		minimum := r
		for n := unicode.SimpleFold(r); n != r; n = unicode.SimpleFold(n) {
			if n < minimum {
				minimum = n
			}
		}
		return minimum
	}
	for i := range needle {
		needle[i] = fold(needle[i])
	}
	matches := []int{}
	for i := 0; i+len(needle) <= len(hay); i++ {
		ok := true
		for j, n := range needle {
			if fold(hay[i+j]) != n {
				ok = false
				break
			}
		}
		if ok {
			matches = append(matches, i)
		}
	}
	if len(matches) == 0 {
		return -1, -1
	}
	at := matches[0]
	if back {
		at = matches[len(matches)-1]
		for i := len(matches) - 1; i >= 0; i-- {
			if matches[i] < start {
				at = matches[i]
				break
			}
		}
	} else {
		for _, i := range matches {
			if i >= start {
				at = i
				break
			}
		}
	}
	return at, at + len(needle)
}
func (a *App) drawFile(w *nucular.Window, v *fileView) {
	if v == nil {
		return
	}
	title(w, v.Path, a.p)
	if len(v.Diff) > 0 || v.DiffSummary != "" {
		a.drawDiff(w, v)
		return
	}
	w.Row(28).Static(70, 70, 75)
	if w.ButtonText("Find") {
		v.FindOpen = !v.FindOpen
		v.FocusFind = v.FindOpen
	}
	if w.ButtonText("Go to…") {
		a.fileGoTo(v)
	}
	if w.CheckboxText("Wrap", &v.Wrap) && v.Editor != nil {
		if v.Wrap {
			v.Editor.Flags |= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
		} else {
			v.Editor.Flags &^= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
		}
	}
	if !v.Virtual {
		w.Row(28).Static(85, 100, 120, 95)
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
	}
	if v.More || v.LimitReached {
		muted(w, "Copy and Save use the loaded text only.", a.p)
	}
	w.Row(27).Static(95, 100)
	if w.ButtonText("Copy all") && v.Editor != nil {
		a.copyText(text(v.Editor))
	}
	if w.ButtonText("Save as…") && v.Editor != nil {
		a.saveTextAs(v.Path, v.fileText())
	}
	if v.FindOpen {
		if v.Find == nil {
			v.Find = textEditor("", false)
		}
		w.Row(28).Ratio(.7, .1, .1, .1)
		if v.FocusFind {
			v.FocusFind = false
			w.Master().ActivateEditor(w, v.Find)
		}
		v.Find.Edit(w)
		if w.ButtonText("Previous") {
			a.fileFind(v, true)
		}
		if w.ButtonText("Next") {
			a.fileFind(v, false)
		}
		if w.ButtonText("Close") {
			v.FindOpen, v.FocusFile = false, true
		}
		a.prepareFileAnalysis(v)
		w.Row(24).Dynamic(1)
		color := a.p.Muted
		if v.findLabel() == "No results" || v.Search.err != "" {
			color = a.p.Danger
		}
		w.LabelColored(v.findLabel(), "LC", color)
		if v.Search.err != "" {
			w.Row(26).Dynamic(1)
			if w.ButtonText("Retry search") {
				v.Search.editor = nil
			}
		}
	}
	if v.Error != "" {
		muted(w, v.Error, a.p)
		if v.NonUTF8 {
			w.Row(28).Static(175)
			if w.ButtonText("Show as text (lossy)") {
				v.Lossy = true
				a.reloadFile(v)
			}
		}
		if v.Editor == nil && !v.Loaded {
			return
		}
	}
	if v.FileNotice != "" {
		muted(w, v.FileNotice, a.p)
	}
	if v.Lossy {
		muted(w, "Invalid UTF-8 bytes are shown as �. Copy and Save use this displayed text.", a.p)
	}
	if len(v.Diff) > 0 || v.DiffSummary != "" {
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
	scale := w.Master().Style().Scaling
	w.RowScaled(max(int(80*scale), w.LayoutAvailableHeight()-int(30*scale))).Dynamic(1)
	if v.Editor != nil {
		a.prepareFileAnalysis(v)
		face := typeFace(fontPointSize(w.Master().Style().Font)-1, monoFont)
		a.decorateFile(v, scale, face)
		if v.FocusFile {
			v.FocusFile = false
			w.Master().ActivateEditor(w, v.Editor)
		}
		codeEditor(w, v.Editor)
	} else {
		w.Label("Loading…", "LC")
	}
	w.Row(24).Dynamic(1)
	w.LabelColored(v.fileStatus(), "LC", a.p.Muted)
}
func (a *App) showDiff(c *workspace.Conversation) {
	a.work(func() {
		cmd := platform.CommandContext(a.ctx, "git", "-C", c.Cwd, "diff", "--no-ext-diff", "HEAD")
		out, err := boundedCommandOutput(cmd, 4<<20)
		a.post(func() {
			if len(out) != 0 {
				a.openDiff("Changes · "+c.Title, string(out), c.Cwd)
			}
			if err != nil {
				a.report(err)
			}
		})
	})
}
func (a *App) continueWorktree(c *workspace.Conversation, parent string) {
	a.work(func() {
		path := filepath.Join(parent, fmt.Sprintf("fastrock-%d", time.Now().Unix()))
		git := func(dir string, args ...string) ([]byte, error) {
			argv := append([]string{"-C", dir}, args...)
			return platform.CommandContext(a.ctx, "git", argv...).CombinedOutput()
		}
		patch, err := git(c.Cwd, "diff", "--binary", "HEAD")
		if err == nil {
			_, err = git(c.Cwd, "worktree", "add", "--detach", path, "HEAD")
		}
		if err == nil && len(patch) > 0 {
			cmd := platform.CommandContext(a.ctx, "git", "-C", path, "apply", "--binary")
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

func boundedCommandOutput(cmd *exec.Cmd, limit int64) ([]byte, error) {
	pipe, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	if err = cmd.Start(); err != nil {
		return nil, err
	}
	data, readErr := io.ReadAll(io.LimitReader(pipe, limit+1))
	if int64(len(data)) > limit {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		return data[:limit], fmt.Errorf("diff exceeds %d MiB; use an external viewer for the complete diff", limit>>20)
	}
	err = cmd.Wait()
	if readErr != nil {
		return nil, readErr
	}
	return data, err
}

func (a *App) fileGoTo(v *fileView) {
	a.inputDialog("Go to line", "1", func(value string) {
		line, err := strconv.Atoi(value)
		if err != nil || line < 1 {
			a.toast = "Enter a positive line number"
			return
		}
		a.goToFileLine(v, line, 1)
	})
}
