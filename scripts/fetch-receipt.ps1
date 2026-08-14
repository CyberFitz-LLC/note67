<#
.SYNOPSIS
    Fetch an ExoChain trust receipt and check what it actually attests.

.DESCRIPTION
    A developer tool, deliberately not a feature of the app.

    Reading a receipt back needs the node's ADMIN token — the same credential
    that authorises issuing, revoking and delegating credentials. That does not
    belong in a desktop application installed on end-user machines, so the app
    verifies transcripts locally (which needs nothing) and this script is how a
    developer inspects the node's side.

    The token is never echoed, never written to disk, and never passed on the
    command line where it would land in shell history.

.EXAMPLE
    .\scripts\fetch-receipt.ps1 -Hash dd12b56d5ee9c10f6391e3fd4a49cd28a588ad87b4f88d3b3afaa95e185d4c67

.EXAMPLE
    # Also prove the receipt is about the meeting you think it is.
    .\scripts\fetch-receipt.ps1 -Hash dd12b56d... -NoteId 7f3a1c2e-...

.NOTES
    Token: VaultWarden holds it. Set it for the session with
        $env:EXOCHAIN_ADMIN_TOKEN = (Read-Host -AsSecureString | ConvertFrom-SecureString -AsPlainText)
    or just run the script and paste it at the prompt.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Hash,

    # The note the receipt should be about. Given, the script recomputes the
    # action id and compares — which is what turns "a receipt exists" into
    # "a receipt exists for this meeting".
    [string]$NoteId,

    [string]$NodeUrl = "https://exochain-production.up.railway.app",

    # The full JSON. Off by default: an RFC-3161 token is several kilobytes of
    # base64 and drowns everything worth reading.
    [switch]$Raw
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Token
# ---------------------------------------------------------------------------

$token = $env:EXOCHAIN_ADMIN_TOKEN
if (-not $token) {
    Write-Host "Node admin token (input hidden):" -ForegroundColor Cyan
    $secure = Read-Host -AsSecureString
    $token = [System.Net.NetworkCredential]::new("", $secure).Password
}
if (-not $token) { throw "No token supplied." }

# ---------------------------------------------------------------------------
# Fetch
# ---------------------------------------------------------------------------

$url = "$($NodeUrl.TrimEnd('/'))/api/v1/avc/receipts/$Hash"
Write-Host "`nGET $url" -ForegroundColor DarkGray

try {
    $receipt = Invoke-RestMethod -Uri $url -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 30
} catch {
    $code = $_.Exception.Response.StatusCode.value__
    switch ($code) {
        401 { throw "401 — the node rejected the token. Check it is the admin token for THIS node." }
        404 { throw "404 — the node has no receipt with that hash. Check the hash, and that you are pointed at the node that minted it ($NodeUrl)." }
        default { throw "The node returned $code. $($_.Exception.Message)" }
    }
} finally {
    # Out of the session as soon as it has been used.
    Remove-Variable token -ErrorAction SilentlyContinue
}

# Hash256 comes back as a byte array or a hex string depending on the endpoint.
function ConvertTo-Hex($value) {
    if ($null -eq $value) { return $null }
    if ($value -is [string]) { return $value.ToLower() }
    return (($value | ForEach-Object { $_.ToString("x2") }) -join "")
}

# ---------------------------------------------------------------------------
# What it says
# ---------------------------------------------------------------------------

Write-Host "`n--- what the node signed ---" -ForegroundColor Cyan

$decision = $receipt.decision
$decisionColour = if ($decision -eq 'Allow') { 'Green' } else { 'Red' }
Write-Host ("  decision        : {0}" -f $decision) -ForegroundColor $decisionColour
Write-Host ("  reason codes    : {0}" -f ($receipt.reason_codes -join ", "))
Write-Host ("  validator       : {0}" -f $receipt.validator_did)
Write-Host ("  credential      : {0}" -f (ConvertTo-Hex $receipt.credential_id))
Write-Host ("  action          : {0}" -f (ConvertTo-Hex $receipt.action_id))
Write-Host ("  receipt id      : {0}" -f (ConvertTo-Hex $receipt.receipt_id))

