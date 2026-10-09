package ui

import (
	"context"
	"errors"
	"os"
	"sync"
	"time"
)

// The portable fields travel with a popped-out file; Info additionally detects
// atomic replacement with an identical size and modification time locally.
type fileStamp struct {
	Valid    bool
	Size     int64
	Modified time.Time
	Info     os.FileInfo `json:"-"`
}

func stampFile(info os.FileInfo) fileStamp {
	return fileStamp{Valid: true, Size: info.Size(), Modified: info.ModTime(), Info: info}
}

func (s fileStamp) matches(other fileStamp) bool {
	return s.Valid && other.Valid && s.Size == other.Size && s.Modified.Equal(other.Modified) &&
		(s.Info == nil || other.Info == nil || os.SameFile(s.Info, other.Info))
}

const fileChangedNotice = "This file changed on disk. Showing the last loaded version. Reload to see the changes."
const fileMissingNotice = "This file was deleted or moved. Showing the last loaded version."

func fileNotice(baseline fileStamp, info os.FileInfo, err error) string {
	if errors.Is(err, os.ErrNotExist) {
		if baseline.Valid {
			return fileMissingNotice
		}
		return "This file was deleted or moved."
	}
	if err != nil {
		return "Cannot check the file on disk: " + err.Error()
	}
	if baseline.Valid && !baseline.matches(stampFile(info)) {
		return fileChangedNotice
	}
	return ""
}

type watchedFile struct {
	path     string
	baseline fileStamp
	notice   string
	publish  func(string)
}

// One polling service serves every file tab. It never occupies the bounded
// reader pool or reads mutable view/editor state from a background goroutine.
type fileWatcher struct {
	mu      sync.Mutex
	entries map[*watchedFile]struct{}
}

func (w *fileWatcher) watch(path string, baseline fileStamp, initialNotice string, publish func(string)) func() {
	entry := &watchedFile{path: path, baseline: baseline, notice: initialNotice, publish: publish}
	w.mu.Lock()
	if w.entries == nil {
		w.entries = make(map[*watchedFile]struct{})
	}
	w.entries[entry] = struct{}{}
	w.mu.Unlock()
	return func() {
		w.mu.Lock()
		delete(w.entries, entry)
		w.mu.Unlock()
	}
}

func (w *fileWatcher) run(ctx context.Context) {
	timer := time.NewTicker(2 * time.Second)
	defer timer.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-timer.C:
			w.poll(ctx)
		}
	}
}

func (w *fileWatcher) poll(ctx context.Context) {
	w.mu.Lock()
	entries := make([]*watchedFile, 0, len(w.entries))
	for entry := range w.entries {
		entries = append(entries, entry)
	}
	w.mu.Unlock()
	for _, entry := range entries {
		if ctx.Err() != nil {
			return
		}
		info, err := os.Stat(entry.path)
		notice := fileNotice(entry.baseline, info, err)
		w.mu.Lock()
		_, live := w.entries[entry]
		changed := live && notice != entry.notice
		if changed {
			entry.notice = notice
		}
		w.mu.Unlock()
		if changed {
			entry.publish(notice)
		}
	}
}

func (a *App) watchFile(v *fileView) {
	v.WatchGeneration++
	if v.WatchCancel != nil {
		v.WatchCancel()
		v.WatchCancel = nil
	}
	if v.Virtual || v.Closed || a.ctx == nil {
		return
	}
	if a.fileWatcher == nil {
		a.fileWatcher = &fileWatcher{}
		go a.fileWatcher.run(a.ctx)
	}
	generation := v.WatchGeneration
	v.WatchCancel = a.fileWatcher.watch(v.Path, v.Stamp, v.FileNotice, func(notice string) {
		a.post(func() {
			if !v.Closed && v.WatchGeneration == generation {
				v.FileNotice = notice
			}
		})
	})
}

func (v *fileView) fileText() string {
	if v.Loaded {
		return v.LoadedText
	}
	if v.Editor != nil {
		return text(v.Editor)
	}
	return v.LoadedText
}

func (v *fileView) filePosition() editorPosition {
	if v.Editor == nil && v.PendingPosition != nil {
		return *v.PendingPosition
	}
	return position(v.Editor)
}

func (v *fileView) evictEditor() {
	if v.Editor == nil || !v.Loaded || v.Loading {
		return
	}
	p := position(v.Editor)
	v.PendingPosition = &p
	v.resetFileSearch()
	v.Editor = nil
}

func (v *fileView) dispose() {
	v.resetFileSearch()
	if v.ReadCancel != nil {
		v.ReadCancel()
		v.ReadCancel = nil
	}
	v.Closed = true
	v.DiffGeneration++
	v.LoadGeneration++
	v.WatchGeneration++
	if v.WatchCancel != nil {
		v.WatchCancel()
		v.WatchCancel = nil
	}
}
