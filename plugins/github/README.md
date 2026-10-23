# GitHub plugin for tt-git

Shows each branch's open GitHub pull request (`#1423`) in tt-git's branch
list, and opens it in your browser.

```sh
tt-git plugin install ThirtyTech/tt-git/plugins/github
gh auth login   # or export GH_TOKEN=...; public repositories work without either
```

Installing downloads the prebuilt `tt-git-github` for this version from the
`github-v<version>` GitHub release, so you don't need Go.

See [GitHub](https://thirtytech.github.io/tt-git/plugins/github/) on the doc
site for the token, the remotes it understands (including GitHub Enterprise
Server) and when it looks pull requests up.

## Development

```sh
go build -o bin/tt-git-github .   # bin/tt-git-github.exe on Windows
go test ./...
tt-git plugin link .
```

To release, bump `version` in `tt-git-plugin.toml` and push the tag
`github-v<version>`. The *GitHub plugin* workflow builds every platform and
attaches the binaries to the release.
