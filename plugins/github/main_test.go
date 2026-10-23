package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// With FAKE_TT_LOG set, the test binary stands in for the tt-git CLI: it
// answers `api branch.list` and records every other call, one per line.
func TestMain(m *testing.M) {
	if log := os.Getenv("FAKE_TT_LOG"); log != "" {
		args := os.Args[1:]
		if len(args) == 2 && args[0] == "api" && args[1] == "branch.list" {
			os.Stdout.WriteString(`[{"name":"main"},{"name":"feature/login"},{"name":"old"}]`)
			os.Exit(0)
		}
		f, _ := os.OpenFile(log, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
		line, _ := json.Marshal(args)
		f.Write(append(line, '\n'))
		f.Close()
		os.Stdout.WriteString("{}")
		os.Exit(0)
	}
	os.Exit(m.Run())
}

// pluginEnv sets up what tt-git gives the plugin: a repository whose origin
// is remote, and the test binary as tt-git. gh answers with no token, and
// the API calls go to api. It returns the call log.
func pluginEnv(t *testing.T, remote, api string) string {
	t.Helper()
	repo := t.TempDir()
	for _, args := range [][]string{{"init", "-q"}, {"remote", "add", "origin", remote}} {
		if out, err := exec.Command("git", append([]string{"-C", repo}, args...)...).CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	log := filepath.Join(t.TempDir(), "calls.jsonl")
	t.Setenv("TTGIT_REPO_PATH", repo)
	t.Setenv("TTGIT_BIN_PATH", self)
	t.Setenv("TTGIT_PLUGIN_ID", "thirtytech.github")
	t.Setenv("TTGIT_PLUGIN_STATE_DIR", t.TempDir())
	t.Setenv("FAKE_TT_LOG", log)
	for _, v := range []string{"GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN", "GH_HOST", hostsVar} {
		t.Setenv(v, "")
	}
	oldGh, oldAPI := ghToken, apiBase
	ghToken = func(string) string { return "" }
	apiBase = api
	t.Cleanup(func() { ghToken, apiBase = oldGh, oldAPI })
	return log
}

// httptestServer answers every request with answer, until the test ends.
func httptestServer(t *testing.T, answer func(http.ResponseWriter)) *httptest.Server {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { answer(w) }))
	t.Cleanup(srv.Close)
	return srv
}

func calls(t *testing.T, log string) [][]string {
	t.Helper()
	data, _ := os.ReadFile(log)
	var out [][]string
	for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		if line == "" {
			continue
		}
		var call []string
		if err := json.Unmarshal([]byte(line), &call); err != nil {
			t.Fatal(err)
		}
		out = append(out, call)
	}
	return out
}

func TestRefreshDecoratesBranchesWithPRs(t *testing.T) {
	srv := fakeGitHub(t, "secret")
	defer srv.Close()
	log := pluginEnv(t, "git@github.com:acme/api.git", srv.URL)
	t.Setenv("GITHUB_TOKEN", "secret")

	if err := refresh(true); err != nil {
		t.Fatal(err)
	}
	got := calls(t, log)
	if len(got) != 2 || got[0][1] != "branch.decorations.set" {
		t.Fatalf("calls: %v", got)
	}
	var params struct {
		PluginID    string            `json:"plugin_id"`
		Decorations map[string]string `json:"decorations"`
	}
	if err := json.Unmarshal([]byte(got[0][2]), &params); err != nil {
		t.Fatal(err)
	}
	// feature/search isn't in tt-git's branch list (yet), and is sent
	// anyway; the fork's main isn't.
	if params.PluginID != "thirtytech.github" || len(params.Decorations) != 2 ||
		params.Decorations["feature/login"] != "#1423" || params.Decorations["feature/search"] != "#1431" {
		t.Errorf("params: %+v", params)
	}
	if got[1][0] != "ui" || got[1][1] != "notify" || !strings.Contains(got[1][2], "1 of your branches") {
		t.Errorf("notify: %v", got[1])
	}
}

