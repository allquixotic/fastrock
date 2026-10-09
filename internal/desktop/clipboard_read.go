package desktop

import "github.com/allquixotic/fastrock/internal/desktop/command"

// ReadClipboardText permits asynchronous paste handling by applications that
// support image attachments in the same editor as native clipboard text.
func ReadClipboardText() string { value, _ := ReadClipboardTextResult(); return value }

func ReadClipboardTextResult() (string, error) {
	result := make(chan clipboardResult, 1)
	if err := submitClipboard(clipboardJob{kind: command.GetClipboardCmd, reply: func(value string, err error) { result <- clipboardResult{text: value, err: err} }}); err != nil {
		return "", err
	}
	r := <-result
	return r.text, r.err
}
