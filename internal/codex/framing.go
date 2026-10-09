package codex

import (
	"bufio"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"sync"
)

const maxRPCFrame = 32 << 20

var errFrameTooLarge = errors.New("Codex message exceeds 32 MiB; operation outcome is uncertain")

// Drain oversized frames to their delimiter without retaining their contents.
// A large history response must not disconnect every other window.
func readFrame(r *bufio.Reader, limit int) ([]byte, error) {
	frame, _, err := readFrameHeader(r, limit)
	return frame, err
}

// Keep a tiny envelope while draining, even when params/result precedes id.
// This lets us reject an oversized server request instead of leaving it pending.
func readFrameHeader(r *bufio.Reader, limit int) ([]byte, Message, error) {
	var frame []byte
	var header envelopeReader
	oversized := false
	for {
		part, err := r.ReadSlice('\n')
		if !oversized {
			if len(frame)+len(part) > limit {
				header.feed(frame)
				oversized = true
				frame = nil
			} else {
				frame = append(frame, part...)
			}
		}
		if oversized {
			header.feed(part)
		}
		if err == bufio.ErrBufferFull {
			continue
		}
		if oversized {
			return nil, header.message, errFrameTooLarge
		}
		if err == io.EOF && len(frame) > 0 {
			return frame, header.message, nil
		}
		return frame, header.message, err
	}
}

type envelopeReader struct {
	depth           int
	quoted, escaped bool
	phase           int // 0 key, 1 colon, 2 value, 3 separator
	key             string
	token           []byte
	capture         bool
	message         Message
}

func (r *envelopeReader) finish() {
	if r.phase == 0 {
		_ = json.Unmarshal(r.token, &r.key)
		r.phase = 1
	} else if r.phase == 2 {
		switch r.key {
		case "id":
			if json.Valid(r.token) {
				r.message.ID = append(json.RawMessage(nil), r.token...)
			}
		case "method":
			_ = json.Unmarshal(r.token, &r.message.Method)
		}
		r.phase = 3
	}
	r.token = nil
	r.capture = false
}
func (r *envelopeReader) feed(data []byte) {
	for _, b := range data {
		if r.quoted {
			if r.capture && len(r.token) < 4096 {
				r.token = append(r.token, b)
			}
			if r.escaped {
				r.escaped = false
				continue
			}
			if b == '\\' {
				r.escaped = true
				continue
			}
			if b == '"' {
				r.quoted = false
				if r.depth == 1 {
					r.finish()
				}
			}
			continue
		}
		if b == '"' {
			r.quoted = true
			r.capture = r.depth == 1 && (r.phase == 0 || r.phase == 2 && (r.key == "id" || r.key == "method"))
			if r.capture {
				r.token = []byte{'"'}
			}
			continue
		}
		if b == '{' || b == '[' {
			r.depth++
			continue
		}
		if b == '}' || b == ']' {
			if r.depth == 1 && len(r.token) > 0 {
				r.finish()
			}
			r.depth--
			if r.depth == 1 && r.phase == 2 {
				r.phase = 3
			}
			continue
		}
		if r.depth != 1 {
			continue
		}
		switch b {
		case ':':
			if r.phase == 1 {
				r.phase = 2
			}
		case ',':
			if len(r.token) > 0 {
				r.finish()
			}
			r.phase = 0
			r.key = ""
		case ' ', '\n', '\r', '\t':
		default:
			if r.phase == 2 && r.key == "id" && len(r.token) < 4096 {
				r.token = append(r.token, b)
			}
		}
	}
}

type tailBuffer struct {
	mu   sync.Mutex
	data []byte
}

func (b *tailBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	n := len(p)
	if len(p) > 8192 {
		p = p[len(p)-8192:]
	}
	if excess := len(b.data) + len(p) - 8192; excess > 0 {
		copy(b.data, b.data[excess:])
		b.data = b.data[:len(b.data)-excess]
	}
	b.data = append(b.data, p...)
	return n, nil
}
func (b *tailBuffer) message() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.data) == 0 {
		return ""
	}
	return "\nRecent Codex diagnostic output:\n" + strings.ToValidUTF8(string(b.data), "�")
}
