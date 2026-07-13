# Contributing

Keep each pull request tied to one user problem. Use an issue when the change
needs a product or architecture decision before code review.

## Set up the repository

Run the supported local proof from the repository root.

```bash
scripts/single-host-smoke.sh
```

Read [Run Practice Studio locally](docs/get-started/local.md) if the proof does
not pass.

## Make a change

Keep product rules in Rust. Keep the browser free of secrets and special
privileges. Use the current extension points for packs, model providers,
release checks, and deployment providers.

Update the documentation page that owns the changed behavior. Do not add a new
document when an existing tutorial, operations guide, reference page, or
decision record already owns the topic.

## Run the review checks

```bash
make check
scripts/docs-check.sh
```

Run the browser and exported application proof when you change the clinician
flow or export format.

```bash
scripts/journey.sh
```

## Open a pull request

Include this evidence:

* State the user problem.
* Name the exact commit.
* List the commands and links that prove the change.
* State the remaining limits.
* Link the Netlify deploy preview when the frontend changed.
* Link the DigitalOcean staging proof when the Rust service changed.

Follow the [merge standard](docs/process/merge-standard.md). Do not commit
screenshots, exported bundles, local state, credentials, or patient data.
