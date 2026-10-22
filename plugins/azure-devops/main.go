// tt-git-ado shows each branch's active Azure DevOps pull request in
// tt-git's branch list. tt-git runs it as a plugin: see tt-git-plugin.toml.
//
//	tt-git-ado refresh [--notify]   look up the PRs and set the branch column
//	tt-git-ado open                 open the selected branch's PR in a browser
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// The variable holding the token, shared with the Azure DevOps MCP server.
const tokenVar = "ADO_MCP_AUTH_TOKEN"

// apiBase replaces the organization URL in API calls (tests).
var apiBase string

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: tt-git-ado refresh [--notify] | open")
		os.Exit(2)
	}
	notify := len(os.Args) > 2 && os.Args[2] == "--notify"
	var err error
	switch os.Args[1] {
	case "refresh":
		err = refresh(notify)
	case "open":
		err = open()
	default:
		err = fmt.Errorf("unknown command %q", os.Args[1])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "azure-devops:", err)
		os.Exit(1)
	}
}

// Saved is what the last refresh found, for `open`.
type Saved struct {
	Repo Repo                   `json:"repo"`
	PRs  map[string]PullRequest `json:"prs"`
}

func statePath() string {
	return filepath.Join(os.Getenv("TTGIT_PLUGIN_STATE_DIR"), "prs.json")
}

// refresh looks up the active PRs and replaces this plugin's column in the
// branch list. In a repository that isn't on Azure DevOps it clears the
// column and does nothing else.
func refresh(notify bool) error {
	repo, ok := findRepo()
	if !ok {
		setDecorations(map[string]string{})
		if notify {
			tt("ui", "notify", "Azure DevOps: origin isn't an Azure DevOps remote")
		}
		return nil
	}
	saved, err := lookUp(repo)
	if err != nil {
		if notify {
			tt("ui", "error", "Azure DevOps: "+err.Error())
		}
		return err
	}
	names, err := branchNames()
	if err != nil {
		return err
	}
	decorations := map[string]string{}
	for _, name := range names {
		if pr, ok := saved.PRs[name]; ok {
			decorations[name] = Label(pr)
		}
	}
	if err := setDecorations(decorations); err != nil {
		return err
	}
	if notify {
		tt("ui", "notify", fmt.Sprintf("Azure DevOps: %d of your branches have an active PR", len(decorations)))
	}
	return nil
}

// lookUp fetches repo's active PRs and saves them for `open`.
func lookUp(repo Repo) (Saved, error) {
	token := strings.TrimSpace(os.Getenv(tokenVar))
	if token == "" {
		return Saved{}, errors.New(tokenVar + " is not set; set it to an Azure DevOps access token or PAT before starting tt-git")
	}
	prs, err := Client{Token: token, APIBase: apiBase}.ActivePullRequests(repo)
	if err != nil {
		return Saved{}, err
	}
	saved := Saved{Repo: repo, PRs: ByBranch(prs)}
	if data, err := json.Marshal(saved); err == nil {
		_ = os.WriteFile(statePath(), data, 0o600)
	}
	return saved, nil
}

// open opens the selected branch's PR, looking the PRs up again when the
// last refresh didn't find one.
func open() error {
	var ctx struct {
		SelectedBranch string `json:"selected_branch"`
		Branch         string `json:"branch"`
	}
	_ = json.Unmarshal([]byte(os.Getenv("TTGIT_PLUGIN_CONTEXT_JSON")), &ctx)
	branch := ctx.SelectedBranch
	if branch == "" {
		branch = ctx.Branch
	}
	if branch == "" {
		tt("ui", "notify", "Azure DevOps: select a branch first")
		return nil
	}
	repo, ok := findRepo()
	if !ok {
		tt("ui", "notify", "Azure DevOps: origin isn't an Azure DevOps remote")
		return nil
	}
	var saved Saved
	if data, err := os.ReadFile(statePath()); err == nil {
		_ = json.Unmarshal(data, &saved)
	}
	pr, found := saved.PRs[branch]
	if !found || saved.Repo != repo {
		fresh, err := lookUp(repo)
		if err != nil {
			tt("ui", "error", "Azure DevOps: "+err.Error())
			return err
		}
		pr, found = fresh.PRs[branch]
	}
	if !found {
		tt("ui", "notify", "Azure DevOps: "+branch+" has no active pull request")
		return nil
	}
	return openBrowser(repo.PullRequestURL(pr.ID))
}

// findRepo reads the Azure DevOps repository from origin, else from the
// first remote that is on Azure DevOps.
func findRepo() (Repo, bool) {
	dir := os.Getenv("TTGIT_REPO_PATH")
	git := func(args ...string) string {
		out, err := exec.Command("git", append([]string{"-C", dir}, args...)...).Output()
		if err != nil {
			return ""
		}
		return strings.TrimSpace(string(out))
	}
	if repo, ok := ParseRemote(git("remote", "get-url", "origin")); ok {
		return repo, true
	}
	for _, name := range strings.Fields(git("remote")) {
		if repo, ok := ParseRemote(git("remote", "get-url", name)); ok {
			return repo, true
		}
	}
	return Repo{}, false
}

// branchNames lists the names in tt-git's branch list, so only branches
// you have get a column entry.
func branchNames() ([]string, error) {
	out, err := tt("api", "branch.list")
	if err != nil {
		return nil, err
	}
	var list []struct {
		Name string `json:"name"`
	}
	if err := json.Unmarshal(out, &list); err != nil {
		return nil, fmt.Errorf("reading the branch list: %w", err)
	}
	names := make([]string, len(list))
	for i, b := range list {
		names[i] = b.Name
	}
	return names, nil
}

func setDecorations(decorations map[string]string) error {
	params, _ := json.Marshal(map[string]any{
		"plugin_id":   os.Getenv("TTGIT_PLUGIN_ID"),
		"decorations": decorations,
	})
	_, err := tt("api", "branch.decorations.set", string(params))
	return err
}

// tt runs the tt-git CLI (the plugin API) and returns its stdout.
func tt(args ...string) ([]byte, error) {
	bin := os.Getenv("TTGIT_BIN_PATH")
	if bin == "" {
		bin = "tt-git"
	}
	cmd := exec.Command(bin, args...)
	cmd.Stderr = os.Stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("tt-git %s: %w", args[0], err)
	}
	return out, nil
}

func openBrowser(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	case "darwin":
		cmd = exec.Command("open", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	return cmd.Start()
}
