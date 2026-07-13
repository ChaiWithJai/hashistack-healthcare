# Operate Clerk sign in

Clerk verifies the session. The Rust principal directory assigns the tenant and
application role. A Clerk dashboard administrator has no application role by
default.

The application currently has three roles. A guest owns a temporary synthetic
workspace. A staff user can build and operate tools in one tenant. A clinician
can also approve a release and export the platform audit record. There is no
standing cross tenant superadmin role.

## Keep the environments separate

Use one Clerk instance for staging and another Clerk instance for production.
Each instance needs its own issuer, publishable key, browser origins, and
subject map. Do not reuse production users in staging.

The service reads these settings:

| Setting | Purpose |
|---|---|
| `CLERK_PUBLISHABLE_KEY` | Loads the Clerk sign in form in the browser. |
| `CLERK_ISSUER` | Names the trusted token issuer. It must use HTTPS. |
| `CLERK_JWKS_URL` | Supplies the public signing keys. It must use HTTPS. |
| `CLERK_AUTHORIZED_PARTIES` | Lists each allowed browser origin exactly. |
| `CLERK_SUBJECT_MAP` | Maps a Clerk subject to a Rust principal. |
| `ANON_SESSION_HMAC_KEY` | Signs temporary workspace cookies. Use at least 32 random bytes. |

The service does not need a Clerk secret key at runtime.

Use these operator names in the credential manager:

- `Practice Studio staging superadmin`
- `Practice Studio production superadmin`

“Superadmin” is the operator-facing name for these test identities. It is not
an application role. Both map to the existing tenant-scoped `clinician` role,
which is sufficient to claim and export. Neither can list another tenant.

The repository stores no email address, password, verification code, session
token, Clerk subject, or subject map value.

## Environment configuration

Keep these as protected deployment settings. Replace every angle-bracketed
placeholder in the deployment secret store. Do not put the resolved values in
this repository or a pull request.

| Setting | Staging | Production |
|---|---|---|
| Clerk environment | Development instance | Production instance |
| Credential-manager record | `Practice Studio staging superadmin` | `Practice Studio production superadmin` |
| Test mailbox | `<STAGING_SUPERADMIN_EMAIL>` | `<PRODUCTION_SUPERADMIN_EMAIL>` |
| Rust principal | `staging-test-owner` | `production-smoke-owner` |
| Application role | `clinician` | `clinician` |
| Tenant | `staging-test` | `production-test` |
| Subject map | `<STAGING_CLERK_SUBJECT>=staging-test-owner` | `<PRODUCTION_CLERK_SUBJECT>=production-smoke-owner` |
| Allowed browser origin | `<STAGING_HTTPS_ORIGIN>` | `<PRODUCTION_HTTPS_ORIGIN>` |

The protected staging deployment receives:

```text
CLERK_PUBLISHABLE_KEY=<STAGING_CLERK_PUBLISHABLE_KEY>
CLERK_ISSUER=<STAGING_CLERK_HTTPS_ISSUER>
CLERK_JWKS_URL=<STAGING_CLERK_HTTPS_JWKS_URL>
CLERK_AUTHORIZED_PARTIES=<STAGING_HTTPS_ORIGIN>
CLERK_SUBJECT_MAP=<STAGING_CLERK_SUBJECT>=staging-test-owner
ANON_SESSION_HMAC_KEY=<STAGING_RANDOM_VALUE_AT_LEAST_32_BYTES>
```

The protected production deployment receives a different set:

```text
CLERK_PUBLISHABLE_KEY=<PRODUCTION_CLERK_PUBLISHABLE_KEY>
CLERK_ISSUER=<PRODUCTION_CLERK_HTTPS_ISSUER>
CLERK_JWKS_URL=<PRODUCTION_CLERK_HTTPS_JWKS_URL>
CLERK_AUTHORIZED_PARTIES=<PRODUCTION_HTTPS_ORIGIN>
CLERK_SUBJECT_MAP=<PRODUCTION_CLERK_SUBJECT>=production-smoke-owner
ANON_SESSION_HMAC_KEY=<PRODUCTION_RANDOM_VALUE_AT_LEAST_32_BYTES>
```

Never reuse an issuer, subject, browser origin, session-HMAC key, or test
mailbox across the two environments.

## Create the staging superadmin test user

1. Use a Clerk development instance.
2. Create a dedicated Clerk test user. Use a test address that your team owns.
3. Create an application principal with the clinician role and a synthetic
   test tenant.
4. Sign in once and copy the Clerk subject from the Clerk dashboard.
5. Add the subject and principal pair to `CLERK_SUBJECT_MAP` in the protected
   staging environment settings.
6. Remove `CLERK_DEVELOPMENT_DEFAULT_PRINCIPAL` after the explicit subject map
   works.
7. Revoke the first session and confirm that a new session can claim and export
   a synthetic app.

Clerk development test addresses can use the reserved `+clerk_test` form and
the Clerk test verification code. Use this only in the staging development
instance.

## Create the production superadmin test user

Do not enable Clerk test mode in production. Create a temporary user with a
real team mailbox and multifactor authentication. Map that user to a synthetic
tenant with the clinician role.

Create the user for an approved test window. Revoke its sessions and remove its
subject map after the smoke test. Do not store its password in Git or in a
continuous integration secret.

If the product needs a platform administrator later, add a new application
role with narrow capabilities and tests first. Do not treat a Clerk
organization administrator as a platform administrator.

## Rotate or remove a test owner

Remove the subject from `CLERK_SUBJECT_MAP`, deploy the change, and revoke all
active Clerk sessions for that user. Confirm that the old session receives
HTTP 401. Record the change through the normal release process.

## Verify a deployment

Confirm each result:

1. The health route returns HTTP 200 without a session.
2. The auth config exposes only the auth mode and publishable key.
3. A browser can create an isolated anonymous workspace.
4. A second workspace receives HTTP 404 for the first workspace's app.
5. A guest can build, change, fix, and publish a synthetic preview.
6. Export asks the guest to sign in.
7. A verified user can claim and export the same app.
8. A user from another tenant receives HTTP 404.
9. Sign out removes access to protected exports.

The remote shell proof checks the public boundary. A browser run is required
for the signed in claim and export steps.