func TestFindTokenPrefersGh(t *testing.T) {
	pluginEnv(t, "git@github.com:acme/api.git", "")
	t.Setenv("GH_TOKEN", "from-env")
	t.Setenv("GH_ENTERPRISE_TOKEN", "from-enterprise-env")
	if tok, src := findToken("github.com"); tok != "from-env" || src != "GH_TOKEN" {
		t.Errorf("without gh: %q from %q", tok, src)
	}
	if tok, src := findToken("git.acme.com"); tok != "from-enterprise-env" || src != "GH_ENTERPRISE_TOKEN" {
		t.Errorf("enterprise: %q from %q", tok, src)
	}
	ghToken = func(host string) string { return "gh-" + host }
	if tok, src := findToken("github.com"); tok != "gh-github.com" || src != "gh auth token" {
		t.Errorf("with gh: %q from %q", tok, src)
	}
}

func TestRefreshFindsEnterpriseHosts(t *testing.T) {
	srv := fakeGitHub(t, "")
	defer srv.Close()
	log := pluginEnv(t, "https://git.acme.com/acme/api.git", srv.URL)
	if err := refresh(false); err != nil {
		t.Fatal(err)
	}
	if got := calls(t, log); len(got) != 1 || !strings.Contains(got[0][2], `"decorations":{}`) {
		t.Errorf("without the setting, calls: %v", got)
	}
	os.Remove(log)
	t.Setenv(hostsVar, "other.example, git.acme.com")
	if err := refresh(false); err != nil {
		t.Fatal(err)
	}
	if got := calls(t, log); len(got) != 1 || !strings.Contains(got[0][2], `"#1423"`) {
		t.Errorf("with the setting, calls: %v", got)
	}
}

func TestRefreshOutsideGitHubClearsTheColumn(t *testing.T) {
	log := pluginEnv(t, "https://dev.azure.com/acme/Web%20Shop/_git/api", "")
	if err := refresh(false); err != nil {
		t.Fatal(err)
	}
	got := calls(t, log)
	if len(got) != 1 || !strings.Contains(got[0][2], `"decorations":{}`) {
		t.Errorf("calls: %v", got)
	}
}

func TestRefreshWaitsOutTheRateLimit(t *testing.T) {
	asked := 0
	reset := time.Now().Add(time.Hour).Unix()
	srv := httptestServer(t, func(w http.ResponseWriter) {
		asked++
		w.Header().Set("X-RateLimit-Remaining", "0")
		w.Header().Set("X-RateLimit-Reset", strconv.FormatInt(reset, 10))
		w.WriteHeader(http.StatusForbidden)
	})
	log := pluginEnv(t, "git@github.com:acme/api.git", srv.URL)

	// The first refresh fails (one ✗ notice); later ones don't ask GitHub
	// or fail until the limit resets, and leave the column alone.
	if err := refresh(false); err == nil {
		t.Fatal("first refresh: no error")
	}
	if err := refresh(false); err != nil {
		t.Errorf("second refresh: %v", err)
	}
	if asked != 1 || len(calls(t, log)) != 0 {
		t.Errorf("asked %d times, calls: %v", asked, calls(t, log))
	}
	// Asking says why.
	err := refresh(true)
	got := calls(t, log)
	if err == nil || asked != 1 || len(got) != 1 || got[0][1] != "error" || !strings.Contains(got[0][2], "rate limit") {
		t.Errorf("asked %d times, err %v, calls: %v", asked, err, got)
	}
}

func TestRefreshWithoutATokenOnAPrivateRepoSaysHow(t *testing.T) {
	srv := httptestServer(t, func(w http.ResponseWriter) { w.WriteHeader(http.StatusNotFound) })
	log := pluginEnv(t, "git@github.com:acme/api.git", srv.URL)
	err := refresh(true)
	if err == nil || !strings.Contains(err.Error(), "gh auth login") {
		t.Fatalf("got %v", err)
	}
	got := calls(t, log)
	if len(got) != 1 || got[0][1] != "error" {
		t.Errorf("calls: %v", got)
	}
}