if ($receipt.created_at) {
    $ms = $receipt.created_at.physical_ms
    if ($ms) {
        $when = [DateTimeOffset]::FromUnixTimeMilliseconds([int64]$ms).UtcDateTime
        Write-Host ("  created at      : {0:yyyy-MM-dd HH:mm:ss} UTC" -f $when)
    }
}

# A receipt with no signature is not a receipt. Reported rather than assumed.
$signed = $null -ne $receipt.signature -and "$($receipt.signature)" -ne "Empty"
Write-Host ("  signature       : {0}" -f $(if ($signed) { "present" } else { "MISSING" })) `
    -ForegroundColor $(if ($signed) { 'Green' } else { 'Red' })

# Receipts are hash-chained on the node as well, so a receipt names the one
# before it. A gap there would mean the node's own history had been edited.
if ($receipt.previous_receipt_hash) {
    Write-Host ("  previous receipt: {0}" -f (ConvertTo-Hex $receipt.previous_receipt_hash))
}

# What the node actually attested, in words. The most useful part of the whole
# object and the easiest to overlook among the byte arrays.
if ($receipt.action_descriptor) {
    $d = $receipt.action_descriptor
    Write-Host "`n--- what was attested ---" -ForegroundColor Cyan
    Write-Host ("  actor           : {0}" -f $d.actor_did)
    Write-Host ("  permission      : {0}" -f $d.requested_permission)
    Write-Host ("  tool            : {0}" -f $d.tool)
    Write-Host ("  data class      : {0}" -f $d.data_class)
    Write-Host ("  human approval  : {0}" -f $(if ($d.requires_human_approval) { "required" } else { "not required" }))
}

# A third party's assertion that the receipt existed at a given moment. Without
# it the node is the only witness to its own timing.
if ($receipt.timestamp_provenance) {
    Write-Host "`n--- independent timestamp ---" -ForegroundColor Cyan
    Write-Host ("  provenance      : {0}" -f $receipt.timestamp_provenance)
    $p = $receipt.external_timestamp_proof
    if ($p) {
        Write-Host ("  authority       : {0}" -f $p.authority_did)
        if ($p.issued_at.physical_ms) {
            $t = [DateTimeOffset]::FromUnixTimeMilliseconds([int64]$p.issued_at.physical_ms).UtcDateTime
            Write-Host ("  stamped at      : {0:yyyy-MM-dd HH:mm:ss} UTC" -f $t)
        }
        if ($p.rfc3161) {
            Write-Host ("  kind            : {0}" -f $p.proof_kind)
            Write-Host ("  TSA             : {0}" -f ($p.rfc3161.tsa_subject -split ",")[0])
            Write-Host ("  token           : {0} bytes of DER" -f [math]::Round($p.rfc3161.token_der_base64.Length * 3 / 4))
        }
    }
}

# ---------------------------------------------------------------------------
# Is it about the meeting you think it is?
# ---------------------------------------------------------------------------

if ($NoteId) {
    # Mirrors ACTION_DOMAIN in src-tauri/src/exochain/emit.rs. If that constant
    # ever changes, this check starts failing for correct receipts — which is
    # the right way round, since changing it would orphan every receipt already
    # minted.
    $domain = "note67.action.v1|meeting-attest|"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($domain + $NoteId)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $expected = (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }) -join "")
    $actual = ConvertTo-Hex $receipt.action_id

    Write-Host "`n--- is this receipt about that meeting? ---" -ForegroundColor Cyan
    Write-Host ("  expected action : {0}" -f $expected)
    Write-Host ("  receipt action  : {0}" -f $actual)

    if ($expected -eq $actual) {
        Write-Host "  MATCH — this receipt attests that note." -ForegroundColor Green
    } else {
        Write-Host "  NO MATCH — this receipt is for a different action." -ForegroundColor Red
    }
}

if ($Raw) {
    Write-Host "`n--- raw ---" -ForegroundColor DarkGray
    $receipt | ConvertTo-Json -Depth 12
} else {
    Write-Host "`n(pass -Raw for the full object, including the RFC-3161 token)" -ForegroundColor DarkGray
}
