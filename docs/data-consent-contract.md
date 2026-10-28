# Optional data consent: implementation audit

This is a developer contract, not legal copy or an Installer disclosure.
All Installer summaries and six detail bodies remain exactly `abcdef`.

## Sources and boundaries

Inspected AdminDashboard default-branch commit:
[`c271716871f873c0955005bb3472148b5c89bb05`](https://github.com/mochiOS/AdminDashboard/tree/c271716871f873c0955005bb3472148b5c89bb05).
Its [report queries](https://github.com/mochiOS/AdminDashboard/blob/c271716871f873c0955005bb3472148b5c89bb05/app/api/admin/_lib/reports.ts)
read the shared REPORTS database. The dashboard README/spec identify ingestion
as a separate public API Worker. This repository does not implement those POST
handlers. The receiver repository was not available during this audit; accepted
request fields, server retention and identifier requirements remain unverified.
No requests were sent to ingestion endpoints and no server was modified.

Client source: `services/update/src/diagnostics.rs`, based on services commit
`b895cae5caa19825009a094d843a60db66450d62`. The existing client builds diagnostics
and update-result POST bodies. This change only connects those producers and
sender to the shared per-category consent gate; it adds no collector or endpoint.

## Current categories

| Local category | Dashboard table | Fields used by the inspected queries |
| --- | --- | --- |
| `usage_diagnostics` | `diagnostic_reports` | `device_id`, `last_active_date`, `boot_succeeded`, `previous_shutdown`, `build_number`, `received_at` |
| `crash_reports` | `crash_reports` | `device_id`, `os_version`, `build_number`, `error_code`, `component`, `kernel_panic`, `occurred_on`, `received_at` |
| `update_results` | `update_result_reports` | `target_version`, `target_build`, `outcome`, `error_code`, `occurred_on`, `received_at` |

The table lists query columns, not an asserted complete wire schema. In
particular, query use of `received_at` does not establish a client-sent field.
Purpose, destination, retention and exclusions are intentionally not authored
as real user-facing claims in this change.

Usage and boot stability share diagnostic_reports. DAU, WAU and MAU count
DISTINCT device_id for today, six preceding days plus today, and 29 preceding
days plus today. Diagnostic summaries additionally derive report/device counts,
successful boots and clean shutdown counts/percentages. One consent covers both
usage and boot diagnostics; separate performance consent would not match this
implementation.

Crash summaries group version/build, error code and component and count kernel
panics. Daily crashed-device counts use a same-day diagnostic-report join.
These queries do not establish transmission of memory dumps, stacks or registers;
none is added to the consent contract or collection implementation.

Update queries recognize `success` and `first_boot_succeeded` as successful in
release health. The daily metric explicitly counts `first_boot_succeeded`,
`failed` and `rolled_back`; it does not count the legacy `success` value in that
particular success numerator. The current OS client emits the latter three
outcomes. This difference is documented, not silently changed server-side.
Declining update-result reports never disables update checking or installation.

## Identifier and client wire observations

The existing OS client lazily creates a random UUID v4 and persists it at
`/var/lib/diagnostics/device_id`. It reuses valid stored values and has no monthly
rotation code. This is an observation of this client, not a newly approved
identifier lifecycle. The receiver must be audited before determining the final
identifier format, replacement period or relationship between report types.

Existing diagnostic bodies include event ID, OS version/build/architecture,
installation/active dates, boot/shutdown fields and existing unknown/null flags
beyond the query columns. Existing update bodies include event/device IDs,
source/target release objects, outcome and error code. The sender uses
`/diagnostics` and `/update-results` on the existing public API. Neither its body
schema nor identifier generation is changed here. No crash POST path is inferred
or implemented from the dashboard table name.

No legal receipt, AGREE/DISAGREE record, policy revision or local queue consent
stamp is transmitted to AdminDashboard. Transport encodes only the pre-existing
`body` object. Local authorization metadata stays outside it.

## Installation and future work

The category contract is in `mochios-device-consent`, independent of AppCore and
UI. Initial Installer decisions are drafts until Continue. The machine record
is persisted only at the verified target stage described in device-consent.md.
Three choices, including three DISAGREE decisions, satisfy the optional step.

Schema 2 rejects schema-1 category grants and legacy diagnostics settings;
there is no automatic broadening of consent. Re-reading the shared gate on
acquisition/send makes independently persisted revocation effective. The current
Settings UI is not changed. A new seven-day retention queue and retained-data
re-consent prompt are not implemented; their required behavior remains specified
in [device-consent.md](device-consent.md).
