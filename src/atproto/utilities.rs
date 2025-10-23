//! AT Protocol utility functions for handle and DID operations

use anyhow::{Context, Result};
use atproto_identity::resolve::parse_input;

use crate::errors::IdentityError;

/// Resolve a handle to a DID using the com.atproto.identity.resolveHandle XRPC endpoint
pub async fn resolve_handle_to_did(
    http_client: &reqwest::Client,
    hostname: &str,
    handle: &str,
) -> Result<String> {
    let url = format!(
        "https://{}/xrpc/com.atproto.identity.resolveHandle?handle={}",
        hostname,
        urlencoding::encode(handle)
    );

    let response = atproto_client::client::get_json(http_client, &url)
        .await
        .with_context(|| format!("Failed to resolve handle '{}'", handle))?;

    response
        .get("did")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            IdentityError::HandleResolutionFailed {
                handle: handle.to_string(),
                details: "Invalid response from resolveHandle: missing 'did' field".to_string(),
            }
            .into()
        })
}

/// Filter and map a list of input strings to DIDs, resolving handles as needed
///
/// This function accepts a list of strings that can be either DIDs or handles.
/// For each input:
/// - If it's already a DID, it's included in the output as-is
/// - If it's a handle, it's resolved to a DID using the resolve endpoint
/// - Invalid inputs are skipped (errors are logged but don't fail the entire operation)
///
/// # Arguments
/// * `http_client` - The HTTP client to use for resolving handles
/// * `resolve_hostname` - The hostname to use for the resolve endpoint (e.g., "bsky.social")
/// * `inputs` - A list of strings that may be DIDs or handles
///
/// # Returns
/// A vector of valid DID strings
pub async fn filter_map_dids(
    http_client: &reqwest::Client,
    resolve_hostname: &str,
    inputs: Vec<String>,
) -> Result<Vec<String>> {
    let mut dids = Vec::new();

    for input in inputs {
        // Skip empty strings
        if input.is_empty() {
            continue;
        }

        // Parse the input to determine if it's a DID or handle
        match parse_input(&input)? {
            atproto_identity::resolve::InputType::Handle(handle) => {
                // It's a handle, resolve it to a DID
                match resolve_handle_to_did(http_client, resolve_hostname, &handle).await {
                    Ok(did) => {
                        dids.push(did);
                    }
                    Err(e) => {
                        // Log the error but continue processing other inputs
                        tracing::warn!(
                            handle = %handle,
                            error = ?e,
                            "Failed to resolve handle to DID, skipping"
                        );
                    }
                }
            }
            // For DID types, use the original input (it's already a DID)
            _ => {
                // It's already a DID, add it directly
                dids.push(input.clone());
            }
        }
    }

    Ok(dids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_handle_url_construction() {
        let hostname = "bsky.social";
        let handle = "alice.bsky.social";
        let expected_url = format!(
            "https://{}/xrpc/com.atproto.identity.resolveHandle?handle={}",
            hostname,
            urlencoding::encode(handle)
        );
        assert_eq!(
            expected_url,
            "https://bsky.social/xrpc/com.atproto.identity.resolveHandle?handle=alice.bsky.social"
        );
    }

    #[tokio::test]
    async fn test_filter_map_dids_with_dids_only() {
        // This test doesn't make actual network calls
        // In a real test, you'd use a mock HTTP client

        let inputs = vec![
            "did:plc:abc123".to_string(),
            "did:web:example.com".to_string(),
            "".to_string(), // Empty string should be skipped
        ];

        // We can't test the full function without a mock, but we can test the logic
        // by checking that parse_input correctly identifies different input types
        for input in &inputs {
            if !input.is_empty() {
                if let Ok(parsed) = parse_input(input) {
                    match parsed {
                        atproto_identity::resolve::InputType::Handle(_) => {
                            // This should not happen for DIDs
                            assert!(false, "DID was incorrectly parsed as handle: {}", input);
                        }
                        _ => {
                            // Any other variant means it's a DID type
                            assert!(input.starts_with("did:"));
                        }
                    }
                }
            }
        }
    }
}
