package nucular

import "github.com/aarzilli/nucular/internal/clipboard"

// ReadClipboardText permits asynchronous paste handling by applications that
// support image attachments in the same editor as native clipboard text.
func ReadClipboardText() string { return clipboard.Get() }
