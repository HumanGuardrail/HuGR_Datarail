# WORKING BACKWARDS — the launch announcement, written first

> **Owner-reserved. Status: DRAFT.** Amazon's working-backwards: write the press release before the
> system exists, so the demand is concrete and the team builds toward a felt customer outcome — not a spec.

---

## FOR IMMEDIATE RELEASE

### Datarail makes data move between any two points — sealed so tightly the infrastructure can never read it, delivered exactly once, and proven.

**Today we're launching Datarail, the serverless data rail that moves your data from a defined source to a
defined destination — across containers, clouds, or companies — in a vault that nothing in the middle can
open.** No broker to run. No sidecar to pay for. No trust in the pipe. When there's something to deliver,
an ephemeral rail spawns, carries the sealed vault over the cheapest path available, confirms delivery with
a cryptographic receipt, and disappears — leaving nothing standing to operate or to attack.

**The problem.** Every team that moves data between systems hits the same trap. Brokers and pipelines are
standing infrastructure that *reads your data* and bills you around the clock. The "secure transfer"
servers that promise guaranteed delivery *decrypt your data* on the way through — which is exactly why they
became the biggest breach of the decade. And "exactly once" is, almost everywhere, a marketing word
painted over an at-least-once pipe.

**The solution.** Datarail inverts it. The intelligence and the keys live in a featherweight endpoint at
your container's edge; the rail itself is a dumb, ephemeral, scale-to-zero mover. Because every vault is
sealed end-to-end, the pipe can be the cheapest, most disposable thing available — an S3 bucket, a peer,
shared memory — and it still can't read a byte. Delivery is exactly once, anchored by a signed, verifiable
manifest. Idle cost is essentially zero.

**Customer voice.** *"We move regulated records between two clouds that don't trust each other. With our old
MFT vendor, every transfer meant trusting a server that decrypts the data — a compliance fight and a breach
waiting to happen. With Datarail, the rail physically cannot see the cargo, every delivery comes with a
proof we hand the auditor, and there's no cluster sitting there burning money between transfers. It's the
thing we wished existed."* — *target design partner*

**How it works.** Define a route (source → destination) and the rules each end enforces, in one declarative
file. Hold your keys. `datarail run`. The vault is sealed at the source, moved by an ephemeral rail over
the cheapest viable substrate, and only opened at the destination after its rules pass — and a tampered or
duplicate vault never lands.

**Availability.** *TBD — gated on the proof obligations in `DECOMPOSITION.md`. No headline performance
numbers are published until the benchmark rig measures them against real engines, wins and losses both.*
