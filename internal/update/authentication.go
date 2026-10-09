package update

import (
	"bytes"
	"context"
	"debug/macho"
	"debug/pe"
	"encoding/hex"
	"errors"
	"io"
	"path/filepath"
	"runtime"
	"strings"
)

// Checksums detect transport corruption. Publisher authentication is enforced
// separately on the extracted native executable, before ready and before install.
func (m *Manager) archiveDigest(ctx context.Context, r Release, asset Asset) ([]byte, error) {
	digest := strings.TrimPrefix(asset.Digest, "sha256:")
	if !strings.HasPrefix(asset.Digest, "sha256:") {
		for _, a := range r.Assets {
			if a.Name != "SHA256SUMS" {
				continue
			}
			response, err := m.request(ctx, a.URL)
			if err != nil {
				return nil, err
			}
			data, err := io.ReadAll(io.LimitReader(response.Body, (64<<10)+1))
			response.Body.Close()
			if err != nil {
				return nil, err
			}
			if len(data) > 64<<10 {
				return nil, errors.New("checksum manifest exceeds size limit")
			}
			found := false
			for _, line := range strings.Split(string(data), "\n") {
				fields := strings.Fields(line)
				if len(fields) == 2 && strings.TrimPrefix(fields[1], "*") == asset.Name {
					if found {
						return nil, errors.New("duplicate archive checksum")
					}
					digest, found = fields[0], true
				}
			}
			break
		}
	}
	expected, err := hex.DecodeString(digest)
	if err != nil || len(expected) != 32 {
		return nil, errors.New("release is missing a valid SHA-256 digest")
	}
	return expected, nil
}

// Read the version stamp from a signed native read-only section, never by executing downloaded
// code. A compromised release feed cannot relabel an older signed binary.
func verifyRelease(path, version string) error {
	if err := verifyPlatform(path); err != nil {
		return err
	}
	exe := path
	if runtime.GOOS == "darwin" {
		exe = filepath.Join(path, "Contents", "MacOS", "fastrock")
	}
	return verifyBuildVersion(exe, runtime.GOOS, version)
}

func verifyBuildVersion(exe, goos, version string) error {
	var read func() ([]byte, error)
	if goos == "darwin" {
		file, err := macho.Open(exe)
		if err != nil {
			return err
		}
		defer file.Close()
		section := file.Section("__rodata")
		if section != nil && section.Size <= 64<<20 {
			read = section.Data
		}
	} else if goos == "windows" {
		file, err := pe.Open(exe)
		if err != nil {
			return err
		}
		defer file.Close()
		section := file.Section(".rdata")
		if section != nil && section.Size <= 64<<20 {
			read = section.Data
		}
	}
	if read == nil {
		return errors.New("signed executable has no bounded release metadata section")
	}
	data, err := read()
	if err != nil {
		return err
	}
	expected := []byte("FastrockRelease[" + strings.TrimPrefix(version, "v") + "]")
	if !bytes.Contains(data, expected) {
		return errors.New("signed executable version does not match the selected release")
	}
	return nil
}
