# Legal posture

**This document is the maintainer's own technical risk assessment based
on public information. It is not legal advice.**

Short version: this is a Linux tool that runs a SHA-1 brute-force on a
file the user already owns, without contacting a TeamSpeak server. The
algorithm is public, the existing tooling has been on GitHub unmolested
for years, and no relevant patents exist. Based on the current technical
design and public documentation, risk to a German-resident maintainer
appears low.

## Algorithm provenance

The "security level" formula and the identity-blob obfuscation were
recovered through black-box analysis of the `.ini` file format and the
observable behavior of the official client — not by decompiling, lifting
source, or breaking copy protection. Under German law (`§ 69d Abs. 3
UrhG`), observation, study, and testing of a program's functioning to
determine its underlying ideas and principles is permitted to anyone
entitled to use a copy of the program; this right is mandatory and not
contractually waivable (`§ 69g Abs. 2 UrhG`). Reverse-engineering for
interoperability (`§ 69e UrhG`) is not even invoked because no
decompilation was performed.

References: `landave/TSIdentityTool`, `landave/TeamSpeakHasher`,
`ReSpeak/tsdeclarations`. See [algorithm.md](algorithm.md).

## Patents

A quick search at EPO/DPMA against "TeamSpeak Systems GmbH" yields no
patents relevant to identity hashing or proof-of-work. The technique
(`SHA-1(message || counter)` with leading-zero check) is the
Hashcash/Bitcoin pattern, prior art going back to 1997.

## EULA / Terms

The current TeamSpeak EULA contains a generic reverse-engineering
restriction and limits "add-ons" to those distributed by TeamSpeak
Systems. This tool is not an add-on or plugin — it is an independent
offline CLI that operates only on a user-owned file. Generic EULA
RE-prohibitions cannot override `§ 69d Abs. 3 UrhG`, which is statutory
and EULA-fest in Germany.

## DMCA / takedown history

`github/dmca` records two TeamSpeak takedowns in 2018, both targeting
clones of the TeamSpeak software (server/plugin patches). No takedowns
have been filed against identity-hashing tooling. `landave/TeamSpeakHasher`,
`landave/TSIdentityTool`, `bratkartoffel/ts3idtools`, and several forks
continue to operate openly on GitHub.

## §202c StGB ("Hackerparagraph")

Not applicable. The German Constitutional Court ruling
`2 BvR 2233/07` (2009) limited §202c to tools whose purpose is
unambiguously to commit a crime. This tool:

- never contacts a TeamSpeak server;
- does not circumvent any access control;
- operates only on data the user owns;
- performs a SHA-1 search — the same operation as `hashcat`, `john`, and
  every cryptocurrency miner, all of which are legal in Germany.

## Trademark

"TeamSpeak" is a registered word mark of TeamSpeak Systems GmbH. Use here
is nominative — describing what this tool interoperates with — and is
permitted under `§ 23 MarkenG`. Mitigations applied:

- No TeamSpeak logos, screenshots, or icons are used or distributed.
- The repository and binary names do not suggest official affiliation.
- The README carries a prominent disclaimer.

## What you actually get

This is the maintainer's risk assessment for a German-resident hobby
project published under MIT. It is not advice. If you intend to bundle
this tool into a commercial product or distribute it under your own
name, consult a lawyer.
