package codex

import (
	"context"
	"errors"
)

var ErrServerStopped = errors.New("Codex app-server stopped")
var ErrDisconnected = errors.New("Codex app-server disconnected")

const (
	CodeMethodNotFound  = -32601
	CodeInvalidParams   = -32602
	CodeInternalError   = -32603
	CodeDisconnected    = -32002
	CodeRequestRejected = -32003
	CodeRequestCanceled = -32800
)

func (e *RPCError) Is(target error) bool {
	return e.Code == CodeDisconnected && (target == ErrDisconnected || target == ErrServerStopped)
}

func protocolError(err error) *RPCError {
	var rpc *RPCError
	if errors.As(err, &rpc) {
		return rpc
	}
	code := CodeRequestRejected
	if errors.Is(err, ErrDisconnected) || errors.Is(err, ErrServerStopped) {
		code = CodeDisconnected
	}
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		code = CodeRequestCanceled
	}
	return &RPCError{Code: code, Message: err.Error()}
}
