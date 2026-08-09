# Note67 sync service — design

| | |
|---|---|
| Status | Proposal, nothing built |
| Date | 2026-08-09 |
| Decides | D6 from [`../exochain/DECISIONS.md`](../exochain/DECISIONS.md), expanded to multi-user |
| Endpoint | `note67.jtpa.net` |

Archive, multi-device, and a downstream feed, for a team rather than one
person, with shareable transcripts.

## 1. Why a service, and not a database connection

D6 originally said "SQLite primary, Postgres mirror", which implied clients
connecting to Postgres directly. The requirements that arrived since rule that
out:

**OAuth cannot authenticate to Postgres.** Postgres speaks SCRAM, certificates
and LDAP; it does not accept bearer tokens. Entra-authenticated Postgres exists
only as a managed Azure service. Choosing MSAL chose an HTTP service.

**A shared database has no usable authorization boundary.** Any client holding
credentials can read and write every row belonging to every user. Row-level
security could express the rules, but mapping an Entra identity onto a database
role from a desktop client is not a thing worth building.

**Two-way sync is server logic.** Change feeds, per-device cursors, conflict
rules and tombstones have to be implemented identically by every client, and one
client at the wrong version corrupts state for the rest.

**Schema ownership.** Several installs on different versions racing DDL against
one database is a bad outcome. The service owns migrations.

So: clients talk HTTPS to `note67.jtpa.net`; Postgres is private behind it.

## 2. What the server must verify

The server is not a dumb store. If a client submits a transcript version whose
`content_hash` does not match the segments alongside it, the chain attests
nothing — a bug or a bad actor could anchor a hash to content it never
described, and every downstream verification would agree with itself while being
wrong.

**The server recomputes the canonical form and rejects mismatches.** That
requires the byte-exact serialization the app uses (`note67.transcript.v1`), so
the two must share code rather than each implement the spec. Reimplementing it
server-side would reintroduce precisely the divergence the frozen format exists
to prevent.

Consequence: **the service is Rust**, depending on the same crate the app does.
The canonical form moves out of `src-tauri/src/exochain/transcript.rs` into a
small shared crate; the service takes it as a git dependency on this fork, so
there is one definition and no parity to maintain.

## 3. Identity

**Entra identifies the user. The install's DID identifies the device.**

- Desktop app is an MSAL **public client** using authorization code with PKCE:
  system browser, loopback redirect, no client secret in the binary.
- The service validates the JWT against Entra's JWKS — signature, `aud`, `iss`,
  `tid`, and an app role.
- Refresh tokens live in the **OS keychain** (Windows Credential Manager, macOS
  Keychain), not the settings database. The AI provider's API key is stored in
  settings because it reaches one model server on a trusted network; this
  credential reaches every transcript its owner holds.
- The device's DID (per-install, from `exochain::identity`) registers against
  the user. That gives a device list, and lets one machine be revoked without
  disturbing the others.

## 4. Tenancy, ownership and sharing

- **Tenant** = the Entra tenant. Users never cross tenants.
- **Owner** = the user whose device created the note.
- **Shares** are explicit rows: `(note_id, principal, role)`, where a principal
  is an Entra user or group, and role is `viewer` or `editor`.

Entra groups as principals give team-scale sharing without building a workspace
model. A workspace layer can be added later; explicit shares cannot be removed
later, so they are the safer starting point.

Authorization, evaluated server-side on every request:

| Action | Who |
|---|---|
| Read a note and its transcript | owner, or any share |
| Append a transcript version | owner, or an `editor` share |
| Edit note body, tasks, tags | owner, or an `editor` share |
| Share, or change roles | owner |
| Delete from archive | owner |

**Revoking a share does not unshare what was already read.** It stops future
access. Anything the person has already seen, exported or copied is beyond
recall, and the UI must not imply otherwise.

## 5. Sync protocol

Every mutation gets a monotonic `seq` from a server sequence. Clients hold a
cursor per device.

```
GET  /v1/changes?since=<seq>&limit=<n>     → ordered changes the caller may see
POST /v1/changes                           → submit local changes, get assigned seqs
```

Changes carry the entity kind, its id, a payload, and a tombstone flag.
Idempotent by `(device_id, client_change_id)` so a retry after a dropped
connection cannot double-apply.

Conflicts are **last-writer-wins per record**, by `updated_at` with the device
DID as a tiebreak — except the transcript chain.

### The chain cannot use last-writer-wins

If two devices each append a v2 to the same note, LWW silently discards one and
leaves two machines each holding a chain that verifies locally while disagreeing
about history. That is the exact failure the chain exists to make impossible.

**The server holds `UNIQUE (note_id, version)`.** The second v2 is rejected with
the winning version's hash, and that client **re-bases**: it pulls the winner and
re-appends its own content as v3. Nothing is lost — the losing edit moves later
in the history rather than disappearing — and the chain stays linear and
verifiable.

Each version also records **who appended it**: the author's user id and device
DID. In a team that is most of the value — the history says which colleague
re-transcribed or corrected a meeting, not merely that it changed.

## 6. Deletes

Two operations, deliberately distinct:

- **Delete locally** — removes the note from this device and records a local
  suppression so sync does not pull it back. The archive is untouched, and other
  devices are unaffected. Worth labelling "Remove from this device", because a
  delete that resurrects on next sync would be alarming.
- **Delete from archive** — owner only. Writes a tombstone that propagates to
  every device and removes content server-side.

## 7. Receipts

Two egress events deserve attestation, and both reuse the existing shape rather
than inventing one:

- **Mirroring a transcript version** to the archive. Content crossing a boundary
  to a named endpoint is the same shape as a model call, so it can ride LYNK
  with `custody_mode: ReceiptMinimized`.
- **Sharing** a note with a principal. This is the most human-meaningful receipt
  in the system: *this transcript was shared with this person at this time.*

Both anchor to a transcript version rather than to a sync run, so a receipt
always names exactly which content moved.

## 8. Confidentiality posture, stated plainly

The sync host can read every transcript it stores. End-to-end encryption is
**ruled out by the downstream-feed requirement**: if another system reads
transcripts out of Postgres for memory, the data cannot be encrypted to keys the
server lacks.

So the posture is: TLS required in transit, at-rest encryption at the volume
level, authorization enforced server-side, and the host treated as trusted
infrastructure. That is a reasonable trade for a homelab a team already relies
on — but it is a trade, and the app should not claim otherwise.

## 9. Shape

```
Desktop app  ──HTTPS + Entra JWT──▶  note67.jtpa.net (Traefik)
                                            │
                                     note67-sync (Rust/axum)
                                            │
                                     Postgres (private)
```

Deployed on the Fitz Swarm, the same pattern already running for Hindsight MCP.
A separate repository from the app, so upstream merges stay clean; the shared
canonical-form crate is the only coupling.

## 10. Open questions

1. **Does the app keep working signed out?** It should — recording is local and
   must not need a token. But then notes exist before an owner does, and get
   attributed on first sign-in. Simplest is to require sign-in only for sync.
2. **Group membership resolution.** Reading Entra groups from a token's claims
   is limited when a user has many; the alternative is Graph lookups server-side,
   which needs its own app permissions.
3. **Attachment of images.** Note bodies can embed images. Those are files, not
   rows, and need either object storage or exclusion.
4. **Retention.** An archive that never forgets is a liability as well as a
   feature. Worth a policy before there is a year of meetings in it.
