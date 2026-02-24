use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search DuckDuckGo and return structured results.
///
/// Uses DuckDuckGo's internal JSON API:
/// 1. POST to duckduckgo.com to obtain a `vqd` token
/// 2. GET results from links.duckduckgo.com/d.js with the token
pub async fn duckduckgo_search(query: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .cookie_store(true)
        .build()
        .context("failed to build HTTP client")?;

    // Step 1: POST to get vqd token
    let resp = client
        .post("https://duckduckgo.com/")
        .form(&[("q", query)])
        .send()
        .await
        .context("failed to fetch vqd token from DuckDuckGo")?;

    let body = resp
        .text()
        .await
        .context("failed to read DuckDuckGo response body")?;

    let vqd = extract_vqd(&body)
        .context("failed to extract vqd token from DuckDuckGo response")?;

    // Step 2: GET search results JSON
    let resp = client
        .get("https://links.duckduckgo.com/d.js")
        .query(&[("q", query), ("vqd", &vqd), ("kl", "wt-wt"), ("o", "json")])
        .send()
        .await
        .context("failed to fetch search results from DuckDuckGo")?;

    let results_body = resp
        .text()
        .await
        .context("failed to read search results body")?;

    parse_results(&results_body, max_results)
}

/// Extract the vqd token from the DuckDuckGo response body.
fn extract_vqd(body: &str) -> Option<String> {
    // Try vqd="..." pattern first (most common)
    let re = Regex::new(r#"vqd=["']([^"']+)["']"#).ok()?;
    if let Some(caps) = re.captures(body) {
        return Some(caps[1].to_string());
    }
    // Fallback: vqd=TOKEN& or vqd=TOKEN (no quotes)
    let re2 = Regex::new(r#"vqd=([^&"'>\s]+)"#).ok()?;
    if let Some(caps) = re2.captures(body) {
        return Some(caps[1].to_string());
    }
    None
}

/// Parse the JSON results from the d.js endpoint.
fn parse_results(body: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).context("failed to parse search results JSON")?;

    let results = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let html_tag_re = Regex::new(r"<[^>]+>").unwrap();

    let mut out = Vec::new();
    for item in results {
        let url = item.get("u").and_then(|v| v.as_str()).unwrap_or_default();
        let title = item.get("t").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = item.get("a").and_then(|v| v.as_str()).unwrap_or_default();

        // Skip entries without a URL (ads or navigation items)
        if url.is_empty() || url == "https://duckduckgo.com" {
            continue;
        }

        let clean_snippet = html_tag_re.replace_all(snippet, "").to_string();
        let clean_title = html_tag_re.replace_all(title, "").to_string();

        out.push(WebSearchResult {
            title: clean_title,
            url: url.to_string(),
            snippet: clean_snippet,
        });

        if out.len() >= max_results {
            break;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vqd_quoted() {
        let body = r#"something vqd="4-123456789" something"#;
        assert_eq!(extract_vqd(body), Some("4-123456789".to_string()));
    }

    #[test]
    fn test_extract_vqd_unquoted() {
        let body = "something vqd=4-abcdef&other=1";
        assert_eq!(extract_vqd(body), Some("4-abcdef".to_string()));
    }

    #[test]
    fn test_extract_vqd_missing() {
        let body = "no token here";
        assert_eq!(extract_vqd(body), None);
    }

    #[test]
    fn test_parse_results_basic() {
        let json = r#"{
            "results": [
                {"u": "https://example.com", "t": "Example", "a": "A <b>snippet</b> here"},
                {"u": "https://test.com", "t": "Test <i>Page</i>", "a": "Another snippet"}
            ]
        }"#;
        let results = parse_results(json, 5).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].snippet, "A snippet here");
        assert_eq!(results[1].title, "Test Page");
    }

    #[test]
    fn test_parse_results_max_limit() {
        let json = r#"{
            "results": [
                {"u": "https://a.com", "t": "A", "a": "one"},
                {"u": "https://b.com", "t": "B", "a": "two"},
                {"u": "https://c.com", "t": "C", "a": "three"}
            ]
        }"#;
        let results = parse_results(json, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_results_skips_empty_url() {
        let json = r#"{
            "results": [
                {"u": "", "t": "Empty", "a": "no url"},
                {"u": "https://duckduckgo.com", "t": "DDG", "a": "nav"},
                {"u": "https://real.com", "t": "Real", "a": "result"}
            ]
        }"#;
        let results = parse_results(json, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://real.com");
    }
}
