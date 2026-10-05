# Contributing to x402_treazury

Contributions are welcome: bug reports, provider definitions, documentation,
regression tests and implementation improvements. For substantial changes, open
an issue or draft merge request first so we can discuss the behavior, privacy
implications and scope. Pull requests and merge requests follow the same policy.

## Licensing and contributor rights

The project uses GNU AGPLv3 for its open-source edition; see [LICENSE](LICENSE).
The licensing model also allows the project's rights holder to offer alternative
commercial licenses. Contributions intended for inclusion must support both paths.
Third-party dependencies and vendored code retain their own licenses and notices.

The official distribution may include disclosed swap or partner fees that support
development. We do not require AGPL-compliant forks to preserve those fees or
partner settings. Commercial use is permitted under AGPL when its terms are met;
an alternative commercial license provides different terms, rather than being
mandatory merely because a user or company earns money.

Contributors retain copyright in their original work. We seek a **non-exclusive
contributor license grant, not copyright assignment**. The separate contributor
license agreement (CLA) must expressly permit use, modification, distribution and
sublicensing of contributions under AGPL and alternative licenses, including
proprietary commercial terms. It must also address relevant patent permissions
and the contributor's authority to grant those rights. Contributors remain free
to use and license their own work elsewhere.

### Explicit agreement before merging

Before merging an external contribution containing copyrightable material:

1. The maintainer supplies the complete, versioned CLA, identifying the legal
   person or entity receiving the grant and the contributions it covers.
2. Each relevant contributor explicitly accepts that agreement using the stated
   electronic acceptance process. A contributor who has already accepted a CLA
   covering the contribution need not repeat acceptance for every merge request.
3. The maintainer retains the exact agreement version, acceptance record,
   contributor identity and contribution scope. Changes to the agreement are not
   retroactively treated as accepted.

**The CLA and acceptance process have not yet been published.** Until they are,
external patches may be discussed and reviewed, but copyrightable contributions
must not be merged under an assumed commercial relicensing permission. Bug
reports and feature discussions do not require CLA acceptance merely to participate.
This policy also covers contributed documentation, tests and provider definitions;
it does not turn third-party material into contributor-owned work.

If an employer or another party owns the contribution, obtain the necessary
authorization from that rights holder. Identify coauthors and any material copied
or adapted from elsewhere, with its source and license. Do not promise rights you
do not control. AGPL-only third-party code cannot simply be included in a
commercial edition on the strength of your CLA.

## Preparing a change

Keep each merge request focused. Describe the problem, resulting behavior and
validation performed. Explain changes to wallet sharing, network isolation,
spending limits or recovery behavior explicitly. Include regression coverage for
meaningful behavior changes; documentation-only edits do not need new code tests.

Use the existing [provider conventions](providers/README.md) for provider additions.
Keep tool inventories useful and bounded, retain schema constraints, and document
protocol exceptions or observed reliability issues precisely. Do not infer provider
health from one successful request or silently broaden paid test scope.

Read [AGENTS.md](AGENTS.md) for repository invariants and working conventions, and
the [development guide](docs/development.md) for build and test tooling. From the
repository root, typical checks are:

```sh
scripts/zcash.sh build
cargo fmt --check
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh clippy --all-targets -- -D warnings
```

Run focused tests while developing; `scripts/check.sh` is the complete check suite.
Use the pinned toolchain. Run default and no-default-feature suites sequentially
because subprocess tests share an executable path. State which checks passed,
failed or could not run. See [testing](docs/testing.md) for qualification limits;
offline tests do not establish successful live payments or real-Tor isolation.

## Privacy, funds and evidence

- Never submit seeds, private keys, bearer tokens, `.env` files, real wallet state
  or private transaction journals. Use temporary state and unfunded fixture keys.
- Live provider calls, funded tests and wallet operations need explicit operator
  authorization. A contribution or test command is not permission to spend funds.
- Preserve payment uncertainty and durable accounting. Do not replay ambiguous
  payments, bypass ownership protections or introduce direct-network fallbacks.
- Bounds and truncation must be visible in output or logs and in agent-facing
  results where applicable; never silently discard part of a result.
- Keep generated reports and raw evidence in ignored local or private external
  storage. Commit durable findings as tests, configuration or maintained guidance,
  rather than dated run journals. Sanitize diagnostics before sharing them.

For a suspected vulnerability involving funds, credentials or isolation, request
a private reporting channel from a maintainer before posting exploit details or
sensitive evidence in a public issue.
