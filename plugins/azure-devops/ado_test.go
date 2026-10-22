package main

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestParseRemote(t *testing.T) {
	dev := Repo{BaseURL: "https://dev.azure.com/acme", Project: "Web Shop", Name: "api"}
	vs := Repo{BaseURL: "https://acme.visualstudio.com", Project: "Web Shop", Name: "api"}
	cases := map[string]Repo{
		"https://dev.azure.com/acme/Web%20Shop/_git/api":                      dev,
		"https://acme@dev.azure.com/acme/Web%20Shop/_git/api":                 dev,
		"https://dev.azure.com/acme/Web%20Shop/_git/api.git":                  dev,
		"git@ssh.dev.azure.com:v3/acme/Web%20Shop/api":                        dev,
		"ssh://git@ssh.dev.azure.com/v3/acme/Web%20Shop/api":                  dev,
		"https://acme.visualstudio.com/Web%20Shop/_git/api":                   vs,
		"https://acme.visualstudio.com/DefaultCollection/Web%20Shop/_git/api": vs,
		"acme@vs-ssh.visualstudio.com:v3/acme/Web%20Shop/api":                 vs,
	}
	for remote, want := range cases {
		got, ok := ParseRemote(remote)
		if !ok || got != want {
			t.Errorf("ParseRemote(%q) = %+v, %v; want %+v", remote, got, ok, want)
		}
	}
	for _, remote := range []string{"git@github.com:ThirtyTech/tt-git.git", "https://gitlab.com/a/b.git", ""} {
		if got, ok := ParseRemote(remote); ok {
			t.Errorf("ParseRemote(%q) = %+v; want not Azure DevOps", remote, got)
		}
	}
}

func TestPullRequestURL(t *testing.T) {
	repo := Repo{BaseURL: "https://dev.azure.com/acme", Project: "Web Shop", Name: "api"}
	want := "https://dev.azure.com/acme/Web%20Shop/_git/api/pullrequest/42"
	if got := repo.PullRequestURL(42); got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestByBranchKeepsTheNewestPR(t *testing.T) {
	got := ByBranch([]PullRequest{
		{ID: 9, SourceRefName: "refs/heads/feature/login"},
		{ID: 3, SourceRefName: "refs/heads/feature/login"},
		{ID: 5, SourceRefName: "refs/heads/fix"},
		{ID: 6, SourceRefName: "refs/pull/1/merge"},
	})
	if len(got) != 2 || got["feature/login"].ID != 9 || got["fix"].ID != 5 {
		t.Errorf("got %+v", got)
	}
}

// fakeADO answers the pull request list for tokens that authenticate with
// scheme ("Bearer" or "Basic").
func fakeADO(t *testing.T, scheme string) *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/Web Shop/_apis/git/repositories/api/pullrequests" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		if r.URL.Query().Get("searchCriteria.status") != "active" {
			t.Errorf("not asking for active PRs: %q", r.URL.RawQuery)
		}
		auth := r.Header.Get("Authorization")
		if len(auth) < len(scheme) || auth[:len(scheme)] != scheme {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		fmt.Fprint(w, `{"count":2,"value":[`+
			`{"pullRequestId":1423,"title":"Login","sourceRefName":"refs/heads/feature/login"},`+
			`{"pullRequestId":1431,"title":"Search","sourceRefName":"refs/heads/feature/search"}]}`)
	}))
}

func TestActivePullRequests(t *testing.T) {
	repo := Repo{BaseURL: "https://dev.azure.com/acme", Project: "Web Shop", Name: "api"}
	for _, scheme := range []string{"Bearer", "Basic"} {
		srv := fakeADO(t, scheme)
		prs, err := Client{Token: "secret", APIBase: srv.URL}.ActivePullRequests(repo)
		srv.Close()
		if err != nil || len(prs) != 2 || prs[0].ID != 1423 || prs[0].Branch() != "feature/login" {
			t.Errorf("%s: got %+v, %v", scheme, prs, err)
		}
	}
}

func TestActivePullRequestsRefusedToken(t *testing.T) {
	srv := fakeADO(t, "Nothing")
	defer srv.Close()
	repo := Repo{BaseURL: "https://dev.azure.com/acme", Project: "Web Shop", Name: "api"}
	_, err := Client{Token: "secret", APIBase: srv.URL}.ActivePullRequests(repo)
	if _, ok := err.(ErrUnauthorized); !ok {
		t.Errorf("got %v, want ErrUnauthorized", err)
	}
}
