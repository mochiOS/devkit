# Device-wide consent records

The standalone `mochios-device-consent` library supplies a versioned local record and a fail-closed gate. It
contains no collector, uploader, endpoint, account identity, advertising ID, or
network call. AppCore contains no OS-specific consent module. Settings UI is
unchanged; the existing update report sender uses this shared gate.

The machine directory is `/var/config/privacy`; the file is `consent.json`.
Installed Data already provides the machine `/var` hierarchy. These are device
preferences, not per-user home files. Receipts are readable by local users (0644)
and replaceable only with the filesystem/owner authority enforced by the OS.
New consent directories are 0755. The record contains no passwords. Callers must
hold the normal filesystem capabilities; the SDK grants none.

`ConsentStore::at_directory` is an explicit, caller-authorized directory.
Installer uses a verified destination mount and appends `var/config/privacy`;
it must not use `ConsentStore::system()` to save an in-progress installation.
The system accessor is explicit. Linux Installer preview never saves to it;
tests use temporary explicit directories.

Schema 2 stores required OS terms, privacy acknowledgment and OSS confirmation
as document receipts (version, SHA-256 of exact bytes, Unix timestamp). Optional
Usage & Diagnostics (`usage_diagnostics`), Crash Reports (`crash_reports`) and
Update Results (`update_results`) each have an independent unselected/AGREE/
DISAGREE value, policy version, decision timestamp and increasing decision
revision. The revision distinguishes decisions made in the same second. No receipt
contains a local account identifier. New users consume the same machine record;
they do not cause another installation-consent prompt.

Call `ConsentStore::allows(category, current_policy_version)` before optional
acquisition and before every transmission. Missing, malformed, inaccessible,
incomplete or unknown-schema records deny permission. Changed policy versions
also deny it; an old grant must never authorize a changed purpose. The record
replacement is local and atomic, synchronized before publication and read back
by Installer. Incomplete initial records cannot be saved. A save/load failure is
an error, not permission to proceed. No external server receives the receipt.
The directory must be a trusted, capability-protected machine location; symlink
checks do not replace OS authority or protection against privileged mutation.

## Withdrawal and unimplemented future queue behavior

A settings consumer can revoke one category through `set_permission(..., false,
current_policy, now)` and save the complete record, leaving all other categories
and legal acknowledgments intact. Re-read the gate for each acquisition/send;
do not cache a grant for a worker's lifetime. The current module manages consent
only; it does not create diagnostic files or an upload queue.

Any future queue must record the withdrawal timestamp and the category/policy
version on its own entries. On withdrawal:

1. Stop new optional acquisition and all sending for that category immediately.
2. Keep unsent data locally for **at most** `PENDING_RETENTION_SECONDS` (seven
   days), deleting it if there is no renewed permission. A changed system clock
   must not extend that maximum through repeated restarts.
3. Renewed permission within seven days allows new acquisition, but old queued
   data still requires a separate explicit prompt before sending. Default deny.
4. Declining that prompt deletes the retained data. Renewal after expiry never
   resurrects deleted data.
5. Previously uploaded data is not automatically deleted solely because local
   consent was revoked; server deletion is a separate policy and mechanism.

Queue retention metadata belongs to that future implementation and survives a
subsequent consent change; it must not derive an old withdrawal deadline from a
new grant's timestamp. None of these requirements implies a working sender.

## Updates and document revisions

Document receipts identify the exact accepted revision. Future update preflight
must keep the old versioned document artifact, show its diff against the new
revision and the new full text, and obtain terms agreement **before** writing
an update. Refusal cancels the update and preserves the running OS.

For a changed optional-data purpose/content, preflight obtains a new decision
for only that category. Refusal still permits the OS update but leaves the new
policy denied. Each category's version is independent. New schema versions
require explicit validation/migration; unknown schemas never imply consent.
The current Installer uses provisional placeholder revisions; this work does
not change update eligibility, Settings UI, or AppStore. AppStore terms have a separate first-use flow
and are deliberately not represented as an OS-installation agreement.

## Server alignment and legacy permissions

See [the report contract audit](data-consent-contract.md) for the exact inspected
AdminDashboard revision, query fields and client behavior. No rotation period or
new identifier algorithm is specified by this consent model. Installer generates
no device identifier. All disclosure bodies remain external `abcdef` files.

Schema 1 and the previous `active_usage` / `performance_diagnostics` fields are
not aliases for the new categories. Their grants are rejected, not promoted.
The legacy `diagnostics_enabled` / `diagnostics_consent` configuration cannot
supply any new permission. A new explicit decision is needed for each category.

Existing report producers check their own category before acquiring optional
report data. The sender re-reads current consent before each POST. Local queue
metadata binds each entry to its category, policy version, decision timestamp
and revision; it is never included in the HTTP body. Legacy entries without a
matching receipt, or entries predating withdrawal/re-consent, cannot be sent.
The legacy transport field `basic_diagnostics_consent` is retained on existing
endpoint bodies for wire compatibility; it does not grant another category.

This change does not implement a new queue or the future seven-day retention /
re-consent prompt. The existing sender discards denied/stale queued reports;
it never automatically uploads retained old data after re-consent. The future
retention rules above remain required before implementing retained-data replay.
Update checks, downloads and OS installation do not depend on report consent.
No crash collector or sender is added; future ones must use the same crash gate.
