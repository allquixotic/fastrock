package rally

import (
	"encoding/json"
	"errors"
	"net/http"
	"strings"
)

type responseStatus struct {
	Errors []string
}

type queryResult struct {
	responseStatus
	Results                                []Object
	TotalResultCount, StartIndex, PageSize int
}

// decodePage parses the complete envelope directly into its final result.
// Error fields and custom artifact properties are decoded in the same pass.
func (c *Client) decodePage(raw []byte, status int) (Page, error) {
	var envelope struct {
		QueryResult     *queryResult
		OperationResult *responseStatus
	}
	err := json.Unmarshal(raw, &envelope)
	var messages []string
	if envelope.QueryResult != nil {
		messages = append(messages, envelope.QueryResult.Errors...)
	}
	if envelope.OperationResult != nil {
		messages = append(messages, envelope.OperationResult.Errors...)
	}
	if err = c.responseError(status, err, messages); err != nil {
		return Page{}, err
	}
	p := envelope.QueryResult
	if p == nil {
		return Page{}, errors.New("Rally response has no QueryResult")
	}
	return Page{p.Results, p.TotalResultCount, p.StartIndex, p.PageSize}, nil
}

func (c *Client) responseError(status int, decodeErr error, messages []string) error {
	if decodeErr != nil {
		if status >= 400 {
			return &APIError{Status: status, Messages: []string{http.StatusText(status)}}
		}
		return errors.New("Rally returned invalid JSON")
	}
	if len(messages) > 0 {
		for i := range messages {
			messages[i] = strings.ReplaceAll(messages[i], c.token, "[redacted]")
		}
		return &APIError{Status: status, Messages: messages}
	}
	if status >= 400 {
		return &APIError{Status: status, Messages: []string{http.StatusText(status)}}
	}
	return nil
}
