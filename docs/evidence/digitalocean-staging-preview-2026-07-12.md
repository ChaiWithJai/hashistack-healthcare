# DigitalOcean staging preview proof

The initial release merged through PR #26 as commit
`862e4f69b4954214a7d48ee0d8b1e874f49ad8cf`. That exact commit was deployed to
the synthetic DigitalOcean staging service at
`https://138-197-27-225.sslip.io`.

The provider-neutral remote proof passed after deployment. It created a
synthetic post-op app, repaired the deliberately failing auto-logoff gate,
refused a real-data promotion, published only to the `synthetic-demo` pool,
reported unavailable runtime telemetry honestly, and exported owned Rust
source and deploy manifests.

The browser test covered login, starter selection, app creation, the blocked
release, the suggested fix, the clinician signature, the synthetic release,
and the audit record. Screenshots belong to the PR or the workflow run. They
are not stored in Git.

## Repeatable PR preview

The `staging-preview` workflow accepts an open PR number. It resolves the
current head commit from GitHub and waits for approval through the `staging`
environment. It deploys that exact commit, runs the remote proof, and comments
the preview link on the same PR. GitHub stores the deployment key, host, host
key, and development token as environment secrets.

This is synthetic staging. It is not approved for patient data.
