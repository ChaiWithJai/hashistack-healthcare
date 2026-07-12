# Try the hosted preview

You can build a synthetic clinical tool without an account. The service gives
your browser a private workspace that lasts for 24 hours. Another browser
cannot list or change your apps.

## Safety limit

Use synthetic examples only. The hosted preview is not approved for patient
data or clinical care.

An anonymous workspace cannot export source to an unverified user. It also
cannot release an app to a patient data system. A guest can publish an isolated
synthetic preview after the release checks pass.

## Current URLs

| Environment | URL | Source | Status |
|---|---|---|---|
| Staging | [138-197-27-225.sslip.io](https://138-197-27-225.sslip.io) | DigitalOcean Droplet | Active |
| Pull request | See the bot comment on the pull request | Exact pull request commit on staging | Active after approval |
| Production | Not published | None | Pending |

## Test the main flow

1. Describe a small practice tool.
2. Choose a signed clinical starter.
3. Ask for one change.
4. Open the release gate.
5. Apply the suggested fix when a check fails.
6. Publish the isolated synthetic preview.
7. Select "Make this mine."
8. Sign in and export the app.

## Test accounts

Staging and production use separate Clerk instances and separate users. Ask the
project owner for the credential manager record named "Practice Studio staging
test owner." Do not put an email address, password, verification code, session
token, or Clerk subject in an issue.

The production smoke account is created only for an approved test window. It
uses a synthetic tenant and is removed after the test.

## Report a problem

Include the environment, pull request number, deployed commit, time, and failed
step. Do not include tokens or patient information.
