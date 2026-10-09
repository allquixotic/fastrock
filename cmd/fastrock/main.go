package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"runtime/debug"
	"time"

	"github.com/allquixotic/fastrock/internal/buildinfo"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/ui"
	"github.com/allquixotic/fastrock/internal/update"
)

func main() { os.Exit(run()) }
func run() int {
	doctor := flag.Bool("doctor", false, "Check installed Codex and its protocol without opening a window")
	version := flag.Bool("version", false, "Print Fastrock version")
	popout := flag.String("popout", "", "Internal window transfer ticket")
	applyUpdate := flag.String("apply-update", "", "Internal deferred update job")
	flag.Parse()
	if *applyUpdate != "" {
		if e := update.Apply(*applyUpdate); e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		return 0
	}
	if *popout != "" {
		if decoded, err := hex.DecodeString(*popout); err != nil || len(decoded) != 32 {
			platform.ShowError("Invalid window transfer ticket")
			return 1
		}
	}
	if *version {
		fmt.Println("Fastrock " + buildinfo.Version)
		return 0
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	if *doctor {
		ctx, cancel := context.WithTimeout(ctx, 45*time.Second)
		defer cancel()
		c, e := codex.Start(ctx)
		if e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		defer c.Close()
		cwd, _ := os.Getwd()
		catalog, e := c.Catalog(ctx, cwd)
		if e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		result := map[string]any{"codex": c.Version, "model": catalog.DefaultModel(), "models": catalog.Models, "protocol": "compatible"}
		b, _ := json.MarshalIndent(result, "", "  ")
		fmt.Println(string(b))
		return 0
	}
	startupNotice := ""
	if *popout == "" && update.IsRelease(buildinfo.Version) {
		exe, err := os.Executable()
		if err == nil {
			relaunched, installErr := update.PrepareInstallation(exe)
			if installErr != nil {
				startupNotice = "Per-user installation: " + installErr.Error()
			}
			if relaunched && installErr == nil {
				return 0
			}
		}
	}
	store, e := settings.Open()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	if *popout == "" {
		release, err := platform.AcquireInstance(store.Dir)
		if err != nil {
			platform.ShowError(err.Error())
			return 1
		}
		defer release()
	}
	if data, err := os.ReadFile(filepath.Join(store.Dir, "updates", "last-update.json")); err == nil {
		var last update.Status
		if json.Unmarshal(data, &last) == nil && last.State == "error" {
			startupNotice = last.Message
			_ = os.Remove(filepath.Join(store.Dir, "updates", "last-update.json"))
		}
	}
	prefs, e := store.Load()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	address, token := os.Getenv("FASTROCK_BROKER"), os.Getenv("FASTROCK_BROKER_TOKEN")
	var broker *codex.Broker
	var updater *update.Manager
	connection := ui.Connection{Address: address, Token: token, Ticket: *popout, Notice: startupNotice}
	// Run invokes this off-thread after FLTK confirms the first displayed frame.
	// It also joins pending startup before returning, so service ownership below
	// stays synchronized even if the window closes during CLI initialization.
	connection.Connect = func(windowCtx context.Context) (ui.Connection, error) {
		if *popout == "" {
			if broker != nil {
				select {
				case <-broker.Done():
					updater.Close()
					broker, updater = nil, nil
				default:
				}
			}
			if broker == nil {
				backend, err := codex.StartWithStartup(ctx, windowCtx)
				if err != nil {
					return ui.Connection{}, err
				}
				owned, err := codex.NewBroker(backend)
				if err != nil {
					backend.Close()
					return ui.Connection{}, err
				}
				owned.SetStarter(func() (*codex.Client, error) { return codex.Start(ctx) })
				owned.SetPreferences(prefs, store.Save)
				exe, _ := os.Executable()
				manager := update.New(buildinfo.Version, filepath.Join(store.Dir, "updates"), exe, func(s update.Status) { owned.UpdateStatus(s) })
				owned.SetUpdateHandler(func(check bool) any {
					if check {
						manager.Check(ctx)
					}
					return manager.Status()
				})
				client, err := codex.DialWithStartup(ctx, windowCtx, owned.Address(), owned.Token())
				if err != nil {
					owned.Close()
					manager.Close()
					return ui.Connection{}, err
				}
				broker, updater = owned, manager
				address, token = owned.Address(), owned.Token()
				manager.Check(ctx)
				return ui.Connection{Client: client, Address: address, Token: token, Notice: startupNotice}, nil
			}
		}
		client, err := codex.DialWithStartup(ctx, windowCtx, address, token)
		return ui.Connection{Client: client, Address: address, Token: token, Ticket: *popout, Notice: startupNotice}, err
	}
	_ = os.Unsetenv("FASTROCK_BROKER")
	_ = os.Unsetenv("FASTROCK_BROKER_TOKEN")
	code := ui.Run(ctx, store, prefs, connection)
	if broker != nil {
		debug.FreeOSMemory()
		broker.ReportServiceMemory(platform.ProcessMemoryBytes())
		go func() {
			ticker := time.NewTicker(10 * time.Second)
			defer ticker.Stop()
			for {
				select {
				case <-broker.Done():
					return
				case <-ticker.C:
					broker.ReportServiceMemory(platform.ProcessMemoryBytes())
				}
			}
		}()
		broker.Wait()
		updater.Close()
		if job := updater.Pending(); job != "" {
			if err := update.LaunchHelper(job); err != nil {
				platform.ShowError("The update is downloaded but could not start: " + err.Error())
			}
		}
	}
	return code
}
