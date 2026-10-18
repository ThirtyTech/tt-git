// An action that opens this plugin's pane for the selected branch.
import { execFileSync } from "node:child_process";

const ctx = JSON.parse(process.env.TTGIT_PLUGIN_CONTEXT_JSON ?? "{}");
execFileSync(process.env.TTGIT_BIN_PATH ?? "tt-git", [
  "plugin", "pane", "open",
  "--plugin", process.env.TTGIT_PLUGIN_ID,
  "--entrypoint", "list",
  "--env", `PR_BRANCH=${ctx.selected_branch ?? ""}`,
]);
