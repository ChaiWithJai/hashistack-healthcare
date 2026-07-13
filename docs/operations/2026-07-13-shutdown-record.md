# Shutdown record: July 13, 2026

## Scope

This record covers resources uniquely attributable to HashiStack Healthcare
Studio. Unrelated account resources were intentionally preserved.

## Deleted cloud resources

- DigitalOcean droplet `584090793`, `hashistack-healthcare-studio`
- DigitalOcean firewall `4024db86-764c-40b8-95d2-c17fde83c23d`
- DigitalOcean GenAI agent `9a0ba72a-7e58-11f1-aee4-4e013e2ddde4`
- Netlify site `8956042c-9a19-49f0-9a4c-31f808cff96e`, `gethoursback`
- Netlify site `ca0d2a60-bf58-4d63-845f-ed3eb37b2785`, `gethoursback-staging`

Deletion was verified by DigitalOcean `404 Not Found` responses, Netlify site
removal, failed DNS resolution for the staging hostname, and `404` responses
from the former Netlify subdomains.

## Billing statement

The last available DigitalOcean invoice preview attributed $0.28 to the project
droplet through 2026-07-13T00:00Z. Already-incurred charges cannot be removed,
and provider usage can post after deletion. The account showed $140.45 in
month-to-date usage, mostly from unrelated resources. Removing the droplet,
firewall, agent, and sites stops their continuing runtime use; it does not make
the whole DigitalOcean or Netlify account bill zero.

The project Netlify sites were not marked premium. The Netlify account itself is
a paid account with unrelated sites. The `gethoursback.com` registration
predates this project and was preserved because deleting or canceling a domain
is broader than tearing down this runtime.

## Preserved unrelated resources

- DigitalOcean droplet `539503047`, `writebook`
- DigitalOcean managed database `teachers-pet-valkey`
- the default DigitalOcean project and VPC
- the `gethoursback.com` domain registration and pre-existing DNS records
- unrelated Netlify sites

## Verification limits

The configured GCP account required interactive reauthentication, so an
account-side GCP inventory could not be completed. No GCP deployment state or
project-named resource was found in the repository. No Cloudflare credentials,
deployment configuration, or Cloudflare-hosted DNS was found. These are
recorded as “no evidence found,” not as verified zero-resource claims.

GitHub history, issues, PRs, Actions logs, and committed evidence are retained
inside the public archived repository as the durable audit trail.
