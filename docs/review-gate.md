# Required CodeRabbit reviews

CodeRabbit's built-in `CodeRabbit` check considers skipped and rate-limited
reviews passing. The `CodeRabbit reviewed` check requires an actual completed
review and CodeRabbit approval on each PR's latest commit. Requested changes
block merging until CodeRabbit approves, unless a person with repository write
access records an explicit override for that commit. An ordinary human approval
does not automatically bypass the requirement.

The gate accepts only the newest `CodeRabbit` check from the CodeRabbit app
(ID 347564), with conclusion `success`, title `Finished`, and a summary beginning
`Result: Success - Review completed`. These fields were checked against PR #221
and CodeRabbit's [status-check documentation](https://docs.coderabbit.ai/pr-reviews/status-check)
on November 10, 2026. Unknown result formats block merging until reviewed and
updated deliberately. The newest CodeRabbit review must also be `APPROVED` on
the same commit. Automatic Approvals is enabled in `.coderabbit.yaml`, with
author approval commands disabled. CodeRabbit's [approval requirements](https://docs.coderabbit.ai/pr-reviews/automatic-approvals)
include a completed review and resolved required threads. Commit statuses are
not accepted; keep CodeRabbit's `reviews.review_progress` enabled so it
publishes check runs.

## Enable enforcement

Merge the workflow first so it exists on the default branch. An administrator
then adds `CodeRabbit reviewed` (source: GitHub Actions) to
**Require status checks to pass** in the
[main merge queue ruleset](https://github.com/tangency-labs/pcbcraft/rules/26960373),
preserving `checks`, `web` and `licenses`. Until those requirements are saved,
this workflow alone does not prevent merging. Keep the ruleset active and review
its bypass list if the requirement must also apply to administrators. This gate
already checks CodeRabbit's status. Requiring the built-in `CodeRabbit` check
separately can prevent the deliberate override from working when that check is
failed or cancelled; leave it informational if the scoped override should work.

Use **Actions → CodeRabbit review gate → Run workflow** with a PR number to
initialize the gate for PRs already open when it is deployed. Select the default
branch. Check the published `CodeRabbit reviewed` result before enqueueing a PR;
GitHub offers a recent check for selection in the ruleset once it has run.

## Failed, skipped or rate-limited reviews

The gate waits up to 25 minutes while a review is missing or running. Skipped,
rate-limited, cancelled and failed reviews produce a failing gate immediately,
even when CodeRabbit itself reports neutral or success. A CodeRabbit request for
changes also fails the gate. A review without an approving decision remains
pending, then fails after the timeout. API errors also block the gate.
A cancelled observer can leave the check pending, which still prevents
merging when it is required.

Read CodeRabbit's result, resolve its cause, and request another review. Use
`@coderabbitai rate limit` to inspect availability after a rate limit, or
`@coderabbitai full review` to retry a failed review on the same commit. Once the
review completes successfully and CodeRabbit approves, rerun the gate's Actions
run or dispatch it with the PR number and an empty override reason. Resolving
threads without pushing does not itself rerun this observer. A new commit
automatically starts a new gate for the new head.

## Explicit override

For a false positive, outage or other deliberate exception, a person with
repository write access can run **Actions → CodeRabbit review gate → Run
workflow** on the default branch. Supply the PR number and a nonempty
**override_reason** (at most 2,000 characters). The workflow verifies the
triggering account's current write permission and that it is a human account.
It publishes `CodeRabbit review override` with the reason, username, PR number,
commit and a link to the Actions run. The normal required gate then passes for
that exact PR head, and the merge queue honors the same record. Do not make the
override record itself a required check.

The override deliberately waives missing, skipped, failed or rate-limited
CodeRabbit reviews and requested changes. It preserves `checks`, `web`,
`licenses` and any other repository requirements. It persists for that commit
but does not cover a new commit or another PR. Ordinary approval and comments
containing override text cannot grant it.

The gate runs again on a merge group, resolves the queue entry whose temporary
head matches that group, and verifies original PR reviews/approvals or scoped
overrides for that entry and its predecessors. For the existing MERGE queue, it
also confirms that each reviewed PR head is an ancestor of the synthetic commit.
The synthetic CodeRabbit queue result (`Pull request review passed`) never
substitutes for a real PR review. A changed queue or PR head
blocks the old gate; GitHub's replacement group runs its own check. Live queue
behavior must be confirmed after deployment before relying on enforcement.

The observer checks out only the repository's trusted default branch, including
for fork PRs and merge groups. It never checks out or executes a contributor's
head with its check-writing token. Requests have 15-second timeouts, and the job
has a 30-minute limit. Regression tests use original, synthetic API records:

```sh
python3 -B -m unittest discover -s scripts -p 'test_coderabbit_review_gate.py'
```
