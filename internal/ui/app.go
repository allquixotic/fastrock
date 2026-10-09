// Package ui ports the Codex GUI shell to native nucular widgets.
package ui

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"log"
	"os"
	"path/filepath"
	"runtime/debug"
	"strings"
	"sync"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/update"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

type App struct {
	savedViewsRevision               uint64
	notices                          []notice
	noticeKind                       string
	sessionLoading                   bool
	workOnce                         sync.Once
	readJobs, writeJobs, controlJobs chan workTask
	systemLight                      bool
	themeChanges                     chan bool
	scopeChoices                     map[string]*pickerChoices
	timeboxChoices                   map[string]*timeboxChoices
	events                           eventInbox
	recentRows                       []*workspace.Conversation
	recentReady                      bool
	threadSearch                     threadSearchState
	clientBounds                     image.Rectangle
	allowWindowClose                 bool
	windowReturn                     *windowReturn
	warnings                         []string
	skillCache                       skillCache
	updateStatus                     update.Status
	monoFace                         font.Face
	navigationFace                   font.Face
	draftChats                       map[string]bool
	openThreads                      []codex.OpenThread
	openChatIndex                    openChatIndex
	newThreadPending                 bool
	crossSequence                    uint64
	layoutJobs                       chan func()
	rallyGeneration                  int
	scopeGeneration                  int
	scopeCancel                      context.CancelFunc
	scopeWorkspace                   string
	importedSessions                 []string
	transferBuffer                   map[string][]codex.Message
	transferPending                  bool
	incomingTicket                   string
	incomingTab                      string
	readyTicket                      string
	mailbox                          map[string][]mailMessage
	deliveryConsents                 []*deliveryConsent
	replyWaits                       []*replyWait
	drawMax                          time.Duration
	drawCount                        uint64
	infoViews                        map[string]*conversationInfo
	infoCollapsed                    map[string]bool
	detached                         map[string]bool
	recordShortcut                   string
	clipboard                        string
	connection                       Connection
	popping                          map[string]bool
	transfers                        map[string]string
	transferChildren                 map[string]*transferChild
	transferTimers                   map[string]*time.Timer
	transferCancelling               map[string]bool
	sessionName                      string
	lastMaintenance                  time.Time
	lastAttachmentCollection         time.Time
	memoryBytes                      uint64
	fileWatcher                      *fileWatcher

	ctx                                               context.Context
	cancel                                            context.CancelFunc
	store                                             *settings.Store
	prefs                                             settings.Preferences
	window                                            nucular.MasterWindow
	p                                                 palette
	state                                             *workspace.State
	client                                            *codex.Client
	catalog                                           codex.Catalog
	updates                                           chan func()
	status, toast, fatal                              string
	catalogError                                      string
	exitCode                                          int
	chats                                             map[string]*chatView
	rallyViews                                        map[string]*rallyView
	files                                             map[string]*fileView
	sidebarSearch, newFolder                          *nucular.TextEditor
	archived                                          bool
	collapsed                                         map[string]bool
	settingsView                                      *settingsView
	approvals                                         []approval
	approvalDiffs                                     []approvalDiffEntry
	recaps                                            map[string]*recapRun
	recapCreates                                      chan struct{}
	assistant                                         *assistantView
	rallyClient                                       *rally.Client
	rallyUser                                         rally.Object
	rallyUserLoading                                  bool
	rallyUserError                                    string
	iterationScope, iterationError                    string
	workspaces, projects, iterations, releases, users []rally.Object
	rallyErr                                          string
	paletteOpen                                       bool
	paletteSearch                                     *nucular.TextEditor
	writerDone                                        chan struct{}
	persistenceError                                  string
	sessionBlocked                                    bool
	sessions                                          chan session
	preferences                                       chan preferenceWrite
	preferencesQueued, preferencesServer              settings.Preferences
	preferencesRevision                               uint64
	preferencesDone                                   chan struct{}
	lastCheckpoint                                    time.Time
	documentCheckpoints                               map[string]*documentCheckpoint
	chatCheckpoints                                   map[string]*workspace.Conversation
	pendingCheckpoint, publishedCheckpoint            *session
	checkpointTimer                                   *time.Timer
	checkpointObserveTimer                            *time.Timer
	checkpointGeneration                              uint64
	checkpointEpoch                                   uint64
	historyCursor                                     map[bool]string
	historyPages                                      map[bool]*threadPageState
	accountData                                       map[string]any
	accountLoaded, accountLoading                     bool
	accountError                                      string
	accountRequest, accountServer                     uint64
	accountUsage                                      usageState
	timeRefresh                                       *time.Timer
	timeRefreshInterval                               time.Duration
	timeRefreshGeneration                             uint64
	connecting                                        bool
	serverPaused                                      bool
	serverStarting, restartPending                    bool
	serverError, restartNote, startedProvider         string
	serverGeneration                                  uint64
	catalogGeneration                                 uint64
	policyRequest, policyServer                       uint64
	policyLoading                                     bool
	policyError                                       string
	dragTab                                           string
	dragTabX                                          int
	dragTabMoving                                     bool
	tabWidths                                         []int
	visibleTab                                        string
	sidebarCache                                      sidebarCache
}
type approval struct {
	Delivery       *codex.DeliveryRequest
	Content        approvalContent
	CodeEditor     *nucular.TextEditor
	HeightLimit    int
	Details        string
	FormError      string
	OriginThreadID string
	ThreadID       string
	Params         map[string]any
	Submitting     bool
	Choices        []approvalChoice
	Focused        bool
	Selected       int
	Armed          time.Time
	URL            string
	Elicitation    bool
	Message        codex.Message
	Title          string
	Questions      []question
}
type question struct {
	ID, Header, Text string
	Error            string
	Other            bool
	Options          []string
	Descriptions     []string
	Selected         int
	Editor           *nucular.TextEditor
	Secret           bool
	Type             string
	Required         bool
	Form             *elicitationField
}
type fileView struct {
	ReadCancel                 context.CancelFunc
	Closed                     bool
	LoadGeneration             int
	WatchGeneration            int
	WatchCancel                func()
	Stamp                      fileStamp
	Loaded                     bool
	LoadedText                 string
	FileNotice                 string
	Lossy                      bool
	NonUTF8                    bool
	PendingLine, PendingColumn int
	BasePath                   string
	PendingPosition            *editorPosition
	Offset                     int64
	More                       bool
	LimitReached               bool
	Search                     fileSearch
	FocusFind, FocusFile       bool
	Loading                    bool
	Diff                       []diffFile
	DiffGeneration             int
	DiffLayout                 *diffLayout
	DiffCollapsed              []string
	DiffSource, DiffSummary    string
	DiffTruncated              bool
	DiffColumns                int
	DiffScroll                 image.Point
	DiffRestoreScroll          bool
	DiffSelection              diffSelection
	DiffJump                   bool
	DiffJumpFile, DiffJumpRow  int
	Find                       *nucular.TextEditor
	FindOpen, Wrap, Virtual    bool
	Path                       string
	Editor                     *nucular.TextEditor
	Error                      string
}

