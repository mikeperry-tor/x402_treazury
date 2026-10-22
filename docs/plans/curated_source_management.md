# Curated providers in source management

Enable source-managed listeners to discover and register shipped provider
definitions, preserving their reviewed API contracts rather than importing only
their OpenAPI documents. Add offline discovery of bundled providers through local
search and a provider-list tool. Keep generic public-specification registration available.
Use the existing `dyn_<registration-name>__<operation>` naming convention; the
user accepted `__` for consistency. A curated registration defaults to a short,
stable configuration ID, such as `exa`, producing `dyn_exa__search`. An explicit
registration name replaces that default without repeating the provider prefix.

This is unfinished implementation work. The current task prepares the plan;
runtime implementation is separate.

## Baseline and owning guides

Starting code baseline: `e71f0e1afc490222569d0984b2c2a7a15577fbca`.
Before implementation, record the actual starting SHA and any intervening relevant
changes, then reconcile this plan against them. Preserve unrelated local files,
including the untracked `TODO`, deployment overrides and `run_servers.sh`.

Current behavior:

- `source_details` accepts a directory slug or API base URL and probes fixed
  documentation candidates. It does not resolve reusable provider definitions.
- `source_add` accepts `spec_url` and optional `name`. It imports remote JSON into
  a default catalog configuration with path/tag selections and a fixed pricing
  key, without provider overrides, mappings, help or instructions.
- Registrations persist accepted document bytes and candidate selections.
  Publication, restoration and operator inspection rebuild tools from those
  inputs; operator refresh assumes a remote specification URL.
- The x402 List directory embeds its own reviewed configuration and schema.
  Other provider files are explicit filesystem inputs for static deployment.

