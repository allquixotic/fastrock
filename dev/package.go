//go:build ignore

// go run dev/package.go -os windows -arch amd64 -version v1.2.3
package main

import (
	"archive/zip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"flag"
	"fmt"
	"github.com/allquixotic/fastrock/internal/update"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

func must(e error) {
	if e != nil {
		panic(e)
	}
}
func run(env []string, name string, args ...string) {
	c := exec.Command(name, args...)
	c.Env = env
	c.Stdout = os.Stdout
	c.Stderr = os.Stderr
	must(c.Run())
}
func main() {
	goos := flag.String("os", runtime.GOOS, "Target OS")
	arch := flag.String("arch", runtime.GOARCH, "Target architecture")
	version := flag.String("version", "", "Release version")
	phase := flag.String("phase", "all", "build, package, or all (sign before package)")
	unsigned := flag.Bool("unsigned", false, "Developer fixtures only: skip native signing and notarization")
	dmg := flag.Bool("dmg", false, "Also build a macOS DMG")
	flag.Parse()
	v := strings.TrimPrefix(*version, "v")
	if !update.IsRelease(v) {
		panic("a release version is required")
	}
	if *phase != "all" && *phase != "build" && *phase != "package" {
		panic("invalid phase")
	}
	if *goos != "darwin" && *goos != "windows" {
		panic("unsupported release OS")
	}
	if *dmg && *goos != "darwin" {
		panic("DMGs require macOS")
	}
	if *arch != "amd64" && *arch != "arm64" {
		panic("unsupported architecture")
	}
	root := filepath.Join("build", "release", *goos+"-"+*arch)
	if *phase != "package" {
		must(os.RemoveAll(root))
		must(os.MkdirAll(root, 0755))
		must(os.MkdirAll("dist", 0755))
		exe := filepath.Join(root, "fastrock.exe")
		ld := "-s -w -X github.com/allquixotic/fastrock/internal/buildinfo.ReleaseStamp=FastrockRelease[" + v + "]"
		if *goos == "windows" {
			ld += " -H=windowsgui"
		} else if *goos == "darwin" {
			exe = filepath.Join(root, "Fastrock.app", "Contents", "MacOS", "fastrock")
			must(os.MkdirAll(filepath.Dir(exe), 0755))
			plist, e := os.ReadFile("packaging/macos/Info.plist")
			must(e)
			plist = []byte(strings.ReplaceAll(string(plist), "@FASTROCK_VERSION@", v))
			must(os.WriteFile(filepath.Join(root, "Fastrock.app", "Contents", "Info.plist"), plist, 0644))
		} else {
			panic("unsupported release OS")
		}
		env := append(os.Environ(), "CGO_ENABLED=0", "GOOS="+*goos, "GOARCH="+*arch)
		run(env, "go", "build", "-trimpath", "-ldflags", ld, "-o", exe, "./cmd/fastrock")
	}
	if *phase == "build" {
		return
	}
	must(os.MkdirAll("dist", 0755))
	if *goos == "darwin" && !*unsigned {
		if runtime.GOOS != "darwin" {
			panic("macOS release packaging requires codesign on macOS")
		}
		run(os.Environ(), "python3", "dev/sign_macos.py", "--app", filepath.Join(root, "Fastrock.app"), "--reports", filepath.Join(root, "..", "signing-"+*arch))
	} else if *goos == "windows" && !*unsigned {
		if runtime.GOOS != "windows" {
			panic("Windows release packaging requires native Authenticode verification; use -phase build for cross-builds")
		}
		run(os.Environ(), "powershell.exe", "-NoProfile", "-NonInteractive", "-File", "dev/verify-windows.ps1", "-Path", filepath.Join(root, "fastrock.exe"))
	}

	_ = os.Remove(filepath.Join(root, "Applications"))
	name := "fastrock-" + *goos + "-" + *arch
	zpath := filepath.Join("dist", name+".zip")
	out, e := os.Create(zpath)
	must(e)
	z := zip.NewWriter(out)
	must(filepath.WalkDir(root, func(path string, d os.DirEntry, e error) error {
		if e != nil {
			return e
		}
		if d.IsDir() {
			return nil
		}
		info, e := d.Info()
		if e != nil {
			return e
		}
		h, e := zip.FileInfoHeader(info)
		if e != nil {
			return e
		}
		rel, e := filepath.Rel(root, path)
		if e != nil {
			return e
		}
		h.Name = filepath.ToSlash(rel)
		h.Method = zip.Deflate
		h.Modified = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
		w, e := z.CreateHeader(h)
		if e != nil {
			return e
		}
		f, e := os.Open(path)
		if e != nil {
			return e
		}
		_, e = io.Copy(w, f)
		return errors.Join(e, f.Close())
	}))
	must(z.Close())
	must(out.Close())
	checksum(zpath)
	if *dmg {
		link := filepath.Join(root, "Applications")
		_ = os.Remove(link)
		must(os.Symlink("/Applications", link))
		path := filepath.Join("dist", name+".dmg")
		_ = os.Remove(path)
		run(os.Environ(), "hdiutil", "create", "-volname", "Fastrock", "-srcfolder", root, "-ov", "-format", "UDZO", path)
		if !*unsigned {
			run(os.Environ(), "python3", "dev/sign_macos.py", "--dmg", path, "--reports", filepath.Join(root, "..", "signing-"+*arch))
		}
		checksum(path)
	}
}
func checksum(path string) {
	f, e := os.Open(path)
	must(e)
	h := sha256.New()
	_, e = io.Copy(h, f)
	must(e)
	must(f.Close())
	must(os.WriteFile(path+".sha256", []byte(fmt.Sprintf("%s  %s\n", hex.EncodeToString(h.Sum(nil)), filepath.Base(path))), 0644))
}
