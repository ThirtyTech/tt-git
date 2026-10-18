// Opens the selected commit/branch (or a clicked URL) on GitHub.
// Everything comes from tt-git's environment; no SDK needed.
import { execFileSync, spawn } from "node:child_process";

const ctx = JSON.parse(process.env.TTGIT_PLUGIN_CONTEXT_JSON ?? "{}");
const bin = process.env.TTGIT_BIN_PATH ?? "tt-git";
const notify = (msg) => execFileSync(bin, ["ui", "notify", msg]);

function repoUrl() {
  const remote = execFileSync("git", ["remote", "get-url", "origin"], { cwd: ctx.repo_path })
    .toString()
    .trim();
  // git@github.com:owner/repo.git or https://github.com/owner/repo(.git)
  const m = remote.match(/github\.com[:/](.+?)(\.git)?$/);
  if (!m) throw new Error(`origin is not a GitHub remote: ${remote}`);
  return `https://github.com/${m[1]}`;
}

function open(url) {
  const [cmd, args] =
    process.platform === "win32" ? ["cmd", ["/c", "start", "", url]]
    : process.platform === "darwin" ? ["open", [url]]
    : ["xdg-open", [url]];
  spawn(cmd, args, { detached: true, stdio: "ignore" }).unref();
}

try {
  const what = process.argv[2];
  let url;
  if (what === "commit" && ctx.selected_commit) url = `${repoUrl()}/commit/${ctx.selected_commit}`;
  else if (what === "branch" && ctx.selected_branch) url = `${repoUrl()}/tree/${ctx.selected_branch}`;
  else if (what === "url") url = process.env.TTGIT_PLUGIN_CLICKED_URL;
  if (!url) throw new Error("nothing selected");
  open(url);
  notify(`opened ${url}`);
} catch (e) {
  notify(`open-on-github: ${e.message}`);
  process.exit(1);
}
