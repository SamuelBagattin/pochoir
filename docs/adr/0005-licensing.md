# ADR-0005: Licensing

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** [ADR-0001](0001-stack-and-dependency-policy.md), in respect of its licensing recommendation only

## Context

The repository is public at <https://github.com/SamuelBagattin/pochoir>. Until
this decision it carried `license = "UNLICENSED"` and no `LICENSE` file, which
means **nobody could legally use or contribute to it**. That is the forcing
constraint: the default in the absence of a licence is "all rights reserved",
so a public repository with no licence is an invitation that cannot be accepted.

It also had to be settled early rather than late. Relicensing after the fact
needs the agreement of every contributor, so the cost of deferring rises with
each pull request.

[ADR-0001](0001-stack-and-dependency-policy.md) recorded this as its largest
open question and recommended `AGPL-3.0-or-later` for the whole project, with
`MIT OR Apache-2.0` as runner-up. That ADR is otherwise unaffected by this one;
only its licensing recommendation is superseded here.

Decided elsewhere and not reopened: the project ships **two artefacts from one
codebase** — a Rust engine compiled to WebAssembly and reused as a native CLI,
and a static web application that calls it (ADR-0001, Part 1). That shape is
what makes a split licence available at all.

### What the licences actually say

- **AGPL-3.0's section 13** requires that if you modify the program and your
  version supports remote network interaction, you must "prominently offer all
  users interacting with it remotely through a computer network … an opportunity
  to receive the Corresponding Source of your version" (quoted from
  `LICENSE-AGPL` in this repository, which is byte-identical to
  <https://www.gnu.org/licenses/agpl-3.0.txt>). The FSF describes AGPLv3 as
  GPLv3's terms plus that paragraph, and recommends it "for any software which
  will commonly be run over a network"
  (<https://www.gnu.org/licenses/license-list.html#AGPLv3.0>).
- **Apache-2.0 is compatible with version 3 of the GNU GPL**, and the FSF
  recommends it over other lax permissive licences for substantial programs
  because of its patent-termination provision
  (<https://www.gnu.org/licenses/license-list.html#apache2>).
- **The Expat licence, commonly called "the MIT License", is compatible with the
  GNU GPL** (<https://www.gnu.org/licenses/license-list.html#Expat>).

Compatibility here is **one-way**: permissively licensed code can be taken into
a copyleft work, not the reverse.

## Options considered

### Option A — `AGPL-3.0-or-later` throughout

ADR-0001's recommendation. Section 13 covers every part of the project,
including the engine.

Cost, as ADR-0001 itself stated honestly: it makes the core crate effectively
unusable in anyone's permissive project, so it will not be adopted by others,
and AGPL crates see little uptake on crates.io. A distribution maintainer
packaging the CLI inherits copyleft for a command-line tool where no network
interaction is involved and §13 therefore buys nothing.

### Option B — `MIT OR Apache-2.0` throughout

The Rust ecosystem default. Maximises reuse and readership.

Cost: it specifically permits the scenario this project exists to prevent —
somebody takes the code, adds a server, and ships a hosted version with our
privacy language intact and no obligation to publish what they are running. For
a tool whose entire claim is "nothing leaves your device", that is not a
peripheral risk.

### Option C — split: permissive engine, copyleft application

`MIT OR Apache-2.0` for `crates/pochoir-core` and `crates/pochoir-cli`;
`AGPL-3.0-or-later` for the web application and site, including any future
`app/` or `web/` directory.

Available because the engine and the application are already separate artefacts
(ADR-0001, Part 1), and workable because the one-way compatibility runs in the
direction we need: the AGPL application may incorporate the permissive engine.

## Decision

**We will split-license the project.** The Rust engine and CLI —
`crates/pochoir-core` and `crates/pochoir-cli` — are `MIT OR Apache-2.0`, at the
user's option. The web application and the site, including any future `app/` or
`web/` directory, are `AGPL-3.0-or-later`.

The reasoning is that §13 and permissive reuse protect different things, and the
two halves of this codebase need different protection:

- **The application is where the harm lives.** The thing worth preventing is
  somebody running a hosted document-watermarking service built on this code
  without disclosing it. §13 is the one licence term that addresses exactly that
  case, and it only bites on software offered over a network — which is the
  application, not the engine.
- **The engine is where the reuse lives.** A watermarking and document-rendering
  pipeline is a genuinely reusable component: integrators should be able to embed
  it and distribution maintainers should be able to package the CLI, neither of
  which should require adopting copyleft. `MIT OR Apache-2.0` is also the
  ecosystem default, so the crates can be published without being dead on
  arrival.
- **Nothing is given up at the seam.** Permissive code can be taken into a
  copyleft work, so the AGPL application may use the engine freely. The reverse
  does not hold, which is the intended asymmetry.

`license` in `Cargo.toml` is `MIT OR Apache-2.0` and describes the crates only.
It is not a statement about the application. [LICENSE.md](../../LICENSE.md) is
the operative notice and says which licence covers which directory.

**Runner-up: Option A, `AGPL-3.0-or-later` throughout.** It lost on the engine
only. ADR-0001's argument for it is correct about the application and this
decision keeps it there; what it got wrong was treating a reusable library and a
hosted-service target as one thing. **The condition under which Option A wins
after all** is the engine turning out not to be reusable in practice — if, after
a year, nobody has embedded `pochoir-core` and the permissive half has bought us
nothing, the asymmetry is pure cost and relicensing the crates to AGPL becomes
the simpler, stronger position. That would need a new ADR and every
contributor's agreement.

## Consequences

### What this gets us

A licence that the project can actually accept contributions under, settled
before the first external pull request. §13 protection exactly where a hosted
fork would appear. An engine that other people can use, which is also the half
most likely to attract the review that a privacy tool needs.

### What this costs us

- **The engine can be embedded in a closed, proprietary product.** Somebody may
  take `pochoir-core`, build it into a commercial application, ship no source
  and give no credit beyond the licence notice, and that is a permitted use, not
  a loophole. We are accepting this deliberately in exchange for the reuse.
- **A permissive engine is a partial route around §13.** A would-be hosted
  competitor can take the engine — the hard, valuable part — and write its own
  front end rather than forking ours, and owes nothing. §13 protects our
  application; it does not protect the pipeline from being reused by a
  server-side product. The protection is real but narrower than Option A's.
- **Two licences is more explaining than one.** Every contributor has to know
  which half they are contributing to, the repository needs `LICENSE.md` to say
  so unambiguously, and a careless file placed in the wrong directory carries
  the wrong licence. This is a documentation burden that Option A and Option B
  do not have.
- **AGPL on the application narrows who will contribute to it** and rules out
  reusing its components in permissive projects, which is the standard cost of
  copyleft and is accepted.

### What this forecloses

Relicensing either half without the agreement of every contributor to it. In
particular, moving the crates to AGPL later — the runner-up condition above —
gets harder with each permissive contribution accepted. Taking AGPL code into
the crates is also now foreclosed, because it would contaminate the permissive
half; a dependency's licence must remain compatible with both halves, which
ADR-0001's rule that every dependency states its full SPDX expression already
checks for.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| A file lands in the wrong half and carries the wrong licence | **Medium** | Medium | `LICENSE.md` states the boundary by directory; `CLAUDE.md` points at it; revisit by adding per-file SPDX headers if it happens once |
| Somebody builds a hosted competitor on the permissive engine with their own front end | Medium | Medium | Accepted cost, stated above. No mitigation available short of Option A |
| The permissive half buys no reuse and the asymmetry is pure cost | Medium | Low | The runner-up trigger above: review after a year of the crates being published |
| A future dependency is copyleft and cannot go in the crates | Low | Medium | ADR-0001 already requires the full SPDX expression, including conjunctive terms, in every dependency ADR |
| A contributor assumes one licence covers everything and objects later | Low | Medium | `LICENSE.md` states what opening a pull request against each half offers the work under |

## Open questions

None. This ADR is `Accepted`.

## Could not verify

- **That Apache-2.0's GPLv3 compatibility extends to AGPLv3.** The FSF states
  Apache-2.0 is compatible with GPL version 3
  (<https://www.gnu.org/licenses/license-list.html#apache2>) and separately that
  AGPLv3's terms are GPLv3's plus the §13 paragraph
  (<https://www.gnu.org/licenses/license-list.html#AGPLv3.0>), but **no primary
  source was found stating the Apache-2.0-into-AGPLv3 case verbatim**. The
  inference is the near-universal reading and the same analysis applies, but it
  is an inference. It is load-bearing for Option C, so it is flagged rather than
  asserted. The direction we rely on (permissive into copyleft) is the
  uncontroversial one.
- **Nothing here is legal advice and none of it was reviewed by a lawyer.** The
  licence texts are quoted accurately from their canonical sources; what they
  mean in a dispute is not something this document establishes.
- Whether a split licence measurably deters a hosted fork in practice. The
  argument is about incentives, not evidence.

## References

- `LICENSE.md` in this repository — the operative notice
- GNU Affero General Public License v3.0 — <https://www.gnu.org/licenses/agpl-3.0.txt>
- Apache License 2.0 — <https://www.apache.org/licenses/LICENSE-2.0.txt>
- MIT (Expat) licence, SPDX canonical text — <https://spdx.org/licenses/MIT.html>
- FSF licence list, Apache-2.0 — <https://www.gnu.org/licenses/license-list.html#apache2>
- FSF licence list, Expat — <https://www.gnu.org/licenses/license-list.html#Expat>
- FSF licence list, AGPLv3 — <https://www.gnu.org/licenses/license-list.html#AGPLv3.0>
