// Package rally implements Rally WSAPI v2 directly; no vendor REST wrapper.
package rally

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/allquixotic/fastrock/internal/buildinfo"
	"io"
	"net"
	"net/http"
	"net/url"
	"path"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

const WSAPI = "/slm/webservice/v2.0/"

func lower(s string) string { return strings.ToLower(s) }

type Client struct {
	schemaMu sync.Mutex
	schemas  map[string]schemaEntry
	slots    chan struct{}
	cache    *pageCache
	base     *url.URL
	token    string
	http     *http.Client
}

func New(endpoint, token string, transport http.RoundTripper) (*Client, error) {
	u, e := url.Parse(strings.TrimRight(strings.TrimSpace(endpoint), "/"))
	if e != nil {
		return nil, errors.New("invalid Rally endpoint")
	}
	if u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return nil, errors.New("Rally endpoint must be an HTTPS server URL")
	}
	local := u.Hostname() == "localhost"
	if ip := net.ParseIP(u.Hostname()); ip != nil {
		local = ip.IsLoopback()
	}
	if u.Scheme != "https" && !(u.Scheme == "http" && local) {
		return nil, errors.New("Rally requires HTTPS (HTTP is allowed for loopback test servers)")
	}
	if strings.TrimSpace(token) == "" {
		return nil, errors.New("configure a Rally API token in Settings")
	}
	u.Path = strings.TrimSuffix(u.Path, strings.TrimSuffix(WSAPI, "/"))
	u.Path = strings.TrimRight(u.Path, "/") + WSAPI
	if transport == nil {
		t := http.DefaultTransport.(*http.Transport).Clone()
		t.MaxIdleConnsPerHost = 4
		transport = t
	}
	c := &Client{slots: make(chan struct{}, 4), cache: newPageCache(), base: u, token: token, http: &http.Client{Timeout: 30 * time.Second, Transport: transport}}
	c.http.CheckRedirect = func(r *http.Request, via []*http.Request) error {
		return errors.New("Rally redirects are disabled; configure the final endpoint in Settings")
	}
	return c, nil
}
func (c *Client) resolve(ref string) (*url.URL, error) {
	u, e := url.Parse(ref)
	if e != nil {
		return nil, errors.New("invalid Rally object reference")
	}
	if u.RawQuery != "" || u.Fragment != "" || strings.Contains(u.Path, "..") || strings.Contains(u.Path, "\\") || u.User != nil {
		return nil, errors.New("invalid Rally object reference")
	}
	if u.IsAbs() {
		if u.Scheme != c.base.Scheme || u.Host != c.base.Host {
			return nil, errors.New("refusing a Rally reference to another server")
		}
	} else if strings.HasPrefix(ref, "/") {
		u = c.base.ResolveReference(u)
	} else {
		u = c.base.ResolveReference(&url.URL{Path: ref})
	}
	if !strings.HasPrefix(u.Path, c.base.Path) {
		return nil, errors.New("Rally reference is outside WSAPI")
	}
	return u, nil
}
func (c *Client) requestBytes(ctx context.Context, method, ref string, query url.Values, body any) ([]byte, int, error) {
	u, e := c.resolve(ref)
	if e != nil {
		return nil, 0, e
	}
	u.RawQuery = query.Encode()
	var b []byte
	if body != nil {
		b, e = json.Marshal(body)
		if e != nil {
			return nil, 0, e
		}
	}
	for attempt := 0; ; attempt++ {
		req, e := http.NewRequestWithContext(ctx, method, u.String(), bytes.NewReader(b))
		if e != nil {
			return nil, 0, e
		}
		req.Header.Set("ZSESSIONID", c.token)
		req.Header.Set("Accept", "application/json")
		req.Header.Set("X-RallyIntegrationName", "Fastrock")
		req.Header.Set("X-RallyIntegrationVendor", "Independent")
		req.Header.Set("X-RallyIntegrationVersion", buildinfo.Version)
		if body != nil {
			req.Header.Set("Content-Type", "application/json")
		}
		select {
		case c.slots <- struct{}{}:
		case <-ctx.Done():
			return nil, 0, ctx.Err()
		}
		resp, e := c.http.Do(req)
		if e != nil {
			<-c.slots
			return nil, 0, fmt.Errorf("Rally connection failed: %w", e)
		}
		raw, e := io.ReadAll(io.LimitReader(resp.Body, (32<<20)+1))
		resp.Body.Close()
		<-c.slots
		if len(raw) > 32<<20 {
			return nil, 0, errors.New("Rally response exceeds the 32 MiB limit; narrow the query")
		}
		if e != nil {
			return nil, 0, e
		}
		if method == http.MethodGet && attempt < 3 && (resp.StatusCode == 429 || resp.StatusCode == 502 || resp.StatusCode == 503 || resp.StatusCode == 504) {
			delay := time.Duration(1<<attempt) * 200 * time.Millisecond
			if n, err := strconv.Atoi(resp.Header.Get("Retry-After")); err == nil && n >= 0 {
				delay = time.Duration(n) * time.Second
			} else if until, err := http.ParseTime(resp.Header.Get("Retry-After")); err == nil {
				delay = time.Until(until)
			}
			if delay < 0 {
				delay = 0
			}
			if delay > 30*time.Second {
				delay = 30 * time.Second
			}
			timer := time.NewTimer(delay)
			select {
			case <-timer.C:
				continue
			case <-ctx.Done():
				timer.Stop()
				return nil, 0, ctx.Err()
			}
		}
		return raw, resp.StatusCode, nil
	}
}
func (c *Client) request(ctx context.Context, method, ref string, query url.Values, body any) (map[string]json.RawMessage, error) {
	raw, status, err := c.requestBytes(ctx, method, ref, query, body)
	if err != nil {
		return nil, err
	}
	var envelope map[string]json.RawMessage
	decodeErr := json.Unmarshal(raw, &envelope)
	var messages []string
	if decodeErr == nil {
		for _, value := range envelope {
			var result responseStatus
			if json.Unmarshal(value, &result) == nil {
				messages = append(messages, result.Errors...)
			}
		}
	}
	if err := c.responseError(status, decodeErr, messages); err != nil {
		return nil, err
	}
	return envelope, nil
}

