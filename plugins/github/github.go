package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

// Repo is a GitHub repository, as named in a remote URL.
type Repo struct {
	// Host is github.com or a GitHub Enterprise Server host.
	Host  string
	Owner string
	Name  string
}

var (
	// https://[user@]{host}[:port]/{owner}/{repo}[.git][/]
	httpsRemote = regexp.MustCompile(`^https?://(?:[^@/]+@)?([^/:]+)(?::\d+)?/([^/]+)/([^/?#]+?)(?:\.git)?/?$`)
	// git@{host}:{owner}/{repo}[.git]
	scpRemote = regexp.MustCompile(`^[^@/]+@([^:/]+):/?([^/]+)/([^/]+?)(?:\.git)?/?$`)
	// ssh://git@{host}[:port]/{owner}/{repo}[.git]
	sshRemote = regexp.MustCompile(`^ssh://(?:[^@/]+@)?([^/:]+)(?::\d+)?/([^/]+)/([^/]+?)(?:\.git)?/?$`)
)

// ParseRemote reads a GitHub repository from a git remote URL. Besides
// github.com it accepts the GitHub Enterprise hosts in enterprise. It
// returns false for remotes on other hosts.
func ParseRemote(remote string, enterprise []string) (Repo, bool) {
	remote = strings.TrimSpace(remote)
	var m []string
	for _, re := range []*regexp.Regexp{httpsRemote, scpRemote, sshRemote} {
		if m = re.FindStringSubmatch(remote); m != nil {
			break
		}
	}
	if m == nil {
		return Repo{}, false
	}
	host := strings.ToLower(m[1])
	switch host {
	case "github.com", "www.github.com", "ssh.github.com":
		return Repo{Host: "github.com", Owner: m[2], Name: m[3]}, true
	}
	for _, h := range enterprise {
		if strings.EqualFold(host, h) {
			return Repo{Host: host, Owner: m[2], Name: m[3]}, true
		}
	}
	return Repo{}, false
}

// FullName is owner/name, as GitHub names the repository.
func (r Repo) FullName() string {
	return r.Owner + "/" + r.Name
}

// APIBase is the REST API's root URL for the repository's host.
func (r Repo) APIBase() string {
	if r.Host == "github.com" {
		return "https://api.github.com"
	}
	return "https://" + r.Host + "/api/v3"
}

// PullRequestURL is the web page of pull request number.
func (r Repo) PullRequestURL(number int) string {
	return fmt.Sprintf("https://%s/%s/%s/pull/%d", r.Host, r.Owner, r.Name, number)
}

// PullRequest is the part of a GitHub pull request the plugin uses.
type PullRequest struct {
	Number int    `json:"number"`
	Title  string `json:"title"`
	Head   struct {
		Ref string `json:"ref"`
		// Repo is nil when the head repository has been deleted.
		Repo *struct {
			FullName string `json:"full_name"`
		} `json:"repo"`
	} `json:"head"`
}

// Branch is the head branch's name when the PR is from a branch of repo,
// else "" (a PR from a fork, or a deleted one).
func (p PullRequest) Branch(repo Repo) string {
	if p.Head.Repo == nil || !strings.EqualFold(p.Head.Repo.FullName, repo.FullName()) {
		return ""
	}
	return p.Head.Ref
}

// Client calls the GitHub REST API, with a token when there is one.
type Client struct {
	HTTP  *http.Client
	Token string
	// TokenSource names where the token came from, for messages.
	TokenSource string
	// APIBase overrides Repo.APIBase (tests).
	APIBase string
}

// ErrUnauthorized means GitHub refused the token.
type ErrUnauthorized struct{ Status, Source string }

func (e ErrUnauthorized) Error() string {
	return "GitHub refused the token from " + e.Source + " (" + e.Status + "); check that it hasn't expired, or run gh auth login"
}

// ErrRateLimited means GitHub won't answer again until Reset.
type ErrRateLimited struct{ Reset time.Time }

func (e ErrRateLimited) Error() string {
	return "GitHub's API rate limit is used up until " + e.Reset.Local().Format("15:04")
}

// maxPages caps a lookup at 1000 open pull requests.
const maxPages = 10

