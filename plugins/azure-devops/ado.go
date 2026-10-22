package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"sort"
	"strings"
	"time"
)

// Repo is an Azure DevOps Git repository, as named in a remote URL.
type Repo struct {
	// BaseURL is the organization's collection URL, like
	// https://dev.azure.com/acme or https://acme.visualstudio.com.
	BaseURL string
	Project string
	Name    string
}

var (
	// https://dev.azure.com/{org}/{project}/_git/{repo}, with an optional user@.
	devAzureHTTPS = regexp.MustCompile(`^https?://(?:[^@/]+@)?dev\.azure\.com/([^/]+)/([^/]+)/_git/([^/?#]+)`)
	// https://{org}.visualstudio.com/[DefaultCollection/]{project}/_git/{repo}
	visualStudioHTTPS = regexp.MustCompile(`^https?://(?:[^@/]+@)?([^./]+)\.visualstudio\.com/(?:DefaultCollection/)?([^/]+)/_git/([^/?#]+)`)
	// git@ssh.dev.azure.com:v3/{org}/{project}/{repo} (also ssh:// form)
	devAzureSSH = regexp.MustCompile(`^(?:ssh://)?[^@]+@ssh\.dev\.azure\.com[:/]v3/([^/]+)/([^/]+)/([^/]+)$`)
	// {org}@vs-ssh.visualstudio.com:v3/{org}/{project}/{repo}
	visualStudioSSH = regexp.MustCompile(`^(?:ssh://)?[^@]+@vs-ssh\.visualstudio\.com[:/]v3/([^/]+)/([^/]+)/([^/]+)$`)
)

// ParseRemote reads an Azure DevOps repository from a git remote URL. It
// returns false for remotes on other hosts.
func ParseRemote(remote string) (Repo, bool) {
	remote = strings.TrimSpace(remote)
	unescape := func(s string) string {
		if u, err := url.PathUnescape(s); err == nil {
			return u
		}
		return s
	}
	repo := func(base, project, name string) (Repo, bool) {
		name = strings.TrimSuffix(name, ".git")
		return Repo{BaseURL: base, Project: unescape(project), Name: unescape(name)}, true
	}
	if m := devAzureHTTPS.FindStringSubmatch(remote); m != nil {
		return repo("https://dev.azure.com/"+m[1], m[2], m[3])
	}
	if m := visualStudioHTTPS.FindStringSubmatch(remote); m != nil {
		return repo("https://"+m[1]+".visualstudio.com", m[2], m[3])
	}
	if m := devAzureSSH.FindStringSubmatch(remote); m != nil {
		return repo("https://dev.azure.com/"+m[1], m[2], m[3])
	}
	if m := visualStudioSSH.FindStringSubmatch(remote); m != nil {
		return repo("https://"+m[1]+".visualstudio.com", m[2], m[3])
	}
	return Repo{}, false
}

// repoPath is the repository's path segment of a REST or web URL.
func (r Repo) repoPath() string {
	return r.BaseURL + "/" + url.PathEscape(r.Project) + "/_git/" + url.PathEscape(r.Name)
}

// PullRequestURL is the web page of pull request id.
func (r Repo) PullRequestURL(id int) string {
	return fmt.Sprintf("%s/pullrequest/%d", r.repoPath(), id)
}

// PullRequest is the part of an Azure DevOps pull request the plugin uses.
type PullRequest struct {
	ID            int    `json:"pullRequestId"`
	Title         string `json:"title"`
	SourceRefName string `json:"sourceRefName"`
}

// Branch is the source branch's name, or "" when the PR isn't from a branch.
func (p PullRequest) Branch() string {
	if b, ok := strings.CutPrefix(p.SourceRefName, "refs/heads/"); ok {
		return b
	}
	return ""
}

// Client calls the Azure DevOps REST API with a token.
type Client struct {
	HTTP  *http.Client
	Token string
	// APIBase overrides Repo.BaseURL for API calls (tests).
	APIBase string
}

// ErrUnauthorized means Azure DevOps refused the token.
type ErrUnauthorized struct{ Status string }

func (e ErrUnauthorized) Error() string {
	return "Azure DevOps refused ADO_MCP_AUTH_TOKEN (" + e.Status + "); check that it hasn't expired and can read code"
}

// ActivePullRequests lists the repository's active pull requests.
func (c Client) ActivePullRequests(r Repo) ([]PullRequest, error) {
	base := r.BaseURL
	if c.APIBase != "" {
		base = c.APIBase
	}
	endpoint := fmt.Sprintf("%s/%s/_apis/git/repositories/%s/pullrequests?searchCriteria.status=active&$top=1000&api-version=7.1",
		base, url.PathEscape(r.Project), url.PathEscape(r.Name))
	// ADO_MCP_AUTH_TOKEN is usually an Entra access token (Bearer); a
	// personal access token needs Basic auth instead.
	var lastErr error
	for _, auth := range []string{"Bearer " + c.Token, "Basic " + base64.StdEncoding.EncodeToString([]byte(":"+c.Token))} {
		prs, err := c.get(endpoint, auth)
		if _, unauthorized := err.(ErrUnauthorized); unauthorized {
			lastErr = err
			continue
		}
		return prs, err
	}
	return nil, lastErr
}

func (c Client) get(endpoint, auth string) ([]PullRequest, error) {
	req, err := http.NewRequest(http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", auth)
	req.Header.Set("Accept", "application/json")
	httpClient := c.HTTP
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 20 * time.Second}
	}
	resp, err := httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	// A rejected token gets 401, or 203 with a sign-in page.
	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusNonAuthoritativeInfo ||
		resp.StatusCode == http.StatusForbidden {
		return nil, ErrUnauthorized{Status: resp.Status}
	}
	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return nil, fmt.Errorf("Azure DevOps answered %s: %s", resp.Status, strings.TrimSpace(string(body)))
	}
	var page struct {
		Value []PullRequest `json:"value"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&page); err != nil {
		return nil, fmt.Errorf("reading Azure DevOps's answer: %w", err)
	}
	return page.Value, nil
}

// ByBranch maps each source branch to its pull request. A branch with
// several active PRs gets the newest.
func ByBranch(prs []PullRequest) map[string]PullRequest {
	sort.Slice(prs, func(i, j int) bool { return prs[i].ID < prs[j].ID })
	out := map[string]PullRequest{}
	for _, pr := range prs {
		if b := pr.Branch(); b != "" {
			out[b] = pr
		}
	}
	return out
}

// Label is what the branch list shows for pr.
func Label(pr PullRequest) string {
	return fmt.Sprintf("!%d", pr.ID)
}