func QueryValues(q Query) url.Values {
	v := url.Values{"fetch": {q.Fetch}, "start": {strconv.Itoa(max(q.Start, 1))}, "pagesize": {strconv.Itoa(min(max(q.PageSize, 1), 2000))}}
	if q.Fetch == "" {
		v.Set("fetch", "true")
	}
	if q.PageSize <= 0 {
		v.Set("pagesize", "200")
	}
	if q.ArtifactTypes != "" {
		v.Set("types", q.ArtifactTypes)
	}
	if q.Expression != "" {
		v.Set("query", q.Expression)
	}
	if q.Order != "" {
		v.Set("order", q.Order)
	}
	if q.Workspace != "" {
		v.Set("workspace", q.Workspace)
	}
	if q.Project != "" {
		v.Set("project", q.Project)
	}
	v.Set("projectScopeUp", strconv.FormatBool(q.Parents))
	v.Set("projectScopeDown", strconv.FormatBool(q.Children))
	return v
}
func (c *Client) Query(ctx context.Context, kind string, q Query) (Page, error) {
	if isArtifactQuery(kind) {
		return c.queryArtifacts(ctx, q)
	}
	canonical, ok := CanonicalKind(kind)
	if !ok {
		return Page{}, errors.New("unsupported Rally entity type")
	}
	if q.ArtifactTypes != "" {
		types, err := canonicalArtifactTypes(strings.Split(q.ArtifactTypes, ","))
		if err != nil {
			return Page{}, err
		}
		if !slices.Contains(types, canonical) {
			return Page{Start: max(q.Start, 1), PageSize: q.PageSize}, nil
		}
	}
	q.ArtifactTypes = ""
	return c.Collection(ctx, strings.ToLower(canonical), q)
}
func (c *Client) Collection(ctx context.Context, ref string, q Query) (Page, error) {
	raw, status, err := c.requestBytes(ctx, http.MethodGet, ref, QueryValues(q), nil)
	if err != nil {
		return Page{}, err
	}
	return c.decodePage(raw, status)
}