// OpenPullRequests lists the repository's open pull requests.
func (c Client) OpenPullRequests(r Repo) ([]PullRequest, error) {
	base := r.APIBase()
	if c.APIBase != "" {
		base = c.APIBase
	}
	next := fmt.Sprintf("%s/repos/%s/%s/pulls?state=open&per_page=100", base, r.Owner, r.Name)
	var all []PullRequest
	for page := 0; next != "" && page < maxPages; page++ {
		prs, link, err := c.get(next)
		if err != nil {
			return nil, err
		}
		all = append(all, prs...)
		next = nextLink(link)
	}
	return all, nil
}

func (c Client) get(endpoint string) ([]PullRequest, string, error) {
	req, err := http.NewRequest(http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, "", err
	}
	if c.Token != "" {
		req.Header.Set("Authorization", "Bearer "+c.Token)
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	req.Header.Set("X-GitHub-Api-Version", "2022-11-28")
	req.Header.Set("User-Agent", "tt-git-github")
	httpClient := c.HTTP
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 20 * time.Second}
	}
	resp, err := httpClient.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()
	if limited, ok := rateLimited(resp); ok {
		return nil, "", limited
	}
	switch resp.StatusCode {
	case http.StatusOK:
	case http.StatusUnauthorized:
		return nil, "", ErrUnauthorized{Status: resp.Status, Source: c.TokenSource}
	case http.StatusNotFound:
		// GitHub hides private repositories from tokens that can't see them.
		if c.Token == "" {
			return nil, "", errors.New("GitHub answered 404 Not Found; if the repository is private, run gh auth login or set GH_TOKEN")
		}
		return nil, "", fmt.Errorf("GitHub answered 404 Not Found; the repository doesn't exist or the token from %s can't see it", c.TokenSource)
	default:
		return nil, "", fmt.Errorf("GitHub answered %s: %s", resp.Status, message(resp.Body))
	}
	var prs []PullRequest
	if err := json.NewDecoder(resp.Body).Decode(&prs); err != nil {
		return nil, "", fmt.Errorf("reading GitHub's answer: %w", err)
	}
	return prs, resp.Header.Get("Link"), nil
}

// rateLimited reads a primary (X-RateLimit-Remaining: 0) or secondary
// (Retry-After) rate limit answer.
func rateLimited(resp *http.Response) (ErrRateLimited, bool) {
	if resp.StatusCode != http.StatusForbidden && resp.StatusCode != http.StatusTooManyRequests {
		return ErrRateLimited{}, false
	}
	if secs, err := strconv.Atoi(resp.Header.Get("Retry-After")); err == nil {
		return ErrRateLimited{Reset: time.Now().Add(time.Duration(secs) * time.Second)}, true
	}
	if resp.Header.Get("X-RateLimit-Remaining") == "0" {
		reset := time.Now().Add(time.Hour)
		if unix, err := strconv.ParseInt(resp.Header.Get("X-RateLimit-Reset"), 10, 64); err == nil {
			reset = time.Unix(unix, 0)
		}
		return ErrRateLimited{Reset: reset}, true
	}
	return ErrRateLimited{}, false
}

// message is the "message" of a GitHub error body, else the start of it.
func message(body io.Reader) string {
	data, _ := io.ReadAll(io.LimitReader(body, 2048))
	var e struct {
		Message string `json:"message"`
	}
	if json.Unmarshal(data, &e) == nil && e.Message != "" {
		return e.Message
	}
	if len(data) > 512 {
		data = data[:512]
	}
	return strings.TrimSpace(string(data))
}

var nextRel = regexp.MustCompile(`<([^>]+)>;\s*rel="next"`)

// nextLink is the rel="next" URL of a Link header, or "".
func nextLink(link string) string {
	if m := nextRel.FindStringSubmatch(link); m != nil {
		return m[1]
	}
	return ""
}

// ByBranch maps each head branch of repo to its pull request. A branch with
// several open PRs gets the newest; PRs from forks are left out.
func ByBranch(prs []PullRequest, repo Repo) map[string]PullRequest {
	sort.Slice(prs, func(i, j int) bool { return prs[i].Number < prs[j].Number })
	out := map[string]PullRequest{}
	for _, pr := range prs {
		if b := pr.Branch(repo); b != "" {
			out[b] = pr
		}
	}
	return out
}

// Label is what the branch list shows for pr.
func Label(pr PullRequest) string {
	return fmt.Sprintf("#%d", pr.Number)
}
