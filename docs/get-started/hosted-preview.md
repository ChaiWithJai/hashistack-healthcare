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
| Pull request | See the proof comment on the pull request | Exact pull request commit on staging | Operator deploy until Tunnel cutover |
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

The Practice Studio Clerk application has two isolated environments.
Development is for staging and production is for production. Create one test
owner in each environment. Map both users to the Rust `clinician` role and a
synthetic tenant. That role can claim and export an app. A staging session
cannot be used in production.

Clerk administrator access does not grant an application role. Practice Studio
does not have a cross tenant superadmin role. This keeps a test account from
reading another tenant by mistake.

Store the sign-in addresses and recovery method in the credential manager as:

- `Practice Studio staging superadmin`
- `Practice Studio production superadmin`

Do not commit passwords, verification codes, session tokens, Clerk subjects,
secret keys, or a subject map. The repository records the role and procedure;
the credential manager records the identities.

To provision or rotate an account:

1. In Clerk, open Practice Studio and switch to the target environment.
2. Create or invite the test user.
3. Create a Rust principal with role `clinician` and tenant `synthetic-test`.
4. Add the Clerk subject to that environment's secret `CLERK_SUBJECT_MAP` and
   map it to that principal. The service does not trust browser metadata for
   authorization.
5. Sign in through the deployed site, claim a guest workspace, and export it.
6. Remove the production account after the approved smoke-test window. The
   staging account may remain for pull-request verification.

The test identity is never used with patient data.

## Current delivery limit

The DigitalOcean firewall limits public SSH to the operator address. The
Cloudflare pipeline uses an Access-protected SSH hostname and does not open port
22 to GitHub runner addresses. Until the owned zone, tunnel, and service token
are configured, an authorized operator deploys the exact pull request commit
and posts the proof comment.

## Report a problem

Include the environment, pull request number, deployed commit, time, and failed
step. Do not include tokens or patient information.