func (c *Client) All(ctx context.Context, kind string, q Query) ([]Object, error) {
	var all []Object
	q.Start = 1
	q.PageSize = 200
	for {
		p, e := c.Query(ctx, kind, q)
		if e != nil {
			return nil, e
		}
		if p.Start != 0 && p.Start != q.Start {
			return nil, errors.New("Rally returned an unexpected page; refresh the query")
		}
		if len(p.Results) == 0 && len(all) < p.Total {
			return nil, errors.New("Rally returned an incomplete result; refresh the query")
		}
		all = append(all, p.Results...)
		if len(all) > 100000 {
			return nil, errors.New("Rally result exceeds 100,000 items; narrow the filter")
		}
		if len(all) >= p.Total || len(p.Results) == 0 {
			return all, nil
		}
		q.Start += len(p.Results)
	}
}
func (c *Client) Get(ctx context.Context, ref string) (Object, error) {
	return c.getFields(ctx, ref, "true")
}

// ReferenceKind accepts only a concrete object reference on this client's
// WSAPI origin, never a collection, query or a reference to another server.
func (c *Client) ReferenceKind(ref string) (string, bool) {
	u, err := c.resolve(ref)
	if err != nil {
		return "", false
	}
	relative := strings.TrimPrefix(u.Path, c.base.Path)
	parts := strings.Split(relative, "/")
	if len(parts) < 2 || parts[len(parts)-1] == "" {
		return "", false
	}
	if id, err := strconv.ParseUint(parts[len(parts)-1], 10, 64); err != nil || id == 0 {
		return "", false
	}
	return CanonicalKind(strings.Join(parts[:len(parts)-1], "/"))
}

