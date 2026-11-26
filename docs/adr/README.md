# Architecture decision records

This directory holds the architecture decision records (ADRs) for **Pochoir**.

An ADR captures **one** architecturally significant decision: the context that
forced it, the options we weighed, what we chose, and what we now have to live
with. It is a record, not a design document — it says what we decided and why,
at the time we decided it, and it is not rewritten when reality changes. When a
decision turns out wrong, we write a new ADR that supersedes the old one and
leave the old one in place, because the reasoning that failed is the useful part.

## When to write one

Write an ADR when a choice is expensive to reverse, or when a future
contributor would otherwise reasonably ask "why on earth is it done this way?".
Concretely, in this project:

- adding, replacing or removing a **dependency** (this is mandatory — see
  [ADR-0001](0001-stack-and-dependency-policy.md));
- choosing or changing an algorithm that affects output quality;
- changing the trust or privacy boundary (anything that would make bytes leave
  the user's device);
- changing the build, hosting or distribution model;
- picking a file format, wire format or on-disk layout.

Do **not** write an ADR for routine implementation work, refactors that change
no interface, or anything a commit message already explains.

## Numbering

- Four digits, zero-padded, allocated **sequentially and permanently**:
  `0001`, `0002`, `0003`, …
- The filename is `NNNN-kebab-case-title.md`.
- `0000-adr-template.md` is the template and is not a decision.
- A number is **never reused**, even if the ADR is rejected or superseded. Gaps
  are fine; collisions are not. Before claiming a number, check the highest one
  already present on `main` *and* in any open branch.
- The title in the filename and the title in the document must match.

## Status values

Exactly one of:

| Status | Meaning |
| --- | --- |
| `Proposed` | Written, open for discussion, **not** binding. Code must not rely on it. |
| `Accepted` | Binding. The codebase should match it; a divergence is a bug in one or the other. |
| `Superseded by ADR-NNNN` | Was accepted, no longer is. The body stays untouched; only the status line changes, plus a pointer to the replacement. |

A fourth state, `Rejected`, is allowed for a decision we deliberately considered
and declined where the reasoning is worth keeping. It is rare; most rejected
options belong in the "Options considered" section of the ADR that rejected them.

Transitions are one-way: `Proposed` → `Accepted` → `Superseded`. We do not
un-accept an ADR. We supersede it.

## Writing rules

- **Recommend one option.** Name the runner-up and say why it lost. An ADR is
  not a survey of everything that exists.
- **Cite primary sources** with URLs for every load-bearing fact: docs.rs,
  crates.io, the crate's own repository, MDN, caniuse, a spec, a vendor's own
  docs. A size, a licence or a browser-support claim without a URL is an
  assertion, not evidence.
- **Flag what you could not verify.** Write "unverified" next to it. An ADR that
  quietly presents a guess as a fact is worse than no ADR.
- **Record consequences honestly**, including the bad ones. The section exists
  for the costs, not the benefits.
- **Mark open questions** that need the owner's decision explicitly, so they are
  findable: a dedicated section, and the ADR stays `Proposed` until they are
  answered. **Closing an open question:** an open question may be closed in place
  after acceptance — tick the box, write the answer beside it and name who
  decided — but only when the answer does not change the decision the ADR
  records. If it would change the decision, write a new ADR that supersedes this
  one instead.
- Keep it readable in a few minutes. If an ADR needs more, it is probably two
  ADRs.

## Index

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-stack-and-dependency-policy.md) | Stack and dependency policy | Accepted |
| [0002](0002-watermark-algorithm-and-quality-modes.md) | Watermark algorithm and output quality modes | Accepted |
| [0003](0003-pdf-input-rasterisation.md) | PDF input rasterisation | Accepted |
| [0004](0004-build-reproducibility-and-deployed-bundle-attestation.md) | Build reproducibility and deployed-bundle attestation | Proposed |
| [0005](0005-licensing.md) | Licensing | Accepted |
| [0006](0006-image-page-size-cap.md) | Cap an image input's page at A4 | Accepted |
| [0007](0007-watermark-specification-corrections.md) | Watermark specification corrections from implementation | Accepted |
