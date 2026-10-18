# Example plugins

| Plugin | Shows |
|---|---|
| [open-on-github](open-on-github) | actions with `commit`/`branch` contexts; a link handler |
| [notify-on-push](notify-on-push) | event hooks calling back through `tt-git ui notify` |
| [pr-list](pr-list) | a popup pane, and an action that opens the pane through the CLI |

The examples need [Node.js](https://nodejs.org). `pr-list` also needs the
[GitHub CLI](https://cli.github.com). To try one:

```sh
tt-git plugin link examples/plugins/open-on-github
tt-git            # then press : and type "github"
```

See [docs/plugins.md](../../docs/plugins.md) for the full plugin reference.
