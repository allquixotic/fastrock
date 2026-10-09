package update

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
	"unicode/utf16"

	"github.com/allquixotic/fastrock/internal/platform"
	"golang.org/x/sys/windows"
)

// Get-AuthenticodeSignature uses Windows Authenticode trust policy and returns
// the signer selected by that verification. The program is constant; the path
// is passed as data, never interpolated into PowerShell source.
func signedPublisher(path string) (string, error) {
	system, err := windows.GetSystemDirectory()
	if err != nil {
		return "", err
	}
	script := `$ErrorActionPreference = 'Stop'; $s = Get-AuthenticodeSignature -LiteralPath $env:FASTROCK_SIGNATURE_PATH; if ($s.Status -ne 'Valid') { throw ('Authenticode verification failed: ' + $s.Status + ' ' + $s.StatusMessage) }; [Console]::Out.Write($s.SignerCertificate.Subject)`
	words := utf16.Encode([]rune(script))
	encoded := make([]byte, len(words)*2)
	for i, w := range words {
		binary.LittleEndian.PutUint16(encoded[i*2:], w)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	command := platform.CommandContext(ctx, filepath.Join(system, "WindowsPowerShell", "v1.0", "powershell.exe"), "-NoProfile", "-NonInteractive", "-EncodedCommand", base64.StdEncoding.EncodeToString(encoded))
	command.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	// Windows PowerShell must rebuild its own module path. Inheriting PS7's
	// modules can prevent its Authenticode command from loading at all.
	command.Env = append(platform.ChildEnv("FASTROCK_SIGNATURE_PATH", "PSModulePath"), "FASTROCK_SIGNATURE_PATH="+path)
	output, err := command.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("Authenticode verification failed: %w: %s", err, strings.TrimSpace(string(output)))
	}
	subject := strings.TrimSpace(string(output))
	if subject == "" {
		return "", errors.New("Authenticode signer is unavailable")
	}
	return subject, nil
}
func verifyPlatform(path string) error {
	current, err := os.Executable()
	if err != nil {
		return err
	}
	trusted, err := signedPublisher(current)
	if err != nil {
		return fmt.Errorf("cannot authenticate the running Fastrock publisher: %w", err)
	}
	next, err := signedPublisher(path)
	if err != nil {
		return err
	}
	// Compare the complete name, not a certificate thumbprint: Azure's public
	// certificates are short-lived and renewed with the same publisher identity.
	if next != trusted {
		return errors.New("update publisher does not match the running Fastrock publisher")
	}
	return nil
}
