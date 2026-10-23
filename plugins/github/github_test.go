package main

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestParseRemote(t *testing.T) {
	gh := Repo{Host: "github.com", Owner: "ThirtyTech", Name: "tt-git"}
	ghe := Repo{Host: "git.acme.com", Owner: "web", Name: "api"}
	cases := map[string]Repo{
		"https://github.com/ThirtyTech/tt-git":               gh,
		"https://github.com/ThirtyTech/tt-git.git":           gh,
		"https://github.com/ThirtyTech/tt-git/":              gh,
		"https://jon@github.com/ThirtyTech/tt-git.git":       gh,
		"git@github.com:ThirtyTech/tt-git.git":               gh,
		"git@github.com:ThirtyTech/tt-git":                   gh,
		"ssh://git@github.com/ThirtyTech/tt-git.git":         gh,
		"ssh://git@ssh.github.com:443/ThirtyTech/tt-git.git": gh,
		"https://git.acme.com/web/api.git":                   ghe,
		"git@GIT.acme.com:web/api.git":                       ghe,
	}
	for remote, want := range cases {
		got, ok := ParseRemote(remote, []string{"git.acme.com"})
		if !ok || got != want {
			t.Errorf("ParseRemote(%q) = %+v, %v; want %+v", remote, got, ok, want)
		}
	}
	for _, remote := range []string{
		"https://dev.azure.com/acme/Web%20Shop/_git/api",
		"https://gitlab.com/a/b.git",
		"https://git.acme.com/web/api.git", // not named as an enterprise host below
		"https://github.com/ThirtyTech",
		"",
	} {
		if got, ok := ParseRemote(remote, nil); ok {
			t.Errorf("ParseRemote(%q) = %+v; want not GitHub", remote, got)
		}
	}
}

func TestURLs(t *testing.T) {
	gh := Repo{Host: "github.com", Owner: "ThirtyTech", Name: "tt-git"}
	ghe := Repo{Host: "git.acme.com", Owner: "web", Name: "api"}
	for got, want := range map[string]string{
		gh.PullRequestURL(42): "https://github.com/ThirtyTech/tt-git/pull/42",
		ghe.PullRequestURL(7): "https://git.acme.com/web/api/pull/7",
		gh.APIBase():          "https://api.github.com",
		ghe.APIBase():         "https://git.acme.com/api/v3",
	} {
		if got != want {
			t.Errorf("got %q, want %q", got, want)
		}
	}
}

func pr(number int, ref, headRepo string) PullRequest {
	var p PullRequest
	p.Number = number
	p.Head.Ref = ref
	if headRepo != "" {
		p.Head.Repo = &struct {
			FullName string `json:"full_name"`
		}{headRepo}
	}
	return p
}

func TestByBranchKeepsTheNewestPRAndSkipsForks(t *testing.T) {
	repo := Repo{Host: "github.com", Owner: "acme", Name: "api"}
	got := ByBranch([]PullRequest{
		pr(9, "feature/login", "acme/api"),
		pr(3, "feature/login", "acme/api"),
		pr(5, "fix", "Acme/API"),
		pr(6, "main", "someone/api"), // a fork's main isn't yours
		pr(7, "gone", ""),            // its fork was deleted
	}, repo)
	if len(got) != 2 || got["feature/login"].Number != 9 || got["fix"].Number != 5 {
		t.Errorf("got %+v", got)
	}
}

// fakeGitHub answers the open pull request list in pages of two, for the
// token "secret" (or anyone, when token is "").
func fakeGitHub(t *testing.T, token string) *httptest.Server {
	pages := []string{
		`[{"number":1431,"title":"Search","head":{"ref":"feature/search","repo":{"full_name":"acme/api"}}},` +
			`{"number":1423,"title":"Login","head":{"ref":"feature/login","repo":{"full_name":"acme/api"}}}]`,
		`[{"number":1400,"title":"Fork","head":{"ref":"main","repo":{"full_name":"someone/api"}}}]`,
	}
	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/repos/acme/api/pulls" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		if r.URL.Query().Get("state") != "open" {
			t.Errorf("not asking for open PRs: %q", r.URL.RawQuery)
		}
		if r.Header.Get("User-Agent") == "" {
			t.Error("no User-Agent")
		}
		if token != "" && r.Header.Get("Authorization") != "Bearer "+token {
			w.WriteHeader(http.StatusUnauthorized)
			fmt.Fprint(w, `{"message":"Bad credentials"}`)
			return
		}
		page, _ := strconv.Atoi(r.URL.Query().Get("page"))
		if page == 0 {
			page = 1
		}
		if page < len(pages) {
			w.Header().Set("Link", fmt.Sprintf(`<%s/repos/acme/api/pulls?state=open&per_page=100&page=%d>; rel="next", <%s/x>; rel="last"`, srv.URL, page+1, srv.URL))
		}
		fmt.Fprint(w, pages[page-1])
	}))
	return srv
}

func TestOpenPullRequestsFollowsPages(t *testing.T) {
	srv := fakeGitHub(t, "secret")
	defer srv.Close()
	repo := Repo{Host: "github.com", Owner: "acme", Name: "api"}
	prs, err := Client{Token: "secret", APIBase: srv.URL}.OpenPullRequests(repo)
	if err != nil || len(prs) != 3 || prs[0].Number != 1431 || prs[2].Number != 1400 {
		t.Errorf("got %+v, %v", prs, err)
	}
}

func TestOpenPullRequestsRefusedToken(t *testing.T) {
	srv := fakeGitHub(t, "secret")
	defer srv.Close()
	repo := Repo{Host: "github.com", Owner: "acme", Name: "api"}
	_, err := Client{Token: "wrong", TokenSource: "GH_TOKEN", APIBase: srv.URL}.OpenPullRequests(repo)
	if e, ok := err.(ErrUnauthorized); !ok || !strings.Contains(e.Error(), "GH_TOKEN") || strings.Contains(e.Error(), "wrong") {
		t.Errorf("got %v, want ErrUnauthorized naming GH_TOKEN and not the token", err)
	}
}

func TestOpenPullRequestsErrors(t *testing.T) {
	reset := time.Now().Add(30 * time.Minute).Truncate(time.Second)
	cases := []struct {
		name    string
		status  int
		headers map[string]string
		check   func(error) bool
	}{
		{"primary rate limit", 403, map[string]string{"X-RateLimit-Remaining": "0", "X-RateLimit-Reset": strconv.FormatInt(reset.Unix(), 10)},
			func(err error) bool { e, ok := err.(ErrRateLimited); return ok && e.Reset.Equal(reset) }},
		{"secondary rate limit", 429, map[string]string{"Retry-After": "60"},
			func(err error) bool { _, ok := err.(ErrRateLimited); return ok }},
		{"forbidden", 403, nil,
			func(err error) bool { return err != nil && strings.Contains(err.Error(), "Resource not accessible") }},
		{"not found", 404, nil,
			func(err error) bool { return err != nil && strings.Contains(err.Error(), "gh auth login") }},
	}
	for _, c := range cases {
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			for k, v := range c.headers {
				w.Header().Set(k, v)
			}
			w.WriteHeader(c.status)
			fmt.Fprint(w, `{"message":"Resource not accessible by integration"}`)
		}))
		repo := Repo{Host: "github.com", Owner: "acme", Name: "api"}
		_, err := Client{APIBase: srv.URL}.OpenPullRequests(repo)
		srv.Close()
		if !c.check(err) {
			t.Errorf("%s: got %v", c.name, err)
		}
	}
}
