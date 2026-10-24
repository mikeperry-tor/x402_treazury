//! Only fixed, authored explanations and numeric HTTP status may enter logs.
use super::LoadStage;

#[derive(Debug)]
pub(crate) struct Problem(pub &'static str);
impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Problem {}

pub(crate) struct Failure {
    pub stage: &'static str,
    pub reason: &'static str,
    pub http_status: Option<u16>,
}
impl Failure {
    pub fn from_error(error: &anyhow::Error, fallback_stage: &'static str) -> Self {
        let stage = error
            .downcast_ref::<LoadStage>()
            .map(|stage| stage.label())
            .unwrap_or(fallback_stage);
        let http = error.downcast_ref::<reqwest::Error>();
        let http_status = http.and_then(reqwest::Error::status).map(|s| s.as_u16());
        let reason = if let Some(problem) = error.downcast_ref::<Problem>() {
            problem.0
        } else if let Some(configuration) = error.downcast_ref::<super::ToolConfigurationError>() {
            configuration.reason
        } else if error.is::<crate::limits::DownloadLimit>() {
            "The catalog exceeds the configured download size limit; no partial catalog was accepted"
        } else if error.is::<super::IncompleteCatalog>() {
            "The server returned only part of the catalog; a complete document is required"
        } else if http_status.is_some() {
            "The catalog server returned an unsuccessful HTTP status"
        } else if let Some(http) = http {
            if http.is_timeout() && http.is_connect() {
                "The connection to the catalog server timed out"
            } else if http.is_timeout() {
                "The catalog server stopped sending data before the response was complete"
            } else if http.is_connect() {
                "Could not connect to the catalog server or configured proxy"
            } else {
                "The catalog HTTP request failed before a complete response was received"
            }
        } else {
            match stage {
                "local_read" => {
                    "Could not read the catalog file; check that it exists and is readable"
                }
                "parse" => "The catalog document is not valid JSON",
                "configuration" => {
                    "Could not prepare the catalog configuration; check provider settings and the API base URL"
                }
                "generation" => {
                    "Could not create tools from the catalog; check its API schemas and provider filters"
                }
                "selection" => {
                    "The catalog could not be used by the configured listeners; check tool selectors, reserved names and cover settings"
                }
                "publication" => {
                    "Could not publish the catalog tools; the previous tool inventory is still available to ready endpoints"
                }
                "body" => "Could not read the complete catalog response",
                _ => "Could not obtain the catalog document; check network and discovery settings",
            }
        };
        Self {
            stage,
            reason,
            http_status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_fixed_and_never_include_untrusted_error_text() {
        for (stage, expected) in [
            (LoadStage::LocalRead, "Could not read the catalog file"),
            (LoadStage::Parse, "not valid JSON"),
            (LoadStage::Configuration, "provider settings"),
            (LoadStage::Generation, "API schemas"),
            (LoadStage::Selection, "configured listeners"),
            (LoadStage::Publication, "previous tool inventory"),
        ] {
            let error = anyhow::anyhow!("secret URL https://user:password@example.com/token")
                .context(stage);
            let failure = Failure::from_error(&error, "fetch_parse");
            assert_eq!(failure.stage, stage.label());
            assert!(failure.reason.contains(expected), "{}", failure.reason);
            assert!(!failure.reason.contains("password"));
        }
        let error = anyhow::anyhow!("private schema prose")
            .context(Problem("The catalog contains an invalid HTTP header name"))
            .context(LoadStage::Generation);
        assert_eq!(
            Failure::from_error(&error, "fetch_parse").reason,
            "The catalog contains an invalid HTTP header name"
        );
    }

    #[test]
    fn detailed_provider_errors_and_safe_log_reasons_remain_separate() {
        use super::super::{Config, ToolConfigurationError, build_tools};
        let document = serde_json::json!({"paths":{"/read":{"get":{}}}});
        for (config, detailed, safe) in [
            (
                Config {
                    include_tools: vec!["private_selector".into()],
                    ..Default::default()
                },
                "unknown provider tool selector private_selector",
                "Provider tool-name filters",
            ),
            (
                Config {
                    include: vec!["/private_path".into()],
                    ..Default::default()
                },
                "no operations matched",
                "Provider path, method or tag filters",
            ),
        ] {
            let error = build_tools(&config, &document, "fixture").unwrap_err();
            assert!(error.to_string().contains(detailed));
            let error = error.context(LoadStage::Generation);
            assert!(error.is::<ToolConfigurationError>());
            let failure = Failure::from_error(&error, "fetch_parse");
            assert_eq!(failure.stage, "generation");
            assert!(failure.reason.contains(safe));
            assert!(
                !failure.reason.contains("private_selector")
                    && !failure.reason.contains("private_path")
            );
        }
    }

    #[tokio::test]
    async fn invalid_cache_revalidation_explains_why_no_catalog_was_accepted() {
        use crate::http_cache::{Entry, Metadata};
        use reqwest::header::{CACHE_CONTROL, ETAG, HeaderMap};
        let headers = HeaderMap::from_iter([
            (CACHE_CONTROL, "max-age=600".parse().unwrap()),
            (ETAG, "\"original\"".parse().unwrap()),
        ]);
        let entry = Entry {
            metadata: Metadata::from_headers(&headers, std::time::Duration::ZERO, None).unwrap(),
            data: b"{}".to_vec(),
        };
        let changed = HeaderMap::from_iter([(ETAG, "\"private-upstream-tag\"".parse().unwrap())]);
        for (cached, expected) in [
            (None, "there is no cached document"),
            (Some(&entry), "does not match the cached document"),
        ] {
            let error = super::super::revalidate_cached_catalog(&changed, cached, None)
                .await
                .unwrap_err();
            let failure = Failure::from_error(&error, "fetch_parse");
            assert_eq!(failure.stage, "headers");
            assert!(failure.reason.contains(expected));
            assert!(!failure.reason.contains("private-upstream-tag"));
        }
    }
}
