//! Provenance-preserving access to public research repositories.

use anyhow::{Context, Result, anyhow};
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

const MAX_RESULTS: usize = 25;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchRepository {
    Arxiv,
    Zenodo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResearchRecord {
    pub repository: ResearchRepository,
    pub identifier: String,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
    pub published: Option<String>,
    pub canonical_url: String,
    pub doi: Option<String>,
    /// Hash of the repository response from which this record was parsed.
    pub response_sha256: String,
    pub retrieved_at: u64,
}

pub struct ResearchClient {
    client: Client,
    zenodo_token: Option<String>,
}

impl ResearchClient {
    pub fn new(zenodo_token: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!(
                "crosstalk/",
                env!("CARGO_PKG_VERSION"),
                " research-client"
            ))
            .build()?;
        Ok(Self {
            client,
            zenodo_token,
        })
    }

    pub async fn search(
        &self,
        repository: ResearchRepository,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ResearchRecord>> {
        if query.trim().is_empty() {
            return Err(anyhow!("research query must not be empty"));
        }
        let limit = limit.clamp(1, MAX_RESULTS);
        match repository {
            ResearchRepository::Arxiv => self.search_arxiv(query, limit).await,
            ResearchRepository::Zenodo => self.search_zenodo(query, limit).await,
        }
    }

    async fn search_arxiv(&self, query: &str, limit: usize) -> Result<Vec<ResearchRecord>> {
        let mut url = Url::parse("https://export.arxiv.org/api/query")?;
        url.query_pairs_mut()
            .append_pair("search_query", &format!("all:{query}"))
            .append_pair("start", "0")
            .append_pair("max_results", &limit.to_string())
            .append_pair("sortBy", "relevance");
        let bytes = self.fetch(url, None).await?;
        parse_arxiv(&bytes)
    }

    async fn search_zenodo(&self, query: &str, limit: usize) -> Result<Vec<ResearchRecord>> {
        let mut url = Url::parse("https://zenodo.org/api/records")?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("size", &limit.to_string())
            .append_pair("sort", "bestmatch");
        let auth = self.zenodo_token.as_deref();
        let bytes = self.fetch(url, auth).await?;
        parse_zenodo(&bytes)
    }

    async fn fetch(&self, url: Url, bearer: Option<&str>) -> Result<Vec<u8>> {
        let mut request = self
            .client
            .get(url)
            .header("Accept", "application/json, application/atom+xml;q=0.9");
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?.error_for_status()?;
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(anyhow!("research repository response exceeds size limit"));
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!("research repository response exceeds size limit"));
        }
        Ok(bytes.to_vec())
    }
}

fn parse_arxiv(bytes: &[u8]) -> Result<Vec<ResearchRecord>> {
    let hash = response_hash(bytes);
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut records = Vec::new();
    let mut current: Option<ArxivEntry> = None;
    let mut tag = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_vec();
                if name == b"entry" {
                    current = Some(ArxivEntry::default());
                }
                tag = name;
            }
            Ok(Event::Text(text)) if current.is_some() => {
                let value = text.unescape()?.into_owned();
                let entry = current.as_mut().expect("checked above");
                match tag.as_slice() {
                    b"id" => entry.id.push_str(&value),
                    b"title" => entry.title.push_str(&value),
                    b"summary" => entry.summary.push_str(&value),
                    b"published" => entry.published.push_str(&value),
                    b"name" => entry.authors.push(value),
                    b"doi" => entry.doi = Some(value),
                    _ => {}
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"entry" => {
                if let Some(entry) = current.take() {
                    let identifier = entry.id.rsplit('/').next().unwrap_or(&entry.id).to_string();
                    records.push(ResearchRecord {
                        repository: ResearchRepository::Arxiv,
                        identifier,
                        title: normalize_space(&entry.title),
                        authors: entry.authors,
                        abstract_text: normalize_space(&entry.summary),
                        published: (!entry.published.is_empty()).then_some(entry.published),
                        canonical_url: entry.id,
                        doi: entry.doi,
                        response_sha256: hash.clone(),
                        retrieved_at: now(),
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error).context("invalid arXiv Atom response"),
            _ => {}
        }
    }
    Ok(records)
}

#[derive(Default)]
struct ArxivEntry {
    id: String,
    title: String,
    summary: String,
    published: String,
    authors: Vec<String>,
    doi: Option<String>,
}

fn parse_zenodo(bytes: &[u8]) -> Result<Vec<ResearchRecord>> {
    let body: serde_json::Value =
        serde_json::from_slice(bytes).context("invalid Zenodo JSON response")?;
    let hits = body
        .pointer("/hits/hits")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("Zenodo response omitted hits.hits"))?;
    let hash = response_hash(bytes);
    Ok(hits
        .iter()
        .filter_map(|hit| {
            let metadata = hit.get("metadata")?;
            let identifier = hit
                .get("id")?
                .as_u64()
                .map(|v| v.to_string())
                .or_else(|| hit.get("id")?.as_str().map(str::to_string))?;
            let title = metadata
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Untitled")
                .to_string();
            let authors = metadata
                .get("creators")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect();
            let doi = metadata
                .get("doi")
                .or_else(|| hit.get("doi"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let canonical_url = hit
                .pointer("/links/html")
                .or_else(|| hit.pointer("/links/self_html"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://zenodo.org/records/{identifier}"));
            Some(ResearchRecord {
                repository: ResearchRepository::Zenodo,
                identifier,
                title,
                authors,
                abstract_text: metadata
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                published: metadata
                    .get("publication_date")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                canonical_url,
                doi,
                response_sha256: hash.clone(),
                retrieved_at: now(),
            })
        })
        .collect())
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn response_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arxiv_atom_with_provenance() {
        let xml = br#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><entry><id>https://arxiv.org/abs/2601.01234v1</id><title>A  useful result</title><summary>Testable prediction.</summary><published>2026-01-03T00:00:00Z</published><author><name>Ada Example</name></author><arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.1/example</arxiv:doi></entry></feed>"#;
        let records = parse_arxiv(xml).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].identifier, "2601.01234v1");
        assert_eq!(records[0].title, "A useful result");
        assert_eq!(records[0].authors, ["Ada Example"]);
        assert_eq!(records[0].response_sha256.len(), 64);
    }

    #[test]
    fn parses_zenodo_records_with_canonical_identity() {
        let json = br#"{"hits":{"hits":[{"id":42,"doi":"10.5281/zenodo.42","links":{"html":"https://zenodo.org/records/42"},"metadata":{"title":"Dataset","description":"Evidence","publication_date":"2026-02-01","creators":[{"name":"Turing, A."}]}}]}}"#;
        let records = parse_zenodo(json).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].identifier, "42");
        assert_eq!(records[0].doi.as_deref(), Some("10.5281/zenodo.42"));
        assert_eq!(records[0].authors, ["Turing, A."]);
    }
}
