# Research and Evidence

## Scope

Crosstalk can retrieve structured search metadata from arXiv and Zenodo before deliberation. This is a discovery and provenance layer, not a general web crawler, full-text archive, citation manager, or automatic truth oracle.

```text
query
  -> repository API
  -> bounded response
  -> typed records + response digest
  -> session JSON artifact
  -> injection-screened excerpts for evolution
  -> adversarial deliberation
```

## Repository adapters

| Repository | CLI | Endpoint | Authentication |
|---|---|---|---|
| arXiv | `--arxiv <QUERY>` | arXiv Atom API | Public search |
| Zenodo | `--zenodo <QUERY>` | Zenodo records API | Public search; optional `ZENODO_ACCESS_TOKEN` bearer token |

The client uses a 30-second HTTP timeout, caps each response at 8 MiB, and clamps result counts to 1–25. `--research-limit` defaults to 10 per requested repository.

Repository failures are isolated. If arXiv fails, Zenodo results can still be attached, and ordinary deliberation can continue without either source.

## Record schema

Each `ResearchRecord` contains:

- repository type;
- repository identifier;
- title and authors;
- abstract or description;
- publication date when available;
- canonical record URL;
- DOI when available;
- Unix retrieval timestamp; and
- SHA-256 of the complete API response from which the record was parsed.

Search results are serialized into these session artifacts:

```text
research/arxiv.json
research/zenodo.json
```

Each parsed record is also registered in the investigation ledger as a distinct `SourceRecord` evidence item with its canonical URI, retrieval time, repository identifier, parsed-record digest, response digest, authors, publication date, and DOI where available. The aggregate JSON artifact is independently content-addressed as session evidence.

The digest commits to the repository response, but Crosstalk currently does not retain the raw response body or full paper. Therefore the digest alone is not a reconstructable archival snapshot. Durable research should additionally preserve the retrieved payload, repository terms permitting, or store it in a content-addressed external archive.

## Evidence supplied to idea evolution

When native evolution is enabled, Crosstalk selects at most 32 session artifacts. For each artifact it:

1. hashes the complete artifact content with SHA-256;
2. takes a UTF-8-safe prefix capped at 8,000 bytes;
3. screens that prefix with `InjectionShield`;
4. enforces the size limit again after sanitization; and
5. supplies the evidence ID, full-content hash, and bounded excerpt independently to the variation and critic prompts.

This gives the critic access to the same cited material as the generator. It does not prove that citations support the candidate, and it may omit relevant material beyond the excerpt boundary.

## Evidence interpretation

Use repository records according to an evidence hierarchy:

| Item | What it supports |
|---|---|
| Search hit or title | That a potentially relevant record exists |
| Abstract or repository description | The authors' summarized claims |
| Full paper or dataset | Inspection of methods, results, and limitations |
| Reproduced analysis | Independent confirmation under a recorded environment |
| Multiple independent reproductions | Stronger evidence of generality and robustness |

None of these automatically proves a model's inference. Citation entailment, study quality, retraction/correction status, conflicts of interest, and independence between sources require separate checks.

## Prompt-injection boundary

Repository metadata and local documents are untrusted input. Injection screening reduces known prompt-like patterns before evolution, but no string filter is complete. Research text must be quoted as evidence rather than treated as instructions. The orchestrator and model prompts should preserve the priority order:

```text
system policy > user objective > orchestration protocol > untrusted evidence text
```

Do not place credentials, private data, or proprietary documents into a session unless every selected model provider and the local retention policy are approved for that data.

## Reproducible research checklist

For consequential conclusions, preserve:

- exact query and repository;
- retrieval timestamp and result limit;
- record identifiers, DOI, and canonical URLs;
- response or content hashes;
- full-text/data version when legally retained;
- code, environment manifest, random seeds, and hardware details;
- excluded studies and exclusion reasons;
- claim-to-source links with precise locators; and
- uncertainty, contradictions, and unresolved cruxes.

## Extension plan

The repository adapter should grow by evidence quality rather than source count alone:

1. store optional raw response snapshots in a content-addressed evidence store;
2. retrieve full text through repository-approved interfaces and preserve licenses;
3. add Crossref/OpenAlex metadata reconciliation and correction/retraction checks;
4. add patent search with jurisdiction, family, priority date, and claims provenance;
5. add claim-level citation entailment and source-independence graphs;
6. support corpus manifests for epigraphy, historical archives, and scientific datasets;
7. expand the implemented objective-feedback path to source-audit and citation-entailment evaluators.
