# The lvpm community index

This repository lists LabVIEW packages that [lvpm](https://github.com/ThatsTheEnd/lvpm) installs. Each package name has one file, `packages/<name>.toml`. That file names the GitHub repository that owns the name and lists each version's release file and SHA-256. The package files themselves stay on their owners' GitHub Releases.

A project reads the index only when it lists it in `lvpm.toml`:

```toml
[sources]
community = "github:labview-package-manager/package-index"
```

Source order matters: lvpm takes a package from the first source that lists it.

## How to register a package

Follow [Publish to the community index](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/usage.md#publish-to-the-community-index) in the lvpm docs. In short: your release workflow runs `lvpm index add` and opens a pull request here.

## The rules

The bot runs `lvpm index check` on every pull request. A name belongs to the first repository that registers it. A pull request may only add or change `.toml` files directly under `packages/`. It may add new versions to a name its own repository owns. It may not change or remove a listed version. Each new version's file must be a release file of the owner's repository, with the SHA-256 and package fields that the entry states. It must also carry a GitHub attestation from that repository. When every new version passes, the bot merges the pull request. Otherwise it comments with one line per refusal, each with a fix. Fix the cause, push a new tag, and close the refused pull request. A pull request that adds no version waits for a maintainer.

## For maintainers

- **Recover or move a name.** Open a pull request that changes `github` in `packages/<name>.toml`. The bot refuses it (rule R2). Review it by hand, then merge it past the bot.
- **Remove a name or a version** (for example a test registration). Do the same with a pull request that deletes the file or the `[[version]]` table.
- **The bot's lvpm.** `.github/workflows/check.yml` installs the lvpm release `LVPM_VERSION`, verified against its `SHA256SUMS`. While the repository variable `LVPM_GIT_REV` is set, it builds lvpm from that commit instead.
