# FlickNote public ChatGPT plugin preparation

Status: source preparation, 2026-09-30. Draft plugin JSON, skill, and branding
are prepared under `plugins/flicknote/`. They are not ready for submission:
production deployment, identity verification, reviewer access and recorded
review evidence remain pending. No upload or production deployment performed.

## Confirmed intent and existing evidence

The owner confirmed successful real ChatGPT MCP connection and intends public
directory publication. Public availability means people can discover/install
the integration; each connected account's notes remain private.

Existing ADR: FlickNote note 2806, "ADR: ChatGPT plugin 的远程私有笔记服务边界".
Retain its single full OAuth grant and user-owned private note collection model.
The ADR is a historical design record; current code and operator documentation
define the implemented tool and lifecycle contracts.

Apps-dev runs CLI v1.13.3. Reported live CRUD/search acceptance marker
`mcp-e2e-t4n8c1x5` passed; notes 2854/2855 were archived, with no active test
leftovers. This does not establish production readiness, live cross-account
isolation, PGroonga execution plans, backup recovery, or performance.

## Information established without owner questions

| Item | Source / finding |
| --- | --- |
| Product name | FlickNote |
| Published operator | Terms identify Guion FZE, Sharjah Publishing City Free Zone, UAE |
| Website | https://flicknote.app/ |
| Support | https://flicknote.app/support; support@flicknote.app |
| Privacy | https://flicknote.app/privacy |
| Terms | https://flicknote.app/terms |
| Branding | Reused fb flicknote-icon-no-bg.png as assets/icon.png; inspected transparent PNG, 320 × 320, 2,157 bytes |
| Existing plans | Website lists Free/Plus/Pro and Neurons; this alone does not establish plugin eligibility or commerce behavior |
| Current integration | Public HTTPS, Streamable HTTP, resource/OAuth discovery, DCR and PKCE S256 |

The four website pages were fetched and their content inspected. The homepage
does not yet explain the public ChatGPT plugin. Policy text describes general
third-party sharing but does not explicitly explain the connected ChatGPT
plugin's note retrieval/write and authorization lifecycle. Prepare specific,
factually supported coverage without inventing retention or training promises.

## Agent-owned preparation choices

- Reuse the current supported remote tool subset and existing authorization.
  Public release is not a reason to introduce new note permissions or business
  APIs. Do not advertise unsupported sharing, attachment access, or recording.
- Use portable root `plugin.json`, `mcp.json`, an adapted FlickNote skill, and
  existing branding. Author-supplied public uploads must not include private
  `.app.json` bindings. Do not duplicate compatibility manifests.
- Prepare a formal production endpoint before submission. Official review
  excludes testing endpoints; successful apps-dev connection is insufficient.
  Select and verify the durable endpoint before review, since the current
  published-server URL update flow is restricted. Production mutation requires
  separate target-specific approval after its concrete diff is ready.
- Audit every remote tool's explicit readOnlyHint/destructiveHint/openWorldHint
  and OAuth securitySchemes. Current source has partial annotations and no
  explicit securitySchemes declaration. Verify reauthorization in ChatGPT;
  preserve existing HTTP-layer token enforcement.
- Draft five distinct positive and three negative review cases from supported
  behavior. Record actual outcomes, including ChatGPT desktop/mobile behavior,
  independently from lower-level integration tests.
- Prepare a real recorded walkthrough and an accessible recording URL; a
  script alone is not evidence. Keep credentials outside packages and docs.
- Complete the portal-issued domain challenge, developer verification, scans
  and authorized legal attestations in the eventual submission workflow.

## Concrete preparation gaps

1. Production deployment and acceptance, including the exact MCP URL.
2. Plugin-specific website and policy coverage. Source JSON, listing copy, PNG
   icon and skill are prepared; final public ZIP waits for verified prerequisites.
3. Explicit tool authorization/annotation review and ChatGPT reauthorization
   tests.
4. Dedicated reviewer access. Current fb web authentication inspected here uses
   email OTP and Google/Apple; a password login path was not found. Official
   review access cannot depend on the owner's email/SMS codes or MFA approval.
   Do not bypass normal user authentication to resolve this.
5. Execute the five positive/three negative draft review cases in plugin.json
   and provide a verified demo recording. All eight new cases are **Not run**;
   earlier CRUD acceptance is separate evidence, not their execution result.
6. Complete the confirmed business publisher's OpenAI verification and choose
   the owning organization/project in the portal.