Read the [agent-source guide](../agent-sources.md),
[configuration and composition rules](../configuration.md),
[catalog and execution architecture](../architecture.md),
[provider policy](../../providers/README.md#provider-policy),
[provider caveats](../../providers/CAVEATS.md),
[network policy](../network-egress.md),
[output boundaries](../architecture.md#mcp-and-output-boundaries),
[local resource policy](../local-resource-limits.md),
[development guide](../development.md), [testing policy](../testing.md),
[build scripts](../../scripts/README.md),
[reproducible builds](../reproducible-builds.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
Preserve the [best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation policy](../wallet-rotation.md#wallet-observation-policy) and
[selection and isolation boundaries](../architecture.md#wallet-selection-and-network-isolation).

## Proposed behavior

### Provider bundle and matching

Embed shipped provider TOML and their referenced local schema assets. Keep one
source of provider definitions; do not copy TOML settings into a second handwritten
registry. Use a small manifest for stable IDs and any explicitly reviewed URL
aliases that cannot be derived from the definitions. Derive lookup keys from the
embedded definitions once in memory. No installed data destination, persistent
index, directory crawling, startup vendor requests or external provider search
path is needed for this feature.

Use configuration IDs independent of presentation names and tool prefixes.
For example, `exa.toml` becomes `exa`, `locus/provider.toml` becomes `locus`, and
the RelayShield phone companion has a distinct ID such as
`relayshield-phone-reputation`. Validate ID uniqueness and asset completeness.
Inventory actual definitions before fixing the manifest; do not assume that file
stem `provider` or an existing tool prefix uniquely identifies a configuration.

Match canonical HTTPS API origin plus exact normalized base path, with explicit
aliases where necessary. Normalize equivalent URL spellings and trailing slashes;
do not use hostname substrings or broad path-prefix matching. Remote specification
lookup uses the canonical complete URL, preserving meaningful query/path values.
Never infer API identity from a marketing website, directory slug or an arbitrary
document's `servers` claim. Return multiple candidates rather than selecting the
first match or automatically combining companion configurations. RelayShield's
main and phone companion share an API base and are an acceptance case.

### Provider descriptions and local discovery

Add a root-level `description` metadata field to provider configuration. Make it
optional for existing deployments and external provider files, but require a
nonblank authored description for every shipped provider. Keep descriptions in
the provider TOML, not duplicated in the bundle manifest. Describe supported
capabilities and useful search terms concisely; distinguish companion catalogs
with different functionality. Do not derive summaries by fetching OpenAPI or copy
long usage instructions, volatile prices or unsupported vendor capabilities into
them. Existing operation override descriptions and `instructions_text` retain
their separate meanings. Update composition, inspection and origins handling for
the new metadata using ordinary field inheritance.

Extend `x402_treazury_sources_search` with `local_only`, defaulting to `false`.
When true, search only the bundled provider metadata, without external directory
or provider requests. Accept `q` and the existing browse `page`/`per_page`
pagination fields. Omitted or blank `q` enumerates the bundle. Use case-insensitive
substring matching across provider ID, display name, description and declared API base, ordered by
stable provider ID. Document this local matching separately from directory ranking
and endpoint tool BM25; no additional persistent or semantic search index is needed.
Allow omitted or `browse` mode; reject `best` and directory-only filter/ranking
arguments before I/O rather than silently ignoring them. Strip `local_only` before
remote dispatch when false. Existing directory defaults and modes stay unchanged;
there is no automatic remote fallback or mixed local/remote result set.

Add `x402_treazury_providers_list` as an explicit local enumeration tool with the
same pagination fields. It calls the same metadata projection and pagination path
as local search without a query. Both return provider ID, display name,
description, declared API base where available and schema provenance, plus total
and continuation information. Do not return schemas or imply that bundled means
registered, funded, currently reachable or live-qualified. Local entries remain
metadata leads; registration still validates current operator destination policy.

Expose local search/list only with `source_management = true`; management-enabled
listeners gain a seventh wrapper, while discover-on-demand alone still exposes
only tool search/call. Both local paths must run without wallet identity acquisition,
registration changes, credential reads, signing, funding or network I/O. Their
results reflect the shipped bundle rather than endpoint registrations or loaded
API tools. `tools_search` continues to search the endpoint's callable inventory.

### Details and registration

`source_details` retains its existing inputs. After resolving a valid API base,
return matching provider IDs, names and schema provenance. A bundled match may
skip speculative URL probes and report that they were skipped because a curated
definition was found. Preserve any requested directory listing. A match is offline
configuration knowledge, not a claim of current reachability or live validation.
Unknown providers retain the existing discovery behavior and response evidence.
Do not register, fetch remote schemas, probe prices or allocate/fund wallets merely
to report a match.

`source_add` accepts exactly one of `provider_id` or `spec_url`, plus optional
`name`. Do not add another API-base registration argument: details already resolves
API bases into selectable IDs. An unknown ID or ambiguous known specification
returns an actionable error/candidate list without publication.

A unique exact match for a known remote specification URL uses its curated
definition. Other specification URLs retain generic import behavior. Do not
replace an explicitly supplied document because its contents name a known API.
Provider-ID registration can load bundled local schemas without agent-selectable
local paths or fabricated public specification URLs. Remote curated schemas use
the existing public network boundary and import ownership machinery.

Keep registration identity endpoint-local. For new curated sources, deduplicate
by owner and provider ID; generic sources retain owner plus canonical spec URL.
Concurrent provider-ID and known-spec additions resolve to the same curated
registration. Repeating the URL of an existing legacy generic registration returns
that registration without implicitly converting its configuration. Explicit
provider-ID registration can create a separate curated source; normal name and
tool collision checks still apply. Refresh preserves registration identity.

### Accepted settings and tool construction

Persist one accepted source definition: provenance/acquisition reference, effective
provider catalog settings and exact accepted document. Reuse existing serialized
catalog types where practical, excluding process handles, cache paths and unrelated
operator state. Do not persist an additional generated inventory or build parallel
generic and curated catalog pipelines. Retain legacy records with their existing
generic defaults, names and URLs; do not reinterpret them using the new bundle.

Build with canonical provider names so existing name-keyed overrides and filters
apply correctly, then rename each resulting tool into the registration namespace.
For example, `netintel_dns_lookup` becomes `dyn_netintel__dns_lookup`, rather than
`dyn_netintel__netintel_dns_lookup`. Preserve complete schemas, descriptions,
original wire paths, parameter routing, method identities and response mappings.
Keep source-name filtering and installed-source summaries consistent with the
existing `__` contract. Reject actual tool-name collisions before durable mutation;
do not add aliases, prefix restrictions or a new naming migration.
Rebind known tool references in agent-facing help, descriptions and source
instructions consistently, including generated namespace guidance and explicitly
named tools under a custom registration name. Use the canonical-to-public name
mapping for actual catalog symbols; do not blindly replace provider names or
arbitrary prose, URLs, wire fields or schema values. Preserve references describing
unsupported external operations as such rather than inventing callable IDs.

Apply these precedence rules deliberately:

| Setting | Proposed behavior |
| --- | --- |
| API base, supported path/method/tag selections, POST exceptions, overrides, help and schema conversion | Use the accepted curated definition; generic imports retain current defaults. |
| Pricing key and credit conversion | Use curated advisory metadata; no registration-time pricing probes or new payment authority. |
| Response mappings and image settings | Preserve curated output behavior through the real bound invocation; existing source-management response-byte limits still apply. |
| Provider recommendations | Retain as advice; registration does not opt into recommendation filtering or add a new selection argument. |
| Wallet, payment caps, public destinations, allowed origins and direct/Tor mode | Existing operator/listener authority remains decisive. |
| Response, help and spec byte limits | Source-management limits govern dynamic operations; do not add a second minimum/counter/limit system. |
| Response read inactivity | Explicit source-management value wins; otherwise use the curated provider value, then the network default. Generic imports retain current inheritance. No whole-work deadline. |
| HTTP/1 and TLS compatibility | Apply reviewed provider compatibility through `src/network.rs`; generic imports and raw calls keep their current strict defaults. |
| Cover traffic | No new implicit enablement; honor applicable provider opt-outs under existing operator network/cover authority. |
| Catalog caches, warming and relays | Do not activate static-deployment machinery for accepted dynamic documents; retain existing dynamic fetch coalescing and lifecycle. |

Persist accepted provider values, but resolve current operator authority when
binding or admitting work; a saved timeout or transport snapshot must not freeze
obsolete wallet/origin policy. Validate help and every effective route through
the existing destination checks, including explicitly permitted cross-origin help.
Review this table at the design gate against every actual Config field and owning
consumer; document any intentionally unsupported field with a concrete reason.
Apply the selected timeout and protocol compatibility to remote schema acquisition
and refresh as well as API/help execution; the provider may need that compatibility
before any tools can be built. Preserve each operation's existing isolation owner.

Compose dynamic source instructions from the immutable static base on each
publication. Respect listener instruction replacement, including empty suppression;
include only currently visible enabled sources. Refresh, removal, restore and
static republishing must neither duplicate nor retain stale guidance. Keep
operation-critical requirements in tool descriptions/help. Do not promise a live
instruction update to clients that cache initialization or tool descriptions.

### Persistence and maintenance

Add, restore, publication, saved inventory inspection and refresh must derive tools
from the same accepted representation. Restart and offline inspection use saved
bytes/settings without bundle re-resolution, vendor requests or wallet credentials.
Keep supported legacy state readable; new fields must have intentional legacy
defaults. Preserve existing restore disable/report behavior for invalid saved
catalogs and startup rejection for corrupt or unsupported registry state. Rejected
new registrations and refreshes must preserve the accepted durable state.

Explicit operator refresh of a curated registration resolves its current bundled
definition and obtains the selected remote or bundled schema. It validates and
atomically replaces the document/settings/provenance together, retaining name,
owner and source identity while advancing the revision. A missing bundled provider
or failed refresh preserves the accepted revision. Generic refresh keeps its
existing remote behavior. Restart alone never adopts changed bundled semantics.
Removal keeps existing tombstone/evidence rules and clears saved catalog content
consistently. Registration deduplication does not refresh a source implicitly.

Retain journal-before-publication, cancellation ownership and snapshot isolation.
Captured calls keep their original route, payer and revision during refresh,
removal or static replacement. Nothing here changes financial journal formats,
funding claims, payment replay authority or registration's no-funding contract.

## Ordered implementation commits

The identifiers below remain stable if titles change. Include each change's tests
and permanent documentation in its implementation commit, rather than deferring
documentation to cleanup.

1. **C1 Bundle shipped provider assets and resolve their identities.** Add the
   embedded bundle, stable ID/alias manifest and derived API/spec lookup. Add
   backward-compatible description metadata and author descriptions for all shipped
   providers, with ordinary composition/inspection support. Implement shared offline
   metadata projection, local filtering and pagination without exposing tools yet. Include
   provider assets in Cargo rebuild triggers, build/runtime source fingerprints,
   dirty detection and relevant release inputs. Inspect all fingerprint consumers.
   Acceptance: relocation-independent local-schema loading; all bundled TOML and
   referenced assets and nonblank descriptions validate offline; normalization,
   unknown/ambiguous match and
   alias tests use small synthetic definitions. A provider edit changes the build
   identity while private local overrides remain outside the bundle/fingerprint.
   Update `providers/README.md`, `docs/configuration.md`, `docs/development.md`
   and affected reproducible-build
   guidance with bundle membership and asset ownership. No tool API changes yet.

2. **C2 Unify accepted source configuration across the complete lifecycle.** Add
   saved provenance/settings with legacy defaults; build canonical tools and rebind
   dynamic names; apply runtime/output policy and instruction composition. Update
   restore, publication, inspection, refresh acquisition/validation and removal
   together. Retain generic behavior and exercise curated definitions internally
   before exposing them through management arguments. Acceptance: minimal authored
   fixtures prove base repair, route exclusions, overrides, help and image execution;
   canonical names are not repeated; guidance names callable dynamic tools even
   with a custom registration name; saved inspection matches generated runtime
   inventory; old registries and current naming remain compatible. Cover the policy
   table, failed refresh preserving state, offline restart after bundle changes,
   cancellation after durable acceptance, static republishing and captured calls.
   Update owning sections in `docs/agent-sources.md`, `docs/architecture.md`,
   `docs/configuration.md` and `docs/network-egress.md` in the same commit.
   **G1 follows C2 before public activation.**

3. **C3 Expose local provider discovery and curated registration.** Update tool schemas,
   descriptions and result provenance; wire shared resolution into details/add,
   origin-aware deduplication and the existing publication lock. Add local-only
   source search and the provider-list wrapper sharing bundle metadata projection;
   update wrapper counts, discovery instructions and inspection/startup consumers.
   Preserve optional
   directory metadata and generic discovery/import fallback. Acceptance: known
   API details require no provider probes, unknown details retain probes, ambiguous
   bases expose both candidates, local curated schemas register without HTTP fetch,
   known spec/provider selectors converge concurrently, and legacy repeated adds
   preserve the accepted source. Local matching covers ID/name/description,
   API-base matches, case handling, blank queries, zero matches and stable pagination; list and blank
   local search agree. Assert zero network requests and wallet work on both local
   paths, including invalid mixed-mode inputs, and no local selector reaches the
   directory upstream. Verify the seventh wrapper requires source-management
   authority and existing remote browse/best behavior stays intact. Rejected
   IDs/arguments/routes and name collisions
   produce no registry publication, wallet allocation, funding, signatures or
   unexpected provider requests. Use existing directory/payment fixtures for paid
   listing ownership; do not introduce live funded qualification. Update
   `docs/agent-sources.md`, configuration examples, the README discovery workflow,
   provider policy and any affected named provider caveats.
   **G2 follows C3 as the final implementation integration gate.**

4. **C4 Complete the documentation handoff and remove this plan.** Verify all
   enduring rules live in their owning guides and that `AGENTS.md` links reach
   bundle/matching, naming, persistence/refresh, policy and test contracts. Add or
   amend links where required. Fix any plan/index references, retain only concrete
   unfinished work, then remove this completed plan without an archive. Do not
   remove financial or qualification evidence. Independently review the exact
   cleanup SHA and the final combined branch after removal.

## Independent reviews and acceptance gates

Before substantive implementation, give an independent read-only design reviewer
the user intent, baseline, policy table, actual bundle inventory and proposed saved
representation. Challenge automatic matching, configuration duplication, every new
check and opportunities to remove responsibilities. Initial planning reviews
identified lifecycle consumers, shared-base ambiguity and name-keyed overrides;
recheck these against the implementation design. Resolve material findings before
building on an affected decision.

Review every exact implementation commit SHA independently, including relevant
surrounding consumers and failure paths; the author cannot serve as its reviewer.
Supply the exact baseline, diff, test names/counts/results and documentation
changes. Address findings using `git commit --fixup=<introducing-SHA>` and obtain
independent review of each exact fixup SHA. Preserve unsquashed history through
final integration. If this planning change is committed, independently review its
exact SHA as well.

**G1 reviews all C1/C2 commits and fixups** across bundled metadata and shared local
browse, acquisition, accepted settings,
catalog conversion, naming, publication, instruction composition, restore,
inspection, refresh, help/output execution and network/payer bindings. Reviewers
must verify full call chains, legacy reads and the absence of new funding/replay
authority. Exit only with a coherent shared representation, policy precedence
supported by nonzero tests and no unresolved material findings.

**G2 reviews all C1–C3 commits and fixups together** from local bundle search/list
and public directory/base/spec inputs through matching, admission, persistent commit,
publication, search,
invocation and operator maintenance. Assign distinct concerns where independent
analysis helps: provider/build identity versus persistence/concurrency/network
ownership. Examine causes of every fixup and repeated assumptions; assess sibling
paths and decide whether broader audit, regression coverage, simplification or a
design change is required. Earlier per-commit approvals do not exempt earlier code.
Exit with documented end-to-end behavior, relevant validation and no unresolved
material findings. Later material changes reopen affected gates.

Use CogC/CycC for substantive Rust refactors and both architecture gates, comparing
against the confirmed starting baseline with the same pinned analyzer. Examine
changed functions and full chains, warnings and unchanged high-risk hotspots,
especially add/restore/snapshot/refresh. Evaluate simpler state, ownership and
control flow before retaining a hotspot; do not extract helpers only to move a
score. Advisory scores require review, not an automatic numeric rejection.
Keep reports under ignored `target/` with source/baseline identities.

Run focused tests through `scripts/zcash.sh`, verify nonzero executed tests and
forbidden side effects, and run formatting and applicable Clippy checks. Choose
socket-capable permissions before localhost fixtures; do not run blocked socket
tests merely to provoke denial. Freeze sources/executables during validation and
serialize builds and feature configurations. Run the relevant discovery, config,
provider, output, network-audit and build-identity suites once at integration;
repeat only for changed behavior or unresolved failures. Use synthetic shared
tests for generic mechanics and named provider fixtures only for real authored
quirks, without full vendor inventory/settings snapshots. Offline evidence does
not establish live settlement, real Tor confinement or a reproducible release.
