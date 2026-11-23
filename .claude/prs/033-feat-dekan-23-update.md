---
title: Update Dekan to 2.3 with the supplied bridge and 1.16 fixes
labels: [enhancement]
branch: dekan-2.3-update
status: open
---

## Summary

Merge the supplied public 1.16 source updates into Dekan while preserving its release identity, signing configuration, webhook workflow, Rich Presence, Turkish/English localization, logos and saved themes. Replace the vendor launcher and bridge with the maintainer's latest binary pair and refresh their pinned hashes. The project and star links point to chrisssst/Dekan.

## Validation

Run the existing local policy and formatting checks, the supported native parser/party/generation test subset, and full Windows GitHub Actions checks. Record real-match validation separately; no match success is claimed by a green build.

## Release

Prepare 2.3 for review. Keep main and latest unchanged until separately requested.
