// Runs inside a tt-git pane (a real PTY): print, then wait for a key.
import { execFileSync } from "node:child_process";

const ctx = JSON.parse(process.env.TTGIT_PLUGIN_CONTEXT_JSON ?? "{}");
const branch = process.env.PR_BRANCH;
const args = ["pr", "list", "--limit", "30"];
if (branch) args.push("--head", branch);

console.log(`\x1b[1mOpen pull requests${branch ? ` for ${branch}` : ""}\x1b[0m\n`);
try {
  const out = execFileSync("gh", args, { cwd: ctx.repo_path, encoding: "utf8" });
  console.log(out.trim() || "(none)");
} catch (e) {
  console.log(`could not run gh: ${e.message}`);
}
console.log("\n\x1b[2mpress any key to close\x1b[0m");

process.stdin.setRawMode?.(true);
process.stdin.once("data", () => process.exit(0));
