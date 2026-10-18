// Event hook: tt-git sets TTGIT_PLUGIN_EVENT_JSON to {"event": ..., "data": ...}.
import { execFileSync } from "node:child_process";

const { event, data } = JSON.parse(process.env.TTGIT_PLUGIN_EVENT_JSON ?? "{}");
const bin = process.env.TTGIT_BIN_PATH ?? "tt-git";

const message =
  event === "push.completed" ? `🚀 pushed ${data.branch ?? "HEAD"}`
  : event === "commit.created" ? `✔ committed ${String(data.oid ?? "").slice(0, 7)} ${data.summary ?? ""}`
  : `event ${event}`;

execFileSync(bin, ["ui", "notify", message]);
