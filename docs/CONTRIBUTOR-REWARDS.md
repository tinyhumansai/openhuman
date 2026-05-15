# Contributor Rewards Automation

OpenHuman uses `.github/workflows/contributor-rewards.yml` to invite eligible
contributors into the Discord and merch reward flow.

## Triggers

The workflow runs when:

- a pull request is merged and the PR author has no earlier merged PR in this
  repository;
- a maintainer applies the `reward user` label to a pull request;
- a maintainer applies the `reward user` label to an issue;
- a maintainer runs the workflow manually to bootstrap the labels.

The workflow creates these labels if they do not already exist:

- `reward user` - maintainer-triggered reward invite;
- `reward sent` - audit label added after the invite comment is posted.

## Idempotency

Each reward comment includes a hidden marker scoped to the GitHub login:

```html
<!-- openhuman-contributor-reward:user=<login> -->
```

Automatic first-merged-PR rewards are skipped when the same login already has a
reward marker elsewhere in the repository. Maintainers can intentionally start
the flow again by applying `reward user`, but the workflow still skips a target
issue or PR that already contains the same marker.

Bot accounts are skipped.

## Configuration

Configure these repository variables under
**Settings -> Secrets and variables -> Actions -> Variables**:

| Variable                         | Required | Purpose                                                                  |
| -------------------------------- | -------- | ------------------------------------------------------------------------ |
| `CONTRIBUTOR_REWARD_DISCORD_URL` | No       | Public Discord invite URL. Defaults to `https://discord.tinyhumans.ai/`. |
| `CONTRIBUTOR_REWARD_MERCH_URL`   | No       | Public merch claim or redemption URL included in the comment.            |
| `CONTRIBUTOR_REWARD_MESSAGE`     | No       | Full custom comment body. Supports tokens listed below.                  |

`CONTRIBUTOR_REWARD_MESSAGE` can use these tokens:

- `{user}` - GitHub mention, for example `@octocat`;
- `{login}` - raw GitHub login;
- `{discord_url}` - configured Discord URL;
- `{merch_url}` - configured merch URL or an empty string;
- `{reason}` - trigger reason;
- `{target_url}` - issue or PR URL.

Do not put private Discord invite mechanics, shipping forms, access tokens, or
other secrets in repository variables used by this workflow. Anything rendered
by the workflow is posted as a public GitHub comment.

## Security Model

The workflow uses `pull_request_target` so it can comment on pull requests from
forks. It must not check out or execute pull request code. The current workflow
only reads the GitHub event payload and calls GitHub APIs through
`actions/github-script`.

Required permissions are limited to:

- `contents: read`;
- `issues: write`;
- `pull-requests: read`.

## Manual Operation

To reward a contributor manually:

1. Open the issue or pull request.
2. Apply the `reward user` label.
3. Wait for the workflow to post the reward comment and add `reward sent`.

If the labels do not exist yet, run **Actions -> Contributor Rewards -> Run
workflow** once on `main`.
