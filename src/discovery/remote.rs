//! Concurrent, owned directory reads with native responses and aggregate liabilities.
use super::{Manager, directory, policy::RemoteDirectory};
use crate::{discovery_relay::document::SharedFailure, output::ToolOutput};
use anyhow::{Result, ensure};
use serde_json::{Map, Value, json};
use std::sync::Arc;

impl Manager {
    pub async fn invoke_search_remote(
        self: &Arc<Self>,
        owner: &str,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput> {
        let policy = self.policy(owner);
        ensure!(policy.remote_search(), "remote_provider_search_disabled");
        directory::validate_search(
            args,
            policy
                .directories
                .iter()
                .any(|d| *d != RemoteDirectory::X402List),
        )?;
        let client = self.source_client(owner)?;
        let (manager, owner, args) = (self.clone(), owner.to_owned(), args.clone());
        client
            .run_owned(async move {
                let children = policy.directories.into_iter().map(|directory| {
                    let (manager, owner, args) = (&manager, &owner, &args);
                    async move {
                        let result = match directory {
                            RemoteDirectory::Coinbase => {
                                manager.invoke_coinbase_search(owner, args, false).await
                            }
                            RemoteDirectory::CoinbaseCurated => {
                                manager.invoke_coinbase_search(owner, args, true).await
                            }
                            RemoteDirectory::X402List => {
                                manager
                                    .invoke_directory(owner, directory::SEARCH, args)
                                    .await
                            }
                        };
                        (directory, result)
                    }
                });
                Ok(aggregate(futures_util::future::join_all(children).await))
            })
            .await?
    }
}

pub(super) fn aggregate(results: Vec<(RemoteDirectory, Result<ToolOutput>)>) -> ToolOutput {
    let mut values = Map::new();
    let mut output = ToolOutput::text(String::new());
    for (directory, result) in results {
        let result = result.and_then(|child| {
            output.payment_may_have_occurred |= child.payment_may_have_occurred;
            output.advisories.extend(child.advisories);
            crate::payment::with_payment_context(
                child.payment_may_have_occurred,
                (|| {
                    ensure!(child.images.is_empty(), "directory response is not JSON");
                    let value: Value = serde_json::from_str(&child.text)
                        .map_err(|_| anyhow::anyhow!("directory response invalid JSON"))?;
                    ensure!(
                        value.is_object(),
                        "directory response must be a JSON object"
                    );
                    Ok(value)
                })(),
            )
        });
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let failure = SharedFailure::capture(&error, "directory_request_failed");
                let possible = failure.possible_payment();
                output.payment_may_have_occurred |= possible;
                output.advisories.push(format!(
                    "{} search failed; sibling results remain available.{}",
                    directory.key(),
                    if possible {
                        " A payment may have occurred; do not automatically repeat this search."
                    } else {
                        ""
                    }
                ));
                let mut value = json!({"local_error":{"kind":"directory_request_failed","payment_may_have_occurred":possible}});
                if let Some(status) = failure.status() {
                    value["local_error"]["http_status"] = json!(status);
                }
                value
            }
        };
        values.insert(directory.key().into(), value);
        if directory == RemoteDirectory::X402List {
            output
                .advisories
                .push("Attribute x402 List directory data to x402-list.com (CC BY 4.0).".into());
        }
    }
    // Serializing a Value to String is infallible; no later conversion can drop child evidence.
    output.text = Value::Object(values).to_string();
    if output.payment_may_have_occurred {
        output.advisories.push("This search may have authorized multiple directory or relay charges; do not automatically repeat it.".into());
    }
    output
}
