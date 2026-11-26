# Licensing

Pochoir is **split-licensed**. Which licence applies depends on which part of
the repository you are looking at.

| Part | Licence | Files |
| --- | --- | --- |
| The Rust engine and CLI — `crates/pochoir-core`, `crates/pochoir-cli` | `MIT OR Apache-2.0`, at your option | [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE) |
| The web application and the site, when they exist — anything under a future `app/` or `web/` directory | `AGPL-3.0-or-later` | [LICENSE-AGPL](LICENSE-AGPL) |

Copyright (c) 2026 Samuel Bagattin.

## Why the split

The harm Pochoir exists to prevent is somebody handling identity documents on a
server. The harm a permissive licence would enable is somebody taking this code,
adding a server, and marketing it with our privacy language intact.

**AGPL-3.0-or-later on the application** is the one licence term that addresses
exactly that case: §13 requires that a modified version offered to users over a
network make its source available to those users. So anyone who forks Pochoir
into a hosted service has to publish what they are running, which is the only
version of the privacy claim a user can check from the outside.

**`MIT OR Apache-2.0` on the engine** keeps the part that is genuinely reusable
genuinely reusable. The core is a watermarking and document-rendering pipeline;
integrators should be able to embed it, distribution maintainers should be able
to package the CLI, and neither should have to adopt copyleft to do so. This is
also the Rust ecosystem's default pair, so the crates can be published without
being effectively unusable in other people's projects.

The two sit together without conflict: AGPL-3.0-or-later can take in
`MIT OR Apache-2.0` code, so the application may use the engine. The reverse
does not hold, which is the intended asymmetry — the engine stays open to
everyone, while the product people actually visit cannot be closed.

## What this means for a contributor

Contributions to `crates/` are taken under `MIT OR Apache-2.0`; contributions to
the application and site are taken under `AGPL-3.0-or-later`. Opening a pull
request against either means you are offering your change under that part's
licence.

## Notes

- [ADR-0001](docs/adr/0001-stack-and-dependency-policy.md) recommended
  `AGPL-3.0-or-later` for the whole project and recorded `MIT OR Apache-2.0` as
  the runner-up. Samuel chose this split instead, on 2026-11-26: it takes the
  §13 protection where it matters and keeps the reuse the runner-up was arguing
  for. The ADR's reasoning stands as written; this file is the decision.
- Every third-party dependency adopted in ADR-0001 is `MIT OR Apache-2.0`-style
  and compatible with both halves.
- `Cargo.toml` carries `license = "MIT OR Apache-2.0"` because it describes the
  crates only. It is not a statement about the application.
