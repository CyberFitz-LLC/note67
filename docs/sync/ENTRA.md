# Entra registrations

Created 2026-08-09 in the **CyberFitz Consulting LLC** tenant
(`aaa2d5a9-514c-4c48-9836-31bd0e93b619`, default domain `cyberfitz.org`).

Two registrations rather than one. The desktop app must be a **public client** —
anything shipped in the binary is public, so it can hold no secret — while the
service needs a confidential identity to call Graph. One registration cannot be
both without giving the desktop app a secret it must not have.

## Note67 Sync API — the service

| | |
|---|---|
| Application (client) ID | `ac2463be-ae20-4347-bda9-8e7e65ecb42c` |
| Application ID URI | `api://ac2463be-ae20-4347-bda9-8e7e65ecb42c` |
| Sign-in audience | Single tenant |
| Access token version | 2 |

**Exposed scope** — `Sync.Access` (id `07d0a668-e0e3-4f46-8ebc-3d6fe2bfd3f0`), delegated, user-consentable.
The service requires this scope in every token it accepts.

**Graph application permission** — `GroupMember.Read.All`, admin consent
granted. This is what resolves group membership server-side: token claims
truncate once a user belongs to many groups, and a sharing rule that silently
stops applying to exactly the best-connected people is the worst kind of bug.

**Client secret** — "note67-sync service", expires **2028-08-09**. In
VaultWarden as *Note67 Sync — Entra app registrations (CyberFitz Consulting
LLC)*, alongside the tenant and client IDs so the service's whole environment
is in one place. Never displayed; written to a file on creation, verified
against the vault copy by hash, then shredded.

Set a reminder well before expiry. When it lapses, group resolution fails while
sign-in and sync keep working — a partial failure that looks like a sharing bug
rather than an expired credential, and one nobody diagnoses quickly cold.

## Note67 Desktop — the app

| | |
|---|---|
| Application (client) ID | `919ea6b8-6e30-4978-8f0d-295542a0b9e0` |
| Sign-in audience | Single tenant |
| Public client | yes (`isFallbackPublicClient`) |
| Redirect URI | `http://localhost` |

Authorization code with PKCE, system browser, loopback redirect. MSAL picks an
ephemeral port under `http://localhost`, which Entra permits for public clients
without registering each port.

**Pre-authorized** for `Sync.Access`, so signing in does not raise a consent
prompt for a first-party app the tenant already trusts.

## What the app needs at runtime

```
tenant   aaa2d5a9-514c-4c48-9836-31bd0e93b619
client   919ea6b8-6e30-4978-8f0d-295542a0b9e0
scope    api://ac2463be-ae20-4347-bda9-8e7e65ecb42c/Sync.Access
```

None of that is secret; it ships in the binary.

## What the service needs

```
AZURE_TENANT_ID       aaa2d5a9-514c-4c48-9836-31bd0e93b619
AZURE_CLIENT_ID       ac2463be-ae20-4347-bda9-8e7e65ecb42c
AZURE_CLIENT_SECRET   (VaultWarden)
EXPECTED_AUDIENCE     api://ac2463be-ae20-4347-bda9-8e7e65ecb42c
```

The service validates every bearer token against Entra's JWKS: signature,
`iss`, `aud`, `tid`, expiry, and the presence of `Sync.Access` in `scp`.

## Not done yet

- **No app roles.** Every authenticated user of the tenant can reach the API and
  owns whatever they create. An admin role, for tenant-wide administration,
  is a later addition.
- **Nothing restricts who may sign in.** If the API should be limited to a
  subset of the tenant, that is "user assignment required" on the service
  principal plus an app role — deliberately not set, since turning it on before
  there is a role would lock everyone out.
