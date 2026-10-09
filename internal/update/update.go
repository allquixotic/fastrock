// Package update implements a per-user GitHub release updater without a service,
// installer runtime or elevated privileges. All methods are safe off the UI loop.
package update

import (
	"archive/zip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

const Repository = "allquixotic/fastrock"
const maxArchive = int64(192 << 20)
const maxExpanded = uint64(512 << 20)

type Status struct {
	State, Version, Message string
	Downloaded, Total       int64
}
type Asset struct {
	Name   string `json:"name"`
	URL    string `json:"browser_download_url"`
	Digest string `json:"digest"`
	Size   int64  `json:"size"`
}
type Release struct {
	Tag               string `json:"tag_name"`
	Draft, Prerelease bool
	Assets            []Asset `json:"assets"`
}
type Manager struct {
	cancel                                       context.CancelFunc
	done                                         chan struct{}
	mu                                           sync.Mutex
	status                                       Status
	running                                      bool
	pending                                      string
	current, cache, target, endpoint, goos, arch string
	client                                       *http.Client
	changed                                      func(Status)
}

func New(current, cache, executable string, changed func(Status)) *Manager {
	pruneCache(cache)
	return &Manager{current: current, cache: cache, target: installTarget(executable), endpoint: "https://api.github.com/repos/" + Repository + "/releases/latest", goos: runtime.GOOS, arch: runtime.GOARCH, client: &http.Client{Timeout: 15 * time.Minute, CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) > 5 || req.URL.Scheme != "https" {
			return errors.New("unsafe update redirect")
		}
		return nil
	}}, changed: changed, status: Status{State: "idle", Message: "Updates are checked automatically on launch."}}
}
func (m *Manager) Status() Status { m.mu.Lock(); defer m.mu.Unlock(); return m.status }
func (m *Manager) publish(s Status) {
	m.mu.Lock()
	m.status = s
	cb := m.changed
	m.mu.Unlock()
	if cb != nil {
		cb(s)
	}
}
func (m *Manager) Pending() string { m.mu.Lock(); defer m.mu.Unlock(); return m.pending }

