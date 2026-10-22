package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
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
// is remote, and the test binary as tt-git. It returns the call log.
func pluginEnv(t *testing.T, remote string) string {
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
	t.Setenv("TTGIT_PLUGIN_ID", "thirtytech.azure-devops")
	t.Setenv("TTGIT_PLUGIN_STATE_DIR", t.TempDir())
	t.Setenv("FAKE_TT_LOG", log)
	return log
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
	srv := fakeADO(t, "Bearer")
	defer srv.Close()
	apiBase = srv.URL
	defer func() { apiBase = "" }()
	log := pluginEnv(t, "https://dev.azure.com/acme/Web%20Shop/_git/api")
	t.Setenv(tokenVar, "secret")

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
	// feature/search isn't in tt-git's branch list (yet), and is sent anyway.
	if params.PluginID != "thirtytech.azure-devops" || len(params.Decorations) != 2 ||
		params.Decorations["feature/login"] != "!1423" || params.Decorations["feature/search"] != "!1431" {
		t.Errorf("params: %+v", params)
	}
	if got[1][0] != "ui" || got[1][1] != "notify" || !strings.Contains(got[1][2], "1 of your branches") {
		t.Errorf("notify: %v", got[1])
	}
}

func TestRefreshOutsideAzureDevOpsClearsTheColumn(t *testing.T) {
	log := pluginEnv(t, "git@github.com:ThirtyTech/tt-git.git")
	t.Setenv(tokenVar, "")
	if err := refresh(false); err != nil {
		t.Fatal(err)
	}
	got := calls(t, log)
	if len(got) != 1 || !strings.Contains(got[0][2], `"decorations":{}`) {
		t.Errorf("calls: %v", got)
	}
}

func TestRefreshWithoutATokenFails(t *testing.T) {
	log := pluginEnv(t, "https://dev.azure.com/acme/Web%20Shop/_git/api")
	t.Setenv(tokenVar, "")
	err := refresh(true)
	if err == nil || !strings.Contains(err.Error(), tokenVar) {
		t.Fatalf("got %v", err)
	}
	got := calls(t, log)
	if len(got) != 1 || got[0][1] != "error" {
		t.Errorf("calls: %v", got)
	}
}