Lifecycle clarification: MCP note_add cannot create drafts. Ordinary creation
queues AI/source processing under existing behavior. Ordinary content/metadata
edits preserve lifecycle; note_submit explicitly submits an existing draft.
Absence of note_submit is not evidence that note creation avoids AI processing
or consumes no processing entitlement.

## Confirmed owner decisions

- Prepare the package now against the intended formally released service;
  production deployment will happen later. `mcp.json` uses the planned endpoint
  `https://gw.flicknote.app/mcp`, based on fse's existing production Gateway
  hostname. The MCP route is not deployed/verified there yet. Verify the exact
  endpoint and OAuth resource before final packaging; a hostname alone is not
  evidence of a working service.
- Publish as the business Guion FZE. OpenAI business verification will follow;
  the manifest records the intended identity, not verified status.
- Available in all OpenAI-supported countries. The explicit publication
  `countries: []` removes package country restrictions; it does not extend
  OpenAI's supported markets.
- All FlickNote accounts can connect. Preserve existing plan/processing
  entitlements and introduce no plugin checkout or upgrade promotion.
- The owner will provide a dedicated test account later. Its independent login
  requirements remain a prerequisite for review, not a requirement to draft JSON.

No new architectural ADR is needed: these choices retain note 2806's service
and ownership boundary. The package version is independently set to 0.1.0;
it does not claim to be the CLI binary's version.

## Prepared source and remaining evidence

- `plugins/flicknote/plugin.json`: portable identity, four existing website
  URLs, listing copy, three starter prompts, icon paths, review cases, commerce
  declaration, unrestricted countries, and initial release notes.
- `plugins/flicknote/mcp.json`: planned production Streamable HTTP connection.
- `plugins/flicknote/skills/flicknote/SKILL.md`: remote note workflows adapted
  from the CLI's MCP contract, without local daemon/recall installation advice.
- `plugins/flicknote/assets/icon.png`: existing FlickNote branding; optional
  separate dark-mode assets and brand colors are omitted.

Validation passed: both JSON documents conform to the fetched Agent Plugins
1.0.0 schemas. Listing lengths, case field types and counts, country targeting,
PNG dimensions/size, contained asset references, and public source inventory
were checked. The portable schema leaves OpenAI extensions unconstrained;
their fields were checked against the current official submission reference.
This is source validation, not an OpenAI portal scan or runtime acceptance.

Reviewer passwords and tokens are never included in the source. Demo URL is
omitted until a real recording exists; no placeholder or empty review field
stands in for missing evidence. No private app bindings or marketplace are
needed in this author-supplied public source.

Review fixture preparation, after the owner supplies the dedicated account:
create project Plugin Review and two sample notes, one containing 霜桥鹭影 and
quartz-lantern, the other unrelated. Create a Launch review note with a concrete
date and two action items. Run positive cases 3–5 in sequence; case 5 uses two
separate turns so restoring requires a new explicit request. Record returned
IDs and cleanup outcomes. Use only that dedicated account and sample data.

Suggested recording sequence: connect the review account, search/read the
sample note, create Review checkpoint, make its exact edit, archive it, then
explicitly restore it; demonstrate one unsupported share-link request. Keep
credentials and unrelated personal notes off screen. Verify the recording and
its accessible URL before adding review.demo_recording_url.

## Official sources

- Package: https://developers.openai.com/plugins/build/plugins
- Submission: https://developers.openai.com/plugins/deploy/submission
- Remote review: https://developers.openai.com/plugins/deploy/app-review
- Guidelines: https://developers.openai.com/plugins/plugin-guidelines
- Authentication: https://developers.openai.com/plugins/build/auth

These findings were checked against the current official documentation.

## Confirmed plugin skill direction

The owner chose a short behavior-oriented skill with little implementation
coupling and few definitions. The revised SKILL.md delegates parameter and tool
contracts to discovered schemas. It removes the duplicate tool reference,
internal queue/status vocabulary, authentication troubleshooting and commerce
explanations. No new glossary term or architectural ADR is needed.

Confirmed choices:

- Save supplied content with its original wording, allowing only necessary
  title/formatting changes. Rewrite or summarize when requested.
- Analysis, advice and decision requests default to proactive personal-context
  search, even without an explicit mention of notes. Simple translations,
  formatting and small text changes usually proceed directly.
- Read relevant notes before relying on them, compare historical records with
  current evidence and identify the notes used. Retrieval authorizes no writes.

The proactive default reduces the need for users to remember which useful
context they previously saved. The task distinction avoids unnecessary
retrieval during simple transformations. Broad mandatory retrieval for every
request was considered and not selected.

This edit changes skill instructions only. Its retrieval behavior still needs
real ChatGPT evaluation; source review is not evidence of model adherence.
