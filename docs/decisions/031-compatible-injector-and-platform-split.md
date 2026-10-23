# ADR-031: Carry forward compatible official injectors and platform separation

Status: accepted for implementation; Windows CI and real-match verification pending.

## Context

Recent game PROP bodies contain both four-byte and eight-byte hash values. Treating every hash as four bytes
misaligns later fields and prevents generated skins or overlays from being built. Some newer official patcher
DLLs also restrict their caller to LTK Manager. A confirmed hook therefore does not alone prove that custom
archives are served. The release must retain Dekan's themes, Turkish/English UI, branding, update key and
existing relay DNS recovery while incorporating the current fixes.

## Decision

Import the reviewed format, arming, reconnect, installer, host-refusal, bounded party-send and shutdown changes
from source commit `7433833542b1512cf4f7ad09d9dcaa6b2b735b49`. Detect variable hash widths with bounded parsing
and preserve eight-byte values during reference remapping. Refuse host-restricted DLLs before arming rather than
patching their bytes. Resolve official release tags to commits before downloading injector files. Keep the
existing publisher-signature check for signed files and accept unsigned official releases as the imported code
does; the protected tools folder and HTTPS official release are the remaining origin anchor.

The platform separation adds a direct dependency on the already-transitive workspace `libc` package. It is used
only by macOS platform modules for native process and filesystem APIs. This imports no additional runtime
component into Windows builds and avoids a separate FFI crate. Windows remains the shipped application target;
the portable macOS gate does not imply a macOS Dekan installer.

Update Tokio to 1.54.1 and ctutils to 0.4.4 using Cargo's lockfile resolution. The latter addresses
RUSTSEC-2026-0385 detected by the dependency gate after integration; it does not change the party protocol. Keep theme assets, preference files, Discord identity,
relay URL and HTTPS DNS recovery, and the Ed25519 update public key byte-identical to Dekan 1.9.

## Consequences

The trust policy now also accepts unsigned files; it cannot authenticate a manually replaced unsigned binary.
Foreign or damaged signatures remain errors, and no DLL bytes or signatures are modified. Existing installed
settings and party configuration continue to be used. Format and integration tests establish file handling and
state transitions; they cannot establish game-engine compatibility or the cause of a particular game exit.
Dekan 2.0 is prepared as a pre-release until a real-match log establishes the result.
