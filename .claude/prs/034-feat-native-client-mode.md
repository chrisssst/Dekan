---
title: Add native in-client selection and Dekan client menu
labels: [enhancement]
branch: dekan-2.3-update
status: open
---

## Changes

Add the opt-in native League client menu and carousel using statically recovered 1.16 interface assets and a Dekan CDP coordinator. Preserve standalone themes, Turkish/English, party relay, Rich Presence, release key and webhook configuration. Use Dekan branding, logo, repository/star links and the existing Discord invite. Verify helper payloads, respect other debugger registrations, defer restarts until idle/lobby and retain the standalone picker when unavailable.

## Validation

Formatting and policy sweeps locally; protocol, serialization and UI tests in Windows CI. Live client/match validation remains pending.