// Check coalesces concurrent manual/startup requests. A ready update is retained.
func (m *Manager) Check(ctx context.Context) {
	m.mu.Lock()
	if m.running || m.pending != "" {
		m.mu.Unlock()
		return
	}
	m.running = true
	ctx, cancel := context.WithCancel(ctx)
	m.cancel = cancel
	done := make(chan struct{})
	m.done = done
	m.mu.Unlock()
	go func() {
		defer func() { cancel(); m.mu.Lock(); m.running = false; close(done); m.mu.Unlock() }()
		m.publish(Status{State: "checking", Message: "Checking GitHub releases…"})
		if err := m.check(ctx); err != nil && ctx.Err() == nil {
			m.publish(Status{State: "error", Message: "Update failed: " + err.Error() + ". Use Help → Check for updates to retry."})
		}
	}()
}
func releaseVersion(s string) ([3]int, error) {
	var v [3]int
	parts := strings.Split(strings.TrimPrefix(s, "v"), ".")
	if len(parts) != 3 {
		return v, errors.New("not a stable release version")
	}
	for i, p := range parts {
		n, e := strconv.Atoi(p)
		if e != nil || n < 0 || strconv.Itoa(n) != p {
			return v, errors.New("not a stable release version")
		}
		v[i] = n
	}
	return v, nil
}
func IsRelease(s string) bool { _, e := releaseVersion(s); return e == nil }
func newer(next, current string) bool {
	a, e := releaseVersion(next)
	if e != nil {
		return false
	}
	b, e := releaseVersion(current)
	if e != nil {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return a[i] > b[i]
		}
	}
	return false
}
func (m *Manager) request(ctx context.Context, rawURL string) (*http.Response, error) {
	u, e := url.Parse(rawURL)
	if e != nil {
		return nil, e
	}
	if u.Scheme != "https" && !(u.Scheme == "http" && (u.Hostname() == "127.0.0.1" || u.Hostname() == "localhost")) {
		return nil, errors.New("update URL must use HTTPS")
	}
	req, e := http.NewRequestWithContext(ctx, "GET", rawURL, nil)
	if e != nil {
		return nil, e
	}
	req.Header.Set("User-Agent", "Fastrock/"+m.current)
	req.Header.Set("Accept", "application/vnd.github+json")
	resp, e := m.client.Do(req)
	if e != nil {
		return nil, e
	}
	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		return nil, fmt.Errorf("GitHub returned HTTP %d", resp.StatusCode)
	}
	return resp, nil
}
func (m *Manager) check(ctx context.Context) error {
	if !IsRelease(m.current) {
		m.publish(Status{State: "development", Message: "Development build: automatic installation is disabled. Use a release build for managed updates."})
		return nil
	}
	resp, e := m.request(ctx, m.endpoint)
	if e != nil {
		return e
	}
	var r Release
	e = json.NewDecoder(io.LimitReader(resp.Body, 2<<20)).Decode(&r)
	resp.Body.Close()
	if e != nil {
		return e
	}
	if r.Draft || r.Prerelease || !IsRelease(r.Tag) {
		return errors.New("latest release metadata is not a stable version")
	}
	if !newer(r.Tag, m.current) {
		m.publish(Status{State: "current", Version: m.current, Message: "Fastrock " + m.current + " is up to date."})
		return nil
	}
	name := "fastrock-" + m.goos + "-" + m.arch + ".zip"
	var asset Asset
	for _, a := range r.Assets {
		if a.Name == name {
			asset = a
			break
		}
	}
	if asset.URL == "" {
		return fmt.Errorf("release %s has no %s", r.Tag, name)
	}
	if asset.Size <= 0 || asset.Size > maxArchive {
		return errors.New("invalid update archive size")
	}
	digest := strings.TrimPrefix(asset.Digest, "sha256:")
	if !strings.HasPrefix(asset.Digest, "sha256:") {
		// Older GitHub assets omit digest. The release workflow also publishes SHA256SUMS.
		for _, a := range r.Assets {
			if a.Name == "SHA256SUMS" {
				rr, err := m.request(ctx, a.URL)
				if err != nil {
					return err
				}
				data, err := io.ReadAll(io.LimitReader(rr.Body, 64<<10))
				rr.Body.Close()
				if err != nil {
					return err
				}
				for _, line := range strings.Split(string(data), "\n") {
					f := strings.Fields(line)
					if len(f) == 2 && strings.TrimPrefix(f[1], "*") == name {
						digest = f[0]
					}
				}
			}
		}
	}
	expected, e := hex.DecodeString(digest)
	if e != nil || len(expected) != 32 {
		return errors.New("release is missing a valid SHA-256 digest")
	}
	if e = os.MkdirAll(m.cache, 0700); e != nil {
		return e
	}
	stage, e := os.MkdirTemp(m.cache, "release-")
	if e != nil {
		return e
	}
	keep := false
	defer func() {
		if !keep {
			os.RemoveAll(stage)
		}
	}()
	archive := filepath.Join(stage, "download.zip")
	out, e := os.OpenFile(archive, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if e != nil {
		return e
	}
	resp, e = m.request(ctx, asset.URL)
	if e != nil {
		out.Close()
		return e
	}
	h := sha256.New()
	m.publish(Status{State: "downloading", Version: r.Tag, Message: "Downloading Fastrock " + r.Tag + "…", Total: asset.Size})
	reader := &progressReader{r: io.LimitReader(resp.Body, maxArchive+1), total: asset.Size, update: func(n int64) {
		m.publish(Status{State: "downloading", Version: r.Tag, Message: "Downloading Fastrock " + r.Tag + "…", Downloaded: n, Total: asset.Size})
	}}
	n, e := io.Copy(io.MultiWriter(out, h), reader)
	closeErr := out.Close()
	resp.Body.Close()
	if e != nil {
		return e
	}
	if closeErr != nil {
		return closeErr
	}
	if n != asset.Size || n > maxArchive {
		return errors.New("update download was truncated or exceeds its size")
	}
	if hex.EncodeToString(h.Sum(nil)) != hex.EncodeToString(expected) {
		return errors.New("update checksum mismatch")
	}
	tree := filepath.Join(stage, "payload")
	if e = extract(archive, tree, m.goos); e != nil {
		return e
	}
	os.Remove(archive)
	staged := filepath.Join(tree, "fastrock.exe")
	if m.goos == "darwin" {
		staged = filepath.Join(tree, "Fastrock.app")
	}
	if e = validatePayload(staged, m.goos); e != nil {
		return e
	}
	job := Job{ParentPID: os.Getpid(), Target: m.target, Staged: staged, Stage: stage, Version: r.Tag, GOOS: m.goos}
	file := filepath.Join(stage, "job.json")
	data, _ := json.Marshal(job)
	if e = os.WriteFile(file, data, 0600); e != nil {
		return e
	}
	m.mu.Lock()
	m.pending = file
	m.mu.Unlock()
	keep = true
	m.publish(Status{State: "ready", Version: r.Tag, Message: "Fastrock " + r.Tag + " is ready. Quit all Fastrock windows to install and restart automatically."})
	return nil
}

type progressReader struct {
	r        io.Reader
	n, total int64
	last     time.Time
	update   func(int64)
}

func (p *progressReader) Read(b []byte) (int, error) {
	n, e := p.r.Read(b)
	p.n += int64(n)
	if time.Since(p.last) > 250*time.Millisecond {
		p.update(p.n)
		p.last = time.Now()
	}
	return n, e
}
func extract(archive, dest, goos string) error {
	z, e := zip.OpenReader(archive)
	if e != nil {
		return e
	}
	defer z.Close()
	var size uint64
	if len(z.File) > 4096 {
		return errors.New("update has too many entries")
	}
	for _, f := range z.File {
		name := f.Name
		if strings.ContainsAny(name, "\\:") || strings.HasPrefix(name, "/") || filepath.ToSlash(filepath.Clean(name)) != strings.TrimSuffix(name, "/") || name == "." || strings.HasPrefix(name, "../") {
			return errors.New("unsafe update archive path")
		}
		if goos == "windows" && name != "fastrock.exe" {
			return errors.New("unexpected Windows update content")
		}
		if goos == "darwin" && name != "Fastrock.app/" && !strings.HasPrefix(name, "Fastrock.app/") {
			return errors.New("unexpected macOS update content")
		}
		if f.Mode()&os.ModeSymlink != 0 || (!f.FileInfo().IsDir() && !f.Mode().IsRegular()) {
			return errors.New("unsupported archive entry")
		}
		if f.UncompressedSize64 > maxExpanded-size {
			return errors.New("expanded update exceeds limit")
		}
		size += f.UncompressedSize64
		target := filepath.Join(dest, filepath.FromSlash(name))
		if f.FileInfo().IsDir() {
			if e = os.MkdirAll(target, 0700); e != nil {
				return e
			}
			continue
		}
		if e = os.MkdirAll(filepath.Dir(target), 0700); e != nil {
			return e
		}
		r, e := f.Open()
		if e != nil {
			return e
		}
		mode := os.FileMode(0600)
		if f.Mode()&0111 != 0 || strings.HasSuffix(name, "/Contents/MacOS/fastrock") {
			mode = 0700
		}
		out, e := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, mode)
		if e != nil {
			r.Close()
			return e
		}
		n, e := io.Copy(out, io.LimitReader(r, int64(f.UncompressedSize64)+1))
		ce := out.Close()
		r.Close()
		if e != nil {
			return e
		}
		if ce != nil {
			return ce
		}
		if uint64(n) != f.UncompressedSize64 {
			return errors.New("invalid expanded update size")
		}
	}
	return nil
}
func validatePayload(path, goos string) error {
	exe := path
	if goos == "darwin" {
		exe = filepath.Join(path, "Contents", "MacOS", "fastrock")
		b, e := os.ReadFile(filepath.Join(path, "Contents", "Info.plist"))
		if e != nil || !strings.Contains(string(b), "com.allquixotic.fastrock") {
			return errors.New("invalid app bundle")
		}
	}
	f, e := os.Open(exe)
	if e != nil {
		return e
	}
	defer f.Close()
	b := make([]byte, 4)
	if _, e = io.ReadFull(f, b); e != nil {
		return e
	}
	if goos == "windows" && string(b[:2]) != "MZ" {
		return errors.New("update is not a Windows executable")
	}
	if goos == "darwin" && !(b[0] == 0xcf && b[1] == 0xfa && b[2] == 0xed && b[3] == 0xfe) {
		return errors.New("update is not a Mach-O executable")
	}
	return nil
}

func pruneCache(cache string) {
	entries, _ := os.ReadDir(cache)
	for _, entry := range entries {
		if !entry.IsDir() || !strings.HasPrefix(entry.Name(), "release-") {
			continue
		}
		path := filepath.Join(cache, entry.Name())
		if _, e := os.Stat(filepath.Join(path, ".complete")); e == nil {
			_ = os.RemoveAll(path)
			continue
		}
		// Abandoned downloads are disposable; keep recent failed jobs for diagnosis.
		if info, e := entry.Info(); e == nil && time.Since(info.ModTime()) > 7*24*time.Hour {
			_ = os.RemoveAll(path)
		}
	}
}

// Close cancels an unfinished download and waits for temporary-file cleanup.
// Call only after the last application window and broker request have closed.
func (m *Manager) Close() {
	m.mu.Lock()
	cancel, done := m.cancel, m.done
	m.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if done != nil {
		<-done
	}
}