func Run(ctx context.Context, store *settings.Store, prefs settings.Preferences, connection Connection) int {
	initUIFonts()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	a := &App{ctx: ctx, cancel: cancel, store: store, prefs: prefs, state: workspace.NewState(), updates: make(chan func(), 512), status: "Starting Codex app-server…", chats: map[string]*chatView{}, rallyViews: map[string]*rallyView{}, files: map[string]*fileView{}, collapsed: map[string]bool{}}
	defer a.stopCheckpointTimers()
	defer a.stopTransferTimers()
	defer a.stopSidebarPreparation()
	a.layoutJobs = make(chan func(), 32)
	for range 2 {
		go func() {
			for {
				select {
				case job := <-a.layoutJobs:
					a.safeWork(job)
				case <-ctx.Done():
					return
				}
			}
		}()
	}
	a.connection = connection
	a.toast = connection.Notice
	a.client = connection.Client
	a.popping, a.transfers = map[string]bool{}, map[string]string{}
	a.sessionName = "session.json"
	if connection.Ticket != "" {
		a.sessionName = "session-popout-" + connection.Ticket[:min(16, len(connection.Ticket))] + ".json"
	}
	a.writerDone = make(chan struct{})
	a.sessions = make(chan session, 1)
	a.preferences = make(chan preferenceWrite, 1)
	a.preferencesDone = make(chan struct{})
	a.preferencesQueued, a.preferencesServer = prefs, prefs
	go func() {
		defer close(a.writerDone)
		persistSessions(a.ctx, a.sessions, func(snapshot session) error {
			return writeSessionFile(a.store, a.sessionFile(), snapshot)
		}, func(err error) {
			a.post(func() {
				if err != nil {
					a.persistenceError = "Drafts are not saved: " + err.Error()
				} else {
					a.persistenceError = ""
				}
			})
		})
	}()
	go a.persistPreferences()

	if connection.Ticket == "" {
		a.sessionLoading = true
	}
	a.p = colors(prefs.Theme == "light")
	a.monoFace = typeFace(prefs.FontSize-1, monoFont)
	a.navigationFace = typeFace(prefs.FontSize+3, regularFont)
	a.sidebarSearch = textEditor("", false)
	a.sidebarSearch.Placeholder = "Search conversations"
	a.newFolder = textEditor(prefs.WorkingDirectory, false)
	a.paletteSearch = textEditor("", false)
	if len(a.state.Tabs) == 0 && connection.Ticket == "" {
		a.state.Open(workspace.New, "New tab", "", "")
	}
	a.window = nucular.NewMasterWindowSize(nucular.WindowNoScrollbar, "Fastrock", platform.WindowSize(), a.draw)
	if clipboard, ok := a.window.(interface{ OnClipboardError(func(error)) }); ok {
		clipboard.OnClipboardError(a.report)
	}
	a.window.SetStyle(makeStyle(a.p, prefs.FontSize))
	a.themeChanges = make(chan bool, 1)
	go a.observeTheme()
	a.applyTheme()
	if guard, ok := a.window.(interface{ OnCloseRequested(func() bool) }); ok {
		guard.OnCloseRequested(a.canCloseWindow)
	}

	a.work(func() {
		if connection.Ticket == "" {
			loaded := readSessions(a.store.Dir)
			a.post(func() { a.applySessions(loaded) })
		}
		client := connection.Client
		var currentPreferences codex.PreferencesState
		if err := client.Call(ctx, "fastrock/preferences", map[string]any{"data": settings.Patch{}}, &currentPreferences); err == nil {
			a.post(func() { a.applyPreferences(currentPreferences, nil) })
		}
		if connection.Ticket != "" {
			var payload tabTransfer
			e := client.Call(ctx, "fastrock/claim", map[string]string{"ticket": connection.Ticket}, &payload)
			if e != nil {
				a.post(func() { a.fatal = e.Error(); a.exitCode = 1; a.closeAbandonedPopout() })
			} else {
				a.post(func() {
					a.incomingTicket = connection.Ticket
					a.transferPending = true
					if err := a.installTransfer(payload); err != nil {
						a.rpc("fastrock/cancel", map[string]string{"ticket": connection.Ticket}, nil)
						a.cancelIncoming()
						a.report(err)
						return
					}
					if payload.Chat != nil {
						a.transferBuffer = map[string][]codex.Message{payload.Chat.ID: nil}
					}
					a.readyTicket = connection.Ticket
				})
			}
		} else {
			a.post(a.restoreDocuments)
		}
		a.post(func() { a.status = client.Version + " · Connected" })
		a.work(func() {
			var status update.Status
			if client.Call(ctx, "fastrock/update", map[string]bool{}, &status) == nil {
				a.post(func() { a.updateStatus = status })
			}
		})
		go a.consume(client)
		a.post(a.refreshCatalog)
		a.post(func() { a.requestThreads(false, "") })
	})
	a.connectRally()
	a.work(func() {
		name := "Fastrock"
		if connection.Ticket != "" {
			name = "Pop-out"
		}
		_ = connection.Client.Notify("fastrock/name", map[string]string{"name": name})
	})
	a.startAutomation()
	a.window.Main()
	a.clearReplyWaits()
	cancel()
	<-a.writerDone
	<-a.preferencesDone
	if err := a.saveSession(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		platform.ShowError(err.Error())
		a.exitCode = 1
	}
	if a.client != nil {
		ctx, done := context.WithTimeout(context.Background(), 15*time.Second)
		var state codex.PreferencesState
		err := a.client.Call(ctx, "fastrock/preferences", map[string]any{"data": settings.Diff(a.preferencesServer, a.prefs)}, &state)
		done()
		if err != nil {
			fmt.Fprintln(os.Stderr, "Preferences not saved:", err)
			a.exitCode = 1
		}
	}
	if a.client != nil {
		a.client.Close()
	}
	return a.exitCode
}
func (a *App) post(f func()) {
	if a.updates == nil {
		return
	}
	var done <-chan struct{}
	if a.ctx != nil {
		done = a.ctx.Done()
	}
	select {
	case a.updates <- f:
		if a.window != nil {
			a.window.Changed()
		}
	case <-done:
	}
}
func (a *App) safeWork(f func()) {
	defer func() {
		if err := recover(); err != nil {
			log.Printf("Fastrock worker panic: %v\n%s", err, debug.Stack())
			a.post(func() { a.toast = fmt.Sprintf("A background operation failed: %v. Retry the operation.", err) })
		}
	}()
	f()
}
func (a *App) report(e error) {
	if e != nil {
		a.toast = e.Error()
		a.noticeKind = "error"
	}
}
func (a *App) rpc(method string, params any, done func(json.RawMessage)) {
	a.rpcResult(method, params, done, nil)
}
func (a *App) rpcResult(method string, params any, done func(json.RawMessage), failed func(error)) {
	a.rpcReporting(method, params, done, failed, true)
}
func (a *App) rpcInline(method string, params any, done func(json.RawMessage), failed func(error)) {
	a.rpcReporting(method, params, done, failed, false)
}
func (a *App) rpcReporting(method string, params any, done func(json.RawMessage), failed func(error), report bool) {
	c := a.client
	if c == nil {
		err := errors.New("Codex is still connecting")
		if report {
			a.report(err)
		}
		if failed != nil {
			failed(err)
		}
		return
	}
	dispatch := a.writeWork
	if strings.HasSuffix(method, "/read") || strings.HasSuffix(method, "/get") || strings.HasSuffix(method, "/list") || strings.HasSuffix(method, "/search") {
		dispatch = a.work
	}
	if method == "turn/interrupt" || strings.HasPrefix(method, "fastrock/") {
		dispatch = a.controlWork
	}
	dispatch(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		e := c.Call(ctx, method, params, &raw)
		a.post(func() {
			if e != nil {
				if report {
					a.report(e)
				}
				if failed != nil {
					failed(e)
				}
				return
			}
			if done != nil {
				done(raw)
			}
		})
	}, func() {
		if failed != nil {
			failed(errWorkQueueFull)
		}
	})
}
func (a *App) refreshCatalog() {
	c, cwd := a.client, a.prefs.WorkingDirectory
	if c == nil {
		return
	}
	a.skillCache.invalidate()
	a.catalogGeneration++
	generation := a.catalogGeneration
	policyRequest := a.policyRequest
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		catalog, e := c.Catalog(ctx, cwd)
		a.post(func() {
			if a.client != c || a.catalogGeneration != generation {
				return
			}
			if e != nil {
				a.toast = "Could not refresh Codex catalog: " + e.Error()
				a.catalogError = a.toast
				return
			}
			if a.policyRequest != policyRequest {
				catalog.Policy, catalog.PolicyLoaded = a.catalog.Policy, a.catalog.PolicyLoaded
				if catalog.Config == nil {
					catalog.Config = map[string]any{}
				}
				if forced, ok := a.catalog.Config["forced_login_method"]; ok {
					catalog.Config["forced_login_method"] = forced
				} else {
					delete(catalog.Config, "forced_login_method")
				}
			}
			a.catalog = catalog
			a.catalogError = ""
			a.observeProvider(str(catalog.Config, "model_provider"))
			a.refreshAccount()
			if s := a.settingsView; s != nil && a.state != nil {
				if tab := a.state.Current(); tab != nil && tab.Kind == workspace.Settings {
					a.loadSettingsPage(s.Page)
				}
			}
		})
	})
}
func (a *App) savePrefs() {
	p := a.prefs
	p.Views = append([]settings.SavedView(nil), p.Views...)
	p.Keymap = cloneKeys(p.Keymap)
	p.RecentFolders = append([]string(nil), p.RecentFolders...)
	write := preferenceWrite{Patch: settings.Diff(a.preferencesQueued, p), Snapshot: p, Client: a.client}
	a.preferencesQueued = p
	if len(write.Patch) == 0 {
		return
	}
	select {
	case a.preferences <- write:
	default:
		select {
		case previous := <-a.preferences:
			for k, v := range write.Patch {
				previous.Patch[k] = v
			}
			write.Patch = previous.Patch
		default:
		}
		select {
		case a.preferences <- write:
		default:
		}
	}

}
func (a *App) theme() {
	a.applyTheme()
	a.savePrefs()
}
func (a *App) applyTheme() {
	a.monoFace = typeFace(a.prefs.FontSize-1, monoFont)
	a.navigationFace = typeFace(a.prefs.FontSize+3, regularFont)
	a.p = colors(a.prefs.Theme == "light" || a.prefs.Theme == "system" && a.systemLight)
	if a.themeChanges != nil {
		select {
		case <-a.themeChanges:
		default:
		}
		a.themeChanges <- a.prefs.Theme == "system"
	}
	a.window.SetStyle(makeStyle(a.p, a.prefs.FontSize))
}
func (a *App) openSettings() {
	a.state.Open(workspace.Settings, "Settings", "", "")
	if a.settingsView == nil {
		a.settingsView = newSettingsView(a.prefs)
		a.loadSettingsPage(a.settingsView.Page)
	}
}
func (a *App) openRally(page string) {
	spec := rally.FindPage(page)
	id := a.state.Open(workspace.Rally, spec.Title, "", page)
	if a.rallyViews[id] == nil {
		v := newRallyView(spec)
		v.Display = settings.DisplayOrDefault(a.prefs.RallyDisplay)
		a.rallyViews[id] = v
		a.refreshRally(v)
	}
}
func (a *App) openFile(path string) {
	if !filepath.IsAbs(path) {
		path = filepath.Join(a.prefs.WorkingDirectory, path)
	}
	id := a.state.Open(workspace.File, filepath.Base(path), path, "")
	if a.files[id] != nil {
		return
	}
	v := &fileView{Path: path, Find: textEditor("", false), Wrap: true}
	a.files[id] = v
	a.loadFilePage(v, false)
}
func (a *App) connectRally() {
	a.rallyClient = nil
	a.rallyUser, a.rallyUserLoading, a.rallyUserError = nil, false, ""
	a.iterationScope, a.iterationError = "", ""
	a.rallyErr = "Connecting to Rally…"
	if a.scopeCancel != nil {
		a.scopeCancel()
	}
	for _, view := range a.rallyViews {
		view.FilterDraft.closePicker()
		if view.cancel != nil {
			view.cancel()
		}
		view.Generation++
		view.Loading = false
	}
	a.rallyGeneration++
	generation := a.rallyGeneration
	p := a.prefs
	a.work(func() {
		token, e := a.store.Token(p.RallyEndpoint)
		if e != nil || token == "" {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = "Connect your Rally endpoint and API token in Settings."
				if e != nil {
					a.rallyErr = "Could not read the Rally credential: " + e.Error()
				}
			})
			return
		}
		c, e := rally.New(p.RallyEndpoint, token, nil)
		if e != nil {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = rallyErrorMessage(e)
			})
			return
		}
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		workspaces, e := scopeObjects(ctx, c, "Workspace", rally.Query{})
		choices := makePickerChoices(workspaces)
		if e != nil {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = rallyErrorMessage(e)
			})
			return
		}
		a.post(func() {
			if a.rallyGeneration != generation {
				return
			}
			a.rallyClient = c
			a.scopeWorkspace = ""
			a.rallyErr = ""
			a.workspaces = workspaces
			a.scopeChoices = map[string]*pickerChoices{"Workspace": choices}
			a.timeboxChoices = nil
			if a.prefs.RallyWorkspace == "" && len(workspaces) > 0 {
				a.prefs.RallyWorkspace = workspaces[0].String("_ref")
			}
			a.loadScope()
			a.loadRallyUser()
			a.reloadRally()
		})
	}, func() { a.rallyErr = errWorkQueueFull.Error() })
}
func (a *App) loadScope() {
	c := a.rallyClient
	if c == nil {
		return
	}
	p := a.prefs
	a.iterationScope, a.iterationError = "", ""
	if a.scopeCancel != nil {
		a.scopeCancel()
	}
	ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
	a.scopeCancel = cancel
	reloadWorkspace := a.scopeWorkspace != p.RallyWorkspace
	a.scopeWorkspace = p.RallyWorkspace
	a.scopeGeneration++
	generation := a.scopeGeneration
	for _, v := range a.rallyViews {
		if v.CurrentIteration {
			a.refreshRallyItems(v)
		}
	}
	a.work(func() {
		defer cancel()
		q := rally.Query{Workspace: p.RallyWorkspace, Project: p.RallyProject, Children: p.ProjectChildren, Parents: p.ProjectParents}
		type result struct {
			kind      string
			items     []rally.Object
			err       error
			choices   *pickerChoices
			timeboxes *timeboxChoices
		}
		results := make(chan result, 4)
		kinds := []string{"Iteration", "Release"}
		if reloadWorkspace {
			kinds = append(kinds, "Project", "User")
		}
		for _, kind := range kinds {
			go func() {
				query := q
				if kind == "Project" || kind == "User" {
					query = rally.Query{Workspace: p.RallyWorkspace}
				}
				items, err := scopeObjects(ctx, c, kind, query)
				r := result{kind: kind, items: items, err: err, choices: makePickerChoices(items)}
				if kind == "Iteration" || kind == "Release" {
					r.timeboxes = makeTimeboxChoices(items, p.RallyProject, time.Now())
				}
				results <- r
			}()
		}
		loaded := make([]result, 0, len(kinds))
		for range kinds {
			loaded = append(loaded, <-results)
		}
		a.post(func() {
			if a.scopeGeneration != generation || a.rallyClient != c {
				return
			}
			for _, r := range loaded {
				if r.err != nil {
					if r.kind == "Iteration" {
						a.iterationError = r.err.Error()
					}
					a.report(fmt.Errorf("%s choices: %w", r.kind, r.err))
					if r.kind == "Project" || r.kind == "User" {
						a.scopeWorkspace = ""
					}
					continue
				}
				if a.scopeChoices == nil {
					a.scopeChoices = map[string]*pickerChoices{}
				}
				a.scopeChoices[r.kind] = r.choices
				if r.timeboxes != nil {
					if a.timeboxChoices == nil {
						a.timeboxChoices = map[string]*timeboxChoices{}
					}
					a.timeboxChoices[r.kind] = r.timeboxes
				}
				switch r.kind {
				case "Project":
					a.projects = r.items
				case "Iteration":
					a.iterations = r.items
					a.iterationScope = a.rallyPresetScope()
				case "Release":
					a.releases = r.items
				case "User":
					a.users = r.items
					delete(a.scopeChoices, "OwnerFilter")
				}
			}
			delete(a.scopeChoices, "Timebox")
			for _, v := range a.rallyViews {
				if v.CurrentIteration {
					a.refreshRallyItems(v)
				}
			}
		})
	}, func() {
		cancel()
		if a.scopeGeneration == generation && a.rallyClient == c {
			a.iterationError = errWorkQueueFull.Error()
			for _, v := range a.rallyViews {
				if v.CurrentIteration {
					a.refreshRallyItems(v)
				}
			}
		}
	})
}
func (a *App) draw(w *nucular.Window) {
	defer a.scheduleTimeUpdate()
	defer a.drawNotices(w)
	a.clientBounds = image.Rect(w.Bounds.X, w.Bounds.Y, w.Bounds.X+w.Bounds.W, w.Bounds.Y+w.Bounds.H)
	started := time.Now()
	defer func() {
		elapsed := time.Since(started)
		a.drawCount++
		if elapsed > a.drawMax {
			a.drawMax = elapsed
		}
	}()
	a.advanceThreadRefreshes()
	deadline := time.Now().Add(4 * time.Millisecond)
	for i := 0; i < 64 && time.Now().Before(deadline); i++ {
		select {
		case f := <-a.updates:
			f()
		default:
			i = 64
		}
	}
	for i := 0; i < 64 && time.Now().Before(deadline); i++ {
		e := a.events.take()
		if e == nil {
			break
		}
		if e.disconnected {
			a.disconnected(e.client)
		} else if a.client == e.client {
			if len(e.message.ID) > 0 && e.message.Method == "item/tool/call" {
				a.runDynamicTool(e.client, e.message)
			} else {
				a.eventDecoded(e.message, e.params)
			}
		}
	}
	a.events.mu.Lock()
	moreEvents := len(a.events.events) > 0
	a.events.mu.Unlock()
	if len(a.updates) > 0 || moreEvents {
		a.window.Changed()
	}
	if a.clipboard != "" {
		w.SetClipboard(a.clipboard)
		a.clipboard = ""
	}
	a.handleNoticeInput(w)
	a.shortcuts(w)
	a.checkpoint()
	a.maintain()
	if a.fatal != "" {
		w.Row(100).Dynamic(1)
		w.Spacing(1)
		title(w, "Fastrock cannot start", a.p)
		w.Row(140).Dynamic(1)
		w.LabelWrap(a.fatal)
		w.Row(30).Static(160)
		if w.ButtonText("Exit") {
			a.window.Close()
		}
		return
	}
	if a.sessionLoading {
		title(w, "Restoring workspace…", a.p)
		return
	}
	a.drawMenu(w)
	if current := a.state.Current(); current != nil && current.Kind == workspace.Chat {
		if c := a.state.Chats[current.Target]; c != nil {
			c.Unread = false
		}
	}
	a.drawTabs(w)
	if a.persistenceError != "" {
		w.Row(42).Dynamic(1)
		w.LabelWrap(a.persistenceError)
	}
	if len(a.warnings) > 0 {
		w.Row(60).Ratio(.9, .1)
		w.LabelWrap(strings.Join(a.warnings, "\n"))
		if w.ButtonText("Dismiss") {
			a.warnings = nil
		}
	}
	a.collectNotice()
	footerHeight := 0
	if a.prefs.StatusBar {
		footerHeight = 25
	}
	h := max(240, w.LayoutAvailableHeight()-footerHeight)
	side := 0
	if a.prefs.Sidebar {
		side = 255
	}
	info := 0
	if a.prefs.Info && a.state.Current() != nil && a.state.Current().Kind == workspace.Chat {
		info = 255
	}
	available := w.LayoutAvailableWidth()
	if available-side-info < 360 && info > 0 {
		info = 0
	}
	if available-side-info < 360 && side > 0 {
		side = 0
	}
	if side == 0 && a.prefs.Sidebar || info == 0 && a.prefs.Info {
		w.Row(28).Static(120, 120)
		if w.ButtonText("Conversations") {
			a.window.PopupOpen("Conversations", nucular.WindowTitle|nucular.WindowClosable, a.modalBounds(340, 600), false, a.drawSidebar)
		}
		if w.ButtonText("Information") {
			a.window.PopupOpen("Information", nucular.WindowTitle|nucular.WindowClosable, a.modalBounds(340, 600), false, a.drawInfo)
		}
		h = max(120, w.LayoutAvailableHeight()-footerHeight)
	}
	var widths []int
	if side > 0 {
		widths = append(widths, side)
	}
	widths = append(widths, max(100, available-side-info))
	if info > 0 {
		widths = append(widths, info)
	}
	w.Row(h).Static(widths...)
	if side > 0 {
		if sw := w.GroupBegin("conversation-sidebar", nucular.WindowNoHScrollbar); sw != nil {
			a.drawSidebar(sw)
			sw.GroupEnd()
		}
	}
	flags := nucular.WindowNoScrollbar
	if t := a.state.Current(); t == nil || t.Kind == workspace.New {
		flags = nucular.WindowNoHScrollbar
	}
	if body := w.GroupBegin("document", flags); body != nil {
		t := a.state.Current()
		if a.windowReturn != nil || a.transferPending || t != nil && a.popping[t.ID] {
			muted(body, "Moving this tab to its new window…", a.p)
		} else if t == nil {
			a.drawNew(body)
		} else {
			switch t.Kind {
			case workspace.New:
				a.drawNew(body)
			case workspace.Chat:
				if a.chats[t.Target] == nil && a.client != nil && !a.serverPaused {
					a.resumeThread(t.Target)
				}
				a.drawChat(body, t.Target)
			case workspace.Rally:
				a.drawRally(body, a.rallyViews[t.ID])
			case workspace.Settings:
				a.drawSettings(body)
			case workspace.File:
				if f := a.files[t.ID]; f != nil && f.Editor == nil && (f.Loaded || f.Error == "") {
					a.ensureFileEditor(f)
				}
				a.drawFile(body, a.files[t.ID])
			}
		}
		body.GroupEnd()
	}
	if info > 0 {
		if iw := w.GroupBegin("conversation-info", nucular.WindowNoHScrollbar); iw != nil {
			a.drawInfo(iw)
			iw.GroupEnd()
		}
	}
	if a.prefs.StatusBar {
		w.Row(25).Static(max(100, w.LayoutAvailableWidth()-28), 28)
		w.LabelColored(a.status, "LC", a.p.Muted)
		if iconButton(w, "close", false, a.p) {
			a.prefs.StatusBar = false
			a.savePrefs()
		}
	}
	if a.paletteOpen {
		a.drawPalette()
	}
	if ticket := a.readyTicket; ticket != "" {
		a.readyTicket = ""
		a.rpc("fastrock/ready", map[string]string{"ticket": ticket}, nil)
	}
}
func (a *App) drawTabs(w *nucular.Window) {
	settingsWidth := nucular.FontWidth(w.Master().Style().Font, "Settings") + 20
	w.Row(38).Static(36, max(100, w.LayoutAvailableWidth()-126-settingsWidth), 30, 30, 30, settingsWidth)
	if iconButton(w, "sidebar", a.prefs.Sidebar, a.p) {
		a.prefs.Sidebar = !a.prefs.Sidebar
		a.savePrefs()
	}
	oldGroup := w.Master().Style().GroupWindow
	w.Master().Style().GroupWindow.Padding = image.Pt(0, 2)
	w.Master().Style().GroupWindow.Spacing = image.Pt(0, 0)
	if strip := w.GroupBegin("tabs", nucular.WindowNoScrollbar); strip != nil {
		count := len(a.state.Tabs)
		width := min(240, max(84, strip.LayoutAvailableWidth()/max(1, count)))
		if cap(a.tabWidths) < count {
			a.tabWidths = make([]int, count)
		} else {
			a.tabWidths = a.tabWidths[:count]
		}
		for i := range a.tabWidths {
			a.tabWidths[i] = width
		}
		maxScroll := max(0, count*width-strip.LayoutAvailableWidth())
		if in := strip.Input(); in.Mouse.HoveringRect(strip.Bounds) && (in.Mouse.ScrollDelta != 0 || in.Mouse.ScrollDeltaX != 0) {
			strip.Scrollbar.X = min(maxScroll, max(0, strip.Scrollbar.X+int(in.Mouse.ScrollDeltaX-in.Mouse.ScrollDelta)*60))
		}
		if a.visibleTab != a.state.Active {
			for i, t := range a.state.Tabs {
				if t.ID == a.state.Active {
					left, right := i*width, (i+1)*width
					if left < strip.Scrollbar.X {
						strip.Scrollbar.X = left
					}
					if right > strip.Scrollbar.X+strip.LayoutAvailableWidth() {
						strip.Scrollbar.X = right - strip.LayoutAvailableWidth()
					}
				}
			}
			a.visibleTab = a.state.Active
		}
		strip.Scrollbar.X = min(maxScroll, max(0, strip.Scrollbar.X))
		strip.Row(34).Static(a.tabWidths...)
		closeID, keepID, moveID := "", "", ""
		keepRight, moveBy := -1, 0
		in := strip.Input()
		if a.dragTab != "" && in.Mouse.Down(mouse.ButtonLeft) && absInt(in.Mouse.Pos.X-a.dragTabX) > 6 {
			a.dragTabMoving = true
		}
		wasDragging := a.dragTabMoving
		var firstTab rect.Rect
		stride, dragFrom := 0, -1
		for i, t := range a.state.Tabs {
			dot := a.tabDot(t)
			activate, close, b := documentTab(strip, a.tabTitle(t), t.ID == a.state.Active, a.dragTabMoving && a.dragTab == t.ID, dot, a.p)
			if i == 0 {
				firstTab = b
			} else if i == 1 {
				stride = b.X - firstTab.X
			}
			if a.dragTab == t.ID {
				dragFrom = i
			}
			in = strip.Input()
			if in.Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) && !in.Mouse.IsClickDownInRect(mouse.ButtonLeft, tabLayout(b).Close, true) {
				a.dragTab = t.ID
				a.dragTabX = in.Mouse.Pos.X
			}
			if b.W > 0 && b.H > 0 {
				if menu := strip.ContextualOpen(0, image.Pt(220, 490), b, nil); menu != nil {
					menu.Row(28).Dynamic(1)
					if menu.MenuItem(label.T("Pop out into new window")) {
						a.popOut(t)
					}
					if menu.MenuItem(label.T("Move to another window…")) {
						a.moveTab(t)
					}
					if t.Kind == workspace.Chat {
						a.chatTabMenu(menu, t)
					}
					if menu.MenuItem(label.T("Close tab")) {
						closeID = t.ID
					}
					if menu.MenuItem(label.T("Close other tabs")) {
						keepID = t.ID
					}
					if menu.MenuItem(label.T("Close tabs to right")) {
						keepRight = i
					}
				}
			}
			if activate && !a.dragTabMoving && !wasDragging {
				a.state.Active = t.ID
			}
			if close && !a.dragTabMoving {
				closeID = t.ID
			}
		}
		in = strip.Input()
		if a.dragTabMoving && dragFrom >= 0 && in.Mouse.HoveringRect(strip.Bounds) {
			to, marker := tabDropTarget(in.Mouse.Pos.X, firstTab.X, stride, count, dragFrom)
			if to != dragFrom {
				gap := max(0, stride-firstTab.W)
				marker = min(strip.Bounds.X+strip.Bounds.W-2, max(strip.Bounds.X, marker-gap/2-1))
				strip.Commands().FillRect(rect.Rect{X: marker, Y: firstTab.Y + 2, W: 2, H: max(0, firstTab.H-4)}, 0, a.p.Accent)
			}
			if in.Mouse.Released(mouse.ButtonLeft) {
				moveID, moveBy = a.dragTab, to-dragFrom
			}
		}
		if moveID != "" {
			a.state.Move(moveID, moveBy)
			a.state.Active = moveID
		}
		var closing []string
		if keepID != "" {
			for _, t := range a.state.Tabs {
				if t.ID != keepID {
					closing = append(closing, t.ID)
				}
			}
			a.state.Active = keepID
		} else if keepRight >= 0 {
			for _, t := range a.state.Tabs[keepRight+1:] {
				closing = append(closing, t.ID)
			}
		}
		if len(closing) > 0 {
			a.closeTabs(closing)
		}

		if closeID != "" {
			a.closeTab(closeID)
		}
		if strip.Input().Mouse.Released(mouse.ButtonLeft) {
			a.dragTab = ""
			a.dragTabMoving = false
		}
		strip.GroupEnd()
	}
	w.Master().Style().GroupWindow = oldGroup
	if menu := w.Menu(label.T("▾"), 300, nil); menu != nil {
		if a.drawTabOverflow(menu) {
			menu.Close()
		}
	}
	if iconButton(w, "plus", false, a.p) {
		a.state.OpenNew()
	}
	if iconButton(w, "info", a.prefs.Info, a.p) {
		a.prefs.Info = !a.prefs.Info
		a.savePrefs()
	}
	if w.ButtonText("Settings") {
		a.openSettings()
	}
}
func absInt(n int) int {
	if n < 0 {
		return -n
	}
	return n
}
func (a *App) drawNew(w *nucular.Window) {
	scale := w.Master().Style().Scaling
	width := min(int(600*scale), max(1, w.LayoutAvailableWidth()-int(48*scale)))
	height := max(1, w.LayoutAvailableHeight())
	w.RowScaled(height).SpaceBegin(1)
	w.LayoutSpacePushScaled(rect.Rect{X: (w.LayoutAvailableWidth() - width) / 2, W: width, H: height})
	if body := w.GroupBegin("start-page", nucular.WindowNoHScrollbar); body != nil {
		a.drawNewContents(body)
		body.GroupEnd()
	}
}

