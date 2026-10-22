# Azure DevOps plugin for tt-git

Shows each branch's active Azure DevOps pull request (`!1423`) in tt-git's
branch list, and opens it in your browser.

```sh
tt-git plugin install ThirtyTech/tt-git/plugins/azure-devops
export ADO_MCP_AUTH_TOKEN=...   # an Entra access token or a PAT with Code (Read)
```

Installing downloads the prebuilt `tt-git-ado` for this version from the
`azure-devops-v<version>` GitHub release, so you don't need Go.

See [Azure DevOps](https://thirtytech.github.io/tt-git/plugins/azure-devops/)
on the doc site for the token, the remotes it understands and when it looks
pull requests up.

## Development

```sh
go build -o bin/tt-git-ado .   # bin/tt-git-ado.exe on Windows
go test ./...
tt-git plugin link .
```

To release, bump `version` in `tt-git-plugin.toml` and push the tag
`azure-devops-v<version>`. The *Azure DevOps plugin* workflow builds every
platform and attaches the binaries to the release.