// The unscoped, unfiltered user read returns the authenticated caller, not
// the workspace's user collection. Keep identity discovery separate from lists.
func (c *Client) CurrentUser(ctx context.Context) (Object, error) {
	o, err := c.getFields(ctx, "user", "ObjectID,UserName,DisplayName")
	if err != nil {
		return nil, err
	}
	ref, err := c.resolve(o.String("_ref"))
	if err != nil || ref == nil || !strings.EqualFold(path.Dir(strings.TrimPrefix(ref.Path, c.base.Path)), "user") || o.String("ObjectID") == "" || path.Base(ref.Path) != o.String("ObjectID") {
		return nil, errors.New("Rally returned an invalid current-user identity")
	}
	return o, nil
}
func (c *Client) getFields(ctx context.Context, ref, fetch string) (Object, error) {
	env, e := c.request(ctx, "GET", ref, url.Values{"fetch": {fetch}}, nil)
	if e != nil {
		return nil, e
	}
	for _, v := range env {
		var o Object
		if json.Unmarshal(v, &o) == nil && o["ObjectID"] != nil {
			return o, nil
		}
	}
	return nil, errors.New("Rally object not found")
}
func (c *Client) Create(ctx context.Context, kind string, fields Object) (Object, error) {
	k, ok := CanonicalKind(kind)
	if !ok {
		return nil, errors.New("unsupported Rally entity type")
	}
	return c.mutate(ctx, "POST", lower(k)+"/create", k, fields)
}
func (c *Client) Update(ctx context.Context, ref, kind string, fields Object) (Object, error) {
	k, ok := CanonicalKind(kind)
	if !ok {
		return nil, errors.New("unsupported Rally entity type")
	}
	return c.mutate(ctx, "POST", ref, k, fields)
}
func (c *Client) mutate(ctx context.Context, method, ref, kind string, fields Object) (Object, error) {
	return c.mutateQuery(ctx, method, ref, kind, fields, nil)
}
func (c *Client) mutateQuery(ctx context.Context, method, ref, kind string, fields Object, query url.Values) (Object, error) {
	defer c.PurgeCache()
	key := path.Base(kind)
	env, e := c.request(ctx, method, ref, query, map[string]any{key: fields})
	if e != nil {
		return nil, e
	}
	for _, k := range []string{"CreateResult", "OperationResult"} {
		if raw, ok := env[k]; ok {
			var r struct{ Object Object }
			if e = json.Unmarshal(raw, &r); e != nil {
				return nil, e
			}
			if r.Object != nil {
				return r.Object, nil
			}
		}
	}
	return nil, errors.New("Rally mutation returned no object")
}
func (c *Client) Delete(ctx context.Context, ref string) error {
	defer c.PurgeCache()
	_, e := c.request(ctx, "DELETE", ref, nil, nil)
	return e
}
func (c *Client) Fields(ctx context.Context, kind string, workspace ...string) ([]Field, error) {
	scope := ""
	if len(workspace) > 0 {
		scope = workspace[0]
	}
	k, ok := CanonicalKind(kind)
	if !ok {
		return nil, errors.New("unsupported Rally entity type")
	}
	key := scope + "\x00" + k
	c.schemaMu.Lock()
	if entry, ok := c.schemas[key]; ok && time.Now().Before(entry.expires) {
		c.schemaMu.Unlock()
		return cloneFields(entry.fields), nil
	}
	c.schemaMu.Unlock()
	definition, e := c.Query(ctx, "TypeDefinition", Query{Expression: Eq("TypePath", k), Workspace: scope, PageSize: 2, Fetch: "ObjectID,TypePath,Attributes"})
	if e != nil {
		return nil, e
	}
	if len(definition.Results) != 1 {
		return nil, errors.New("Rally schema type not found")
	}
	p, e := c.Collection(ctx, definition.Results[0].Ref("Attributes"), Query{PageSize: 2000, Workspace: scope})
	if e != nil {
		return nil, e
	}
	if p.Total > len(p.Results) {
		return nil, errors.New("Rally schema attributes are incomplete")
	}
	fields := make([]Field, 0, len(p.Results))
	for _, o := range p.Results {
		f := Field{Name: o.String("ElementName"), DisplayName: o.String("Name"), AttributeType: o.String("AttributeType"), Required: o.Bool("Required"), ReadOnly: o.Bool("ReadOnly")}
		if f.AttributeType == "OBJECT" {
			f.ReferenceType = o.String("Type")
			if kind, _ := o["AllowedValueType"].(map[string]any); kind != nil {
				if name, _ := kind["TypePath"].(string); name != "" {
					f.ReferenceType = name
				}
			}
		}
		if ref := o.Ref("AllowedValues"); ref != "" {
			values, err := c.Collection(ctx, ref, Query{PageSize: 2000, Workspace: scope})
			if err != nil {
				return nil, fmt.Errorf("%s allowed values: %w", f.Name, err)
			}
			if values.Total > len(values.Results) {
				return nil, fmt.Errorf("%s has too many allowed values; narrow the schema query", f.Name)
			}
			if err == nil {
				for _, v := range values.Results {
					f.AllowedValues = append(f.AllowedValues, v.String("StringValue"))
				}
			}
		}
		fields = append(fields, f)
	}
	c.schemaMu.Lock()
	if c.schemas == nil {
		c.schemas = map[string]schemaEntry{}
	}
	if len(c.schemas) >= 32 {
		clear(c.schemas)
	}
	c.schemas[key] = schemaEntry{cloneFields(fields), time.Now().Add(10 * time.Minute)}
	c.schemaMu.Unlock()
	return fields, nil
}
func (c *Client) Upload(ctx context.Context, parentRef, name, contentType string, data []byte) (Object, error) {
	if len(data) > 5<<20 {
		return nil, errors.New("Rally attachment exceeds 5 MiB")
	}
	content, e := c.Create(ctx, "AttachmentContent", Object{"Content": base64.StdEncoding.EncodeToString(data)})
	if e != nil {
		return nil, e
	}
	attachment, e := c.Create(ctx, "Attachment", Object{"Artifact": parentRef, "Content": content.String("_ref"), "Name": name, "ContentType": contentType, "Size": len(data)})
	if e != nil {
		cleanupCtx, cancel := context.WithTimeout(context.WithoutCancel(ctx), 5*time.Second)
		defer cancel()
		_ = c.Delete(cleanupCtx, content.String("_ref"))
	}
	return attachment, e
}
func Eq(field, value string) string { return "(" + field + " = " + Quote(value) + ")" }
func Quote(s string) string {
	return `"` + strings.NewReplacer(`\`, `\\`, `"`, `\"`, "\n", `\n`, "\r", `\r`).Replace(s) + `"`
}
func And(a, b string) string {
	if a == "" {
		return b
	}
	if b == "" {
		return a
	}
	return "(" + a + " AND " + b + ")"
}

// UpdateIfUnchanged shares the revision guard used by all explicit editors.
func (c *Client) UpdateIfUnchanged(ctx context.Context, before Object, kind string, fields Object) (Object, error) {
	current, err := c.Get(ctx, before.String("_ref"))
	if err != nil {
		return nil, err
	}
	for _, key := range []string{"LastUpdateDate", "VersionId"} {
		if stamp := before.String(key); stamp != "" && stamp != current.String(key) {
			return nil, &ConflictError{Ref: before.String("_ref")}
		}
	}
	return c.Update(ctx, before.String("_ref"), kind, fields)
}