func (a *App) drawNewContents(w *nucular.Window) {
	scale := w.Master().Style().Scaling
	w.RowScaled(max(int(24*scale), (w.LayoutAvailableHeight()-int(560*scale))/3)).Dynamic(1)
	w.Spacing(1)
	title(w, "Start a new thread", a.p)
	caption := "Choose a folder or start a conversation without a project folder."
	face := w.Master().Style().Font
	w.RowScaled(len(nucular.WrapText(face, caption, max(1, w.LayoutAvailableWidth())))*(nucular.FontHeight(face)+2) + 8).Dynamic(1)
	previous := w.Master().Style().Text.Color
	w.Master().Style().Text.Color = a.p.Muted
	w.LabelWrap(caption)
	w.Master().Style().Text.Color = previous
	columns := 4
	if w.LayoutAvailableWidth() < int(500*scale) {
		columns = 2
	}
	w.Row(30).Dynamic(columns)
	if w.ButtonText("Choose folder…") {
		a.choosePath(false, true, func(path string) { setText(a.newFolder, path); a.prefs.WorkingDirectory = path; a.savePrefs() })
	}
	if w.ButtonText("Folderless chat") {
		a.newThread("")
	}
	if w.ButtonText("Open file…") {
		a.choosePath(false, false, a.openFile)
	}
	if w.ButtonText("Resume chat…") {
		a.chooseConversation(func(c *workspace.Conversation) { a.resumeThread(c.ID) })
	}
	title(w, "Project folder", a.p)
	w.Row(32).Dynamic(1)
	a.newFolder.Edit(w)
	w.Row(32).Dynamic(1)
	if primary(w, "New conversation", a.p) {
		a.newThread(text(a.newFolder))
	}
	title(w, "Rally workspace", a.p)
	w.Row(34).Dynamic(columns)
	for _, id := range []string{"teamboard", "backlog", "portfolioitemstreegrid", "reports"} {
		p := rally.FindPage(id)
		if w.ButtonText(p.Title) {
			a.openRally(id)
		}
	}
	if a.rallyErr != "" {
		muted(w, a.rallyErr, a.p)
		w.Row(30).Static(180)
		if w.ButtonText("Configure Rally") {
			a.openSettings()
		}
	}
	if a.accountLoaded && accountNeedsLogin(a.accountData) {
		w.Row(30).Static(180)
		if w.ButtonText("Sign in to Codex…") {
			a.settingsPage("Account")
		}
	}
	title(w, "Recent folders", a.p)
	for _, folder := range a.prefs.RecentFolders {
		w.Row(30).Dynamic(1)
		if w.ButtonText(folder) {
			setText(a.newFolder, folder)
			a.newThread(folder)
		}
		if menu := w.ContextualOpen(0, image.Pt(210, 120), w.LastWidgetBounds, nil); menu != nil {
			if menu.MenuItem(label.T("Open folder")) {
				a.openPath(folder, false)
			}
			if menu.MenuItem(label.T("Copy path")) {
				a.copyText(folder)
			}
			if menu.MenuItem(label.T("Forget folder")) {
				a.forgetFolder(folder)
			}
		}
	}
	title(w, "Recent conversations", a.p)
	rows := a.recentConversations()
	for _, c := range rows[:min(8, len(rows))] {
		w.Row(32).Dynamic(1)
		if w.ButtonText(c.Title + "  ·  " + filepath.Base(c.Cwd)) {
			a.resumeThread(c.ID)
		}
	}
}
