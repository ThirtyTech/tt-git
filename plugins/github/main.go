// tt-git-github shows each branch's open GitHub pull request in tt-git's
// branch list. tt-git runs it as a plugin: see tt-git-plugin.toml.
//
//	tt-git-github refresh [--notify]   look up the PRs and set the branch column
//	tt-git-github open                 open the selected branch's PR in a browser
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// The variable listing GitHub Enterprise Server hosts, comma or space
// separated. GH_HOST, which the gh CLI reads, counts as one too.
const hostsVar = "TTGIT_GITHUB_HOSTS"

// apiBase replaces the host's API URL in API calls (tests).
var apiBase string

// ghToken asks the gh CLI for its token for host (replaced in tests).
var ghToken = func(host string) string {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, "gh", "auth", "token", "--hostname", host).Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: tt-git-github refresh [--notify] | open")
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
		fmt.Fprintln(os.Stderr, "github:", err)
		os.Exit(1)
	}
}

// Saved is what the last refresh found, for `open`, and when GitHub's rate
// limit lets the plugin ask again.
type Saved struct {
	Repo             Repo                   `json:"repo"`
	PRs              map[string]PullRequest `json:"prs"`
	RateLimitedUntil time.Time              `json:"rate_limited_until"`
}

func statePath() string {
	return filepath.Join(os.Getenv("TTGIT_PLUGIN_STATE_DIR"), "prs.json")
}

func loadSaved() Saved {
	var saved Saved
	if data, err := os.ReadFile(statePath()); err == nil {
		_ = json.Unmarshal(data, &saved)
	}
	return saved
}

func (s Saved) save() {
	if data, err := json.Marshal(s); err == nil {
		_ = os.WriteFile(statePath(), data, 0o600)
	}
}

// refresh looks up the open PRs and replaces this plugin's column in the
// branch list. In a repository that isn't on GitHub it clears the column
// and does nothing else.
func refresh(notify bool) error {
	repo, ok := findRepo()
	if !ok {
		setDecorations(map[string]string{})
		if notify {
			tt("ui", "notify", "GitHub: origin isn't a GitHub remote")
		}
		return nil
	}
	saved, err := lookUp(repo)
	if err != nil {
		// While rate limited, events wait quietly (and keep the column)
		// instead of failing on every fetch; asking says why.
		if skipped(err) && !notify {
			fmt.Fprintln(os.Stderr, "github:", err)
			return nil
		}
		if notify {
			tt("ui", "error", "GitHub: "+err.Error())
		}
		return err
	}
	// Every PR's branch, not only the ones tt-git lists: on app.started the
	// branch list may not have loaded yet, and tt-git keeps text for
	// branches it doesn't list. Only a set too long for one command line
	// is cut down to the listed branches.
	decorations := map[string]string{}
	for branch, pr := range saved.PRs {
		decorations[branch] = Label(pr)
	}
	params, _ := json.Marshal(decorations)
	tooLong := len(params) > maxParams
	var listed map[string]string
	if tooLong || notify {
		names, err := branchNames()
		if err != nil {
			return err
		}
		listed = map[string]string{}
		for _, name := range names {
			if text, ok := decorations[name]; ok {
				listed[name] = text
			}
		}
	}
	if tooLong {
		decorations = listed
	}
	if err := setDecorations(decorations); err != nil {
		return err
	}
	if notify {
		tt("ui", "notify", fmt.Sprintf("GitHub: %d of your branches have an open PR", len(listed)))
	}
	return nil
}

// Windows caps a command line at 32767 characters; leave room for the rest.
const maxParams = 24000

// errSkipped wraps the rate limit error when lookUp didn't ask GitHub.
type errSkipped struct{ ErrRateLimited }

func (e errSkipped) Unwrap() error { return e.ErrRateLimited }

func skipped(err error) bool {
	var s errSkipped
	return errors.As(err, &s)
}

// lookUp fetches repo's open PRs and saves them for `open`. After GitHub
// says the rate limit is used up, it doesn't ask again until it resets.
func lookUp(repo Repo) (Saved, error) {
	saved := loadSaved()
	if saved.Repo == repo && time.Now().Before(saved.RateLimitedUntil) {
		return Saved{}, errSkipped{ErrRateLimited{Reset: saved.RateLimitedUntil}}
	}
	token, source := findToken(repo.Host)
	prs, err := Client{Token: token, TokenSource: source, APIBase: apiBase}.OpenPullRequests(repo)
	var limited ErrRateLimited
	if errors.As(err, &limited) {
		if saved.Repo != repo {
			saved = Saved{Repo: repo}
		}
		saved.RateLimitedUntil = limited.Reset
		saved.save()
	}
	if err != nil {
		return Saved{}, err
	}
	saved = Saved{Repo: repo, PRs: ByBranch(prs, repo)}
	saved.save()
	return saved, nil
}

// findToken finds a token for host: the gh CLI's, else the variables gh
// reads. It returns "" (ask without one) when there's none, and where the
// token came from.
func findToken(host string) (token, source string) {
	if t := ghToken(host); t != "" {
		return t, "gh auth token"
	}
	vars := []string{"GH_TOKEN", "GITHUB_TOKEN"}
	if host != "github.com" {
		vars = []string{"GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"}
	}
	for _, v := range vars {
		if t := strings.TrimSpace(os.Getenv(v)); t != "" {
			return t, v
		}
	}
	return "", ""
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
		tt("ui", "notify", "GitHub: select a branch first")
		return nil
	}
	repo, ok := findRepo()
	if !ok {
		tt("ui", "notify", "GitHub: origin isn't a GitHub remote")
		return nil
	}
	saved := loadSaved()
	pr, found := saved.PRs[branch]
	if !found || saved.Repo != repo {
		fresh, err := lookUp(repo)
		if err != nil {
			tt("ui", "error", "GitHub: "+err.Error())
			return err
		}
		pr, found = fresh.PRs[branch]
	}
	if !found {
		tt("ui", "notify", "GitHub: "+branch+" has no open pull request")
		return nil
	}
	return openBrowser(repo.PullRequestURL(pr.Number))
}

// enterpriseHosts are the GitHub Enterprise Server hosts the user named.
func enterpriseHosts() []string {
	hosts := strings.FieldsFunc(os.Getenv(hostsVar), func(r rune) bool { return r == ',' || r == ' ' })
	if h := strings.TrimSpace(os.Getenv("GH_HOST")); h != "" {
		hosts = append(hosts, h)
	}
	return hosts
}

// findRepo reads the GitHub repository from origin, else from the first
// remote that is on GitHub.
func findRepo() (Repo, bool) {
	dir := os.Getenv("TTGIT_REPO_PATH")
	git := func(args ...string) string {
		out, err := exec.Command("git", append([]string{"-C", dir}, args...)...).Output()
		if err != nil {
			return ""
		}
		return strings.TrimSpace(string(out))
	}
	hosts := enterpriseHosts()
	if repo, ok := ParseRemote(git("remote", "get-url", "origin"), hosts); ok {
		return repo, true
	}
	for _, name := range strings.Fields(git("remote")) {
		if repo, ok := ParseRemote(git("remote", "get-url", name), hosts); ok {
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
