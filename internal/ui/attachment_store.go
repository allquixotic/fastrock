package ui

import (
	"crypto/sha256"
	hexencoding "encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func storeClipboardImage(root string, data []byte) (string, error) {
	if len(data) > maxAttachmentBytes {
		return "", fmt.Errorf("clipboard image exceeds 25 MiB")
	}
	dir := filepath.Join(root, "attachments")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return "", err
	}
	sum := sha256.Sum256(data)
	path := filepath.Join(dir, hexencoding.EncodeToString(sum[:])+".png")
	if info, err := os.Lstat(path); err == nil {
		if !info.Mode().IsRegular() || info.Size() != int64(len(data)) {
			return "", fmt.Errorf("stored clipboard image is not a valid regular file")
		}
		f, err := os.Open(path)
		if err != nil {
			return "", err
		}
		h := sha256.New()
		_, err = io.Copy(h, io.LimitReader(f, maxAttachmentBytes+1))
		closeErr := f.Close()
		if err != nil {
			return "", err
		}
		if closeErr != nil {
			return "", closeErr
		}
		if hexencoding.EncodeToString(h.Sum(nil)) != hexencoding.EncodeToString(sum[:]) {
			return "", fmt.Errorf("stored clipboard image failed its content check")
		}
		if _, err := attachmentInputs([]string{path}); err != nil {
			return "", err
		}
		// Renew the grace period before the next session checkpoint references it.
		now := time.Now()
		if err := os.Chtimes(path, now, now); err != nil {
			return "", err
		}
		return path, nil
	} else if !os.IsNotExist(err) {
		return "", err
	}
	var total int64
	files, err := os.ReadDir(dir)
	if err != nil {
		return "", err
	}
	for _, f := range files {
		if info, e := f.Info(); e == nil {
			total += info.Size()
		}
	}
	if total+int64(len(data)) > 512<<20 {
		return "", fmt.Errorf("saved clipboard images exceed 512 MiB; remove unused attachments or attach an existing file")
	}
	f, err := os.CreateTemp(dir, ".clipboard-*.png")
	if err != nil {
		return "", err
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if _, err = f.Write(data); err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err != nil {
		return "", err
	}
	if closeErr != nil {
		return "", closeErr
	}
	if _, err = attachmentInputs([]string{tmp}); err != nil {
		return "", err
	}
	if err = os.Rename(tmp, path); err != nil {
		return "", err
	}
	return path, nil
}

// Keep references in every window's last checkpoint and the live window.
// A one-day grace period also protects freshly pasted, uncheckpointed files.
func collectAttachments(root string, live map[string]bool) {
	paths, _ := filepath.Glob(filepath.Join(root, "session*.json"))
	var stringsIn func(any)
	stringsIn = func(value any) {
		switch v := value.(type) {
		case string:
			live[v] = true
		case []any:
			for _, e := range v {
				stringsIn(e)
			}
		case map[string]any:
			for _, e := range v {
				stringsIn(e)
			}
		}
	}
	for _, path := range paths {
		data, err := readSessionFile(path)
		if err != nil {
			return
		}
		var value any
		if json.Unmarshal(data, &value) != nil {
			return
		}
		stringsIn(value)
	}
	dir := filepath.Join(root, "attachments")
	files, _ := os.ReadDir(dir)
	for _, entry := range files {
		name := entry.Name()
		if len(name) != 68 || !strings.HasSuffix(name, ".png") {
			continue
		}
		if _, err := hexencoding.DecodeString(strings.TrimSuffix(name, ".png")); err != nil {
			continue
		}
		path := filepath.Join(dir, name)
		if live[path] {
			continue
		}
		if info, err := entry.Info(); err == nil && info.Mode().IsRegular() && time.Since(info.ModTime()) > 24*time.Hour {
			_ = os.Remove(path)
		}
	}
}
func (a *App) attachmentReferences() map[string]bool {
	refs := map[string]bool{}
	for id := range a.draftChats {
		if c := a.state.Chats[id]; c != nil {
			for _, path := range c.DraftAttachments {
				refs[path] = true
			}
			for _, d := range c.Queue {
				for _, p := range d.Attachments {
					refs[p] = true
				}
			}
			for _, d := range c.Outbox {
				for _, p := range d.Attachments {
					refs[p] = true
				}
			}
			for _, p := range c.QueueDraft.Attachments {
				refs[p] = true
			}
		}
	}
	for _, v := range a.chats {
		for _, p := range v.Attachments {
			refs[p] = true
		}
	}
	return refs
}
