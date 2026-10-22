//! Resolve provider bases and the trailing `#` exact-endpoint marker once.

use url::Url;

use crate::{ModelError, ModelResult, ProtocolErrorKind};

/// Configuration diagnostics may identify a provider without copying URL-borne
/// credentials. Requests keep the original URL, including its routing query.
pub(crate) fn for_debug(endpoint: &Url) -> Url {
    let mut endpoint = endpoint.clone();
    let _ = endpoint.set_username("");
    let _ = endpoint.set_password(None);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    endpoint
}

pub(crate) fn resolve(endpoint: &Url, route: &[&str]) -> ModelResult<Url> {
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none() {
        return Err(ModelError::protocol(ProtocolErrorKind::InvalidEndpoint));
    }
    let mut endpoint = endpoint.clone();
    match endpoint.fragment() {
        // URL parsing preserves a final '#' as an empty fragment. Consume it
        // before preparing any HTTP request or durable request checkpoint.
        Some("") => {
            endpoint.set_fragment(None);
            return Ok(endpoint);
        }
        Some(_) => return Err(ModelError::protocol(ProtocolErrorKind::InvalidEndpoint)),
        None => {}
    }

    let path = endpoint.path().trim_end_matches('/');
    let suffix = format!("/{}", route.join("/"));
    if path.ends_with(&suffix) {
        return Ok(endpoint);
    }
    // A bare origin uses the standard /v1 prefix. An explicit base path is
    // retained, including versioned or gateway-specific prefixes.
    let is_origin = path.is_empty();
    let trailing_slashes = endpoint.path().len() - path.len();
    let mut segments = endpoint
        .path_segments_mut()
        .map_err(|_| ModelError::protocol(ProtocolErrorKind::InvalidEndpoint))?;
    for _ in 0..trailing_slashes.max(1) {
        segments.pop_if_empty();
    }
    if is_origin {
        segments.push("v1");
    }
    segments.extend(route.iter().copied());
    drop(segments);
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnthropicConfig, AnthropicModel, Model, ModelMessage, ModelRequest, OpenAiConfig,
        OpenAiModel, OpenAiResponsesConfig, OpenAiResponsesModel,
    };

    fn models(base: &str) -> [Box<dyn Model>; 3] {
        let url = Url::parse(base).unwrap();
        [
            Box::new(OpenAiModel::new(OpenAiConfig::new(
                url.clone(),
                "synthetic-key",
                "model",
            ))),
            Box::new(OpenAiResponsesModel::new(OpenAiResponsesConfig::new(
                url.clone(),
                "synthetic-key",
                "model",
            ))),
            Box::new(AnthropicModel::new(AnthropicConfig::new(
                url,
                "synthetic-key",
                "model",
            ))),
        ]
    }

    fn assert_endpoints(base: &str, expected: [&str; 3]) {
        let request = ModelRequest::new(vec![ModelMessage::user_text("hello")]);
        for (model, expected) in models(base).into_iter().zip(expected) {
            assert_eq!(
                model.prepare_request(&request).unwrap().endpoint().as_str(),
                expected,
                "{base}"
            );
            let prepared = model.prepare_stream(request.clone()).unwrap();
            assert_eq!(
                prepared.checkpoint()["endpoint"].as_str(),
                Some(expected),
                "{base}"
            );
        }
    }

    #[test]
    fn provider_requests_complete_origin_and_versioned_bases() {
        for base in [
            "https://www.aidawan.fun",
            "https://www.aidawan.fun/",
            "https://www.aidawan.fun/v1",
            "https://www.aidawan.fun/v1/",
            "https://www.aidawan.fun/v1///",
        ] {
            assert_endpoints(
                base,
                [
                    "https://www.aidawan.fun/v1/chat/completions",
                    "https://www.aidawan.fun/v1/responses",
                    "https://www.aidawan.fun/v1/messages",
                ],
            );
        }
    }

    #[test]
    fn provider_requests_preserve_gateway_paths_and_queries() {
        assert_endpoints(
            "https://gateway.test/proxy%20path/v1/?api-version=2026-01&x=a%2Fb",
            [
                "https://gateway.test/proxy%20path/v1/chat/completions?api-version=2026-01&x=a%2Fb",
                "https://gateway.test/proxy%20path/v1/responses?api-version=2026-01&x=a%2Fb",
                "https://gateway.test/proxy%20path/v1/messages?api-version=2026-01&x=a%2Fb",
            ],
        );
        assert_endpoints(
            "http://localhost:8080/custom/v2",
            [
                "http://localhost:8080/custom/v2/chat/completions",
                "http://localhost:8080/custom/v2/responses",
                "http://localhost:8080/custom/v2/messages",
            ],
        );
    }

    #[test]
    fn a_trailing_hash_uses_the_exact_endpoint_for_every_protocol() {
        for (base, exact) in [
            ("https://www.aidawan.fun#", "https://www.aidawan.fun/"),
            ("https://www.aidawan.fun/v1#", "https://www.aidawan.fun/v1"),
            (
                "https://gateway.test/custom/invoke/#",
                "https://gateway.test/custom/invoke/",
            ),
            (
                "https://gateway.test/custom%2Froute?x=a%2Fb&api-version=2026-01#",
                "https://gateway.test/custom%2Froute?x=a%2Fb&api-version=2026-01",
            ),
        ] {
            assert_endpoints(base, [exact; 3]);
        }
    }

    #[test]
    fn complete_protocol_endpoints_are_not_appended_twice() {
        let request = ModelRequest::new(vec![ModelMessage::user_text("hello")]);
        for (index, route) in ["chat/completions", "responses", "messages"]
            .into_iter()
            .enumerate()
        {
            let base = format!("https://gateway.test/v1/{route}?x=1");
            assert_eq!(
                models(&base)[index]
                    .prepare_request(&request)
                    .unwrap()
                    .endpoint()
                    .as_str(),
                base
            );
        }
    }

    #[test]
    fn fragments_other_than_the_final_marker_are_rejected() {
        let request = ModelRequest::new(vec![ModelMessage::user_text("hello")]);
        for base in [
            "https://gateway.test/v1#fragment",
            "https://gateway.test/v1##",
            "ftp://gateway.test/v1",
        ] {
            for model in models(base) {
                assert!(model.prepare_request(&request).is_err(), "{base}");
                assert!(model.prepare_stream(request.clone()).is_err(), "{base}");
            }
        }
    }
}
