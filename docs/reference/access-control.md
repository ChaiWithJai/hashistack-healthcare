# Access control reference

## Public routes

| Route | Access |
|---|---|
| `/` | Public product interface. |
| `/health` | Public service health. |
| `/auth/config` | Public auth mode and Clerk publishable key. |
| `/api/packs` | Public clinical starter list. |
| `/api/public/session` | Starts or resumes a signed anonymous workspace. |

## Workspace routes

A workspace route accepts a verified Clerk user or a valid anonymous workspace
cookie. The service derives a private tenant from the signed cookie. It does
not store the raw cookie in the app record or audit log.

Workspace routes cover app creation, app changes, restore, review, release
checks, app audit, synthetic preview publication, preview status, and rollback.
Guests can use synthetic data only. A guest cannot publish to the production
pool.

## Owner routes

These routes require a verified owner:

| Route | Rule |
|---|---|
| `/api/apps/:id/claim` | Requires Clerk and the original anonymous workspace cookie. |
| `/api/apps/:id/export` | Requires Clerk and same tenant ownership. |
| `/api/apps/:id/operations` | Requires Clerk and same tenant ownership. |
| `/api/audit/export` | Requires Clerk and the clinician capability. |

Claiming changes the app tenant from the temporary workspace tenant to the
verified user's tenant. The service saves that change before it allows export.
If the durable save fails, the service restores the temporary owner and
returns HTTP 503.

## Roles

| Role | Scope |
|---|---|
| Guest | One temporary synthetic workspace. No export capability. |
| Staff | One tenant. Cannot approve a clinical release or export the platform audit record. |
| Clinician | One tenant. Can approve releases and export the platform audit record. |

There is no application superadmin role. Clerk administrator status does not
change this table.

## Failure responses

| Status | Meaning |
|---|---|
| 401 | The session or anonymous workspace is missing, invalid, or expired. |
| 403 | The user is known but lacks the required capability. |
| 404 | The app does not exist in the caller's tenant. |
| 409 | The app state or release checks do not allow the requested change. |
| 503 | The service could not save the required durable audit or state record. |

## Exported data

An app bundle includes the tenant's app record and app audit details. The
platform audit export keeps doctor written text in keyed hash form. Hosted
preview use remains limited to synthetic examples.
