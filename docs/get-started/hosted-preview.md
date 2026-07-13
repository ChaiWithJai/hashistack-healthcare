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
| Pull request | Open the Netlify `Deploy Preview` check | Exact pull request frontend commit | Automatic |
| Production | Not published | None | Pending |

Netlify gives each pull request its own frontend URL. The static studio uses
the same-origin proxy rules in `netlify.toml` to reach the Rust API on
DigitalOcean. A deploy preview does not replace staging or production.
The DigitalOcean service must set `ANON_NETLIFY_PREVIEW_SITE=gethoursback` so
numbered previews can create anonymous synthetic workspaces. This setting does
not authorize Clerk claim or export on a preview host.

## Test the main flow

1. Describe a small practice tool.
2. Choose a signed clinical starter.
3. Compare the proposed treatments and select one.
4. Review the generated source diff and executable verification results.
5. Accept the candidate, then ask for another change if needed.
6. Open the release gate and apply a suggested fix when a check fails.
7. Publish the isolated synthetic preview.
8. Select "Make this mine." This is the first step that asks for sign in.
9. Sign in and export the app.

## Test accounts

The Practice Studio Clerk application has two isolated environments.
Development is for staging and Production is for production. Create one
superadmin test user in each environment. Map both users to the Rust
`clinician` role and a
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

The exact placeholder configuration and rotation procedure are in
[Operate Clerk sign in](../operations/clerk.md). “Superadmin” is the test-user
label, not a cross-tenant application capability.

## Current delivery limit

The DigitalOcean firewall limits public SSH to the operator address. The
Cloudflare pipeline uses an Access-protected SSH hostname and does not open port
22 to GitHub runner addresses. Until the owned zone, tunnel, and service token
are configured, Netlify proves the pull request frontend automatically. An
authorized operator still deploys the exact Rust commit to DigitalOcean and
posts the backend proof comment.

## Report a problem

Include the environment, pull request number, deployed commit, time, and failed
step. Do not include tokens or patient information.
