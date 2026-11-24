# Contributing to x402_treazury

Contributions are welcome: bug reports, provider definitions, documentation,
regression tests, and implementation improvements. For substantial changes, open
an issue or draft merge request first to discuss behavior, privacy impact, and
scope. Pull requests and merge requests follow the same policy.

## Licensing and contributor rights

The open-source edition uses GNU AGPLv3; see [LICENSE](LICENSE). The rights
holder may also offer alternative commercial licenses, so contributions intended
for inclusion must support both paths. Third-party dependencies and vendored
code keep their own licenses and notices.

The official distribution may include disclosed swap or partner fees that fund
development. AGPL-compliant forks are not required to preserve those fees or
partner settings. Commercial use is allowed under AGPL when its terms are met;
a commercial license simply offers different terms.

Contributors retain copyright in their original work. We require a
**non-exclusive contributor license grant, not copyright assignment**. The
contributor license agreement (CLA) must permit use, modification, distribution,
and sublicensing under both AGPL and alternative commercial terms, address
patent permissions, and confirm the contributor's authority to grant those
rights. Contributors remain free to use and license their own work elsewhere.

### Explicit agreement before merging

Before merging an external contribution containing copyrightable material:

1. The maintainer supplies the complete, versioned CLA identifying the legal
   person or entity receiving the grant and the contributions covered.
2. Each contributor accepts that agreement through the stated electronic
   process. Once accepted, a contributor does not need to re-accept the same
   CLA version for later merge requests.
3. The maintainer records the agreement version, acceptance record, contributor
   identity, and scope. Agreement updates are never accepted retroactively.

**The CLA and acceptance process have not yet been published.** Until they are,
external patches can be discussed and reviewed, though copyrightable
contributions cannot be merged under an assumed commercial relicensing grant.
Bug reports and feature discussions do not require a CLA. This policy covers
code, documentation, tests, and provider definitions, and does not convert
third-party material into contributor-owned work.

If an employer or third party owns the contribution, obtain authorization from
that rights holder first. Identify coauthors and any material copied or adapted
from elsewhere along with its source and license. AGPL-only third-party code
cannot be included in a commercial edition under a contributor CLA.

## Preparing a change

Keep each merge request focused. Describe the problem, the resulting behavior,
and the checks you ran. Call out any changes to wallet sharing, network
isolation, spending limits, or recovery behavior. Include regression tests for
behavior changes; documentation-only edits do not need new code tests.

Follow [provider conventions](providers/README.md) when adding or updating
providers. Keep tool inventories focused, preserve schema constraints, and
document protocol exceptions and reliability limits accurately. Do not infer
provider health from one successful request or silently broaden paid test scope.

Read [AGENTS.md](AGENTS.md) for repository invariants and conventions, and the
[development guide](docs/development.md) for build and test tooling. Typical
checks from the repository root are:

```sh
make build
make fmt-check
make test
make clippy
```

Run focused tests while developing; `make check` runs the full offline suite.
Use the same local toolchain across related checks and run opt-in test-feature
configurations sequentially because subprocess tests share
`target/debug/x402_treazury`. Report the compiler version and which checks
passed, failed, or could not run. See [testing](docs/testing.md) for
qualification limits: offline tests do not validate live payments or live Tor
isolation.

## Privacy, funds, and evidence

- Never commit seeds, private keys, bearer tokens, `.env` files, live wallet
  state, or private transaction journals. Tests must use temporary state and
  unfunded fixture keys.
- Live provider calls, funded tests, and live wallet operations require explicit
  operator authorization.
- Preserve payment uncertainty and durable journals: never replay ambiguous
  payments, bypass ownership checks, or add direct-network fallbacks in Tor
  mode.
- Make limits and truncation visible in logs, CLI output, and agent-facing
  results; never silently drop part of a response.
- Keep generated reports and raw evidence in ignored local or private external
  storage. Commit durable findings as tests, configuration, or maintained
  documentation rather than dated run journals, and sanitize diagnostics before
  sharing them.

For a suspected vulnerability involving funds, credentials, or isolation, ask a
maintainer for a private reporting channel before posting exploit details or
sensitive evidence in a public issue.
