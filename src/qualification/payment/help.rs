//! Private, case-correlated help/cache observations; never payment authority.
use super::*;
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpCache {
    Fetch,
    Hit,
    Shared,
}

pub async fn record(
    started: bool,
    cache: Option<HelpCache>,
    result: Option<&Result<String>>,
) -> Result<()> {
    let Ok(context) = CASE.try_with(Clone::clone) else {
        return Ok(());
    };
    let detail = match result {
        Some(Ok(text)) => json!({"status":"succeeded","bytes":text.len()}),
        Some(Err(error)) => {
            let status = error.chain().find_map(|cause| {
                cause
                    .downcast_ref::<reqwest::Error>()
                    .and_then(|e| e.status())
                    .map(|s| s.as_u16())
            });
            json!({"status":"failed","failure_category":super::super::failure_category(error),"http_status":status})
        }
        None => json!({"status":"started"}),
    };
    ensure!(
        started == result.is_none(),
        "invalid help observation transition"
    );
    tokio::task::spawn_blocking(move || {
        let mut db=connection(&context.guard.binding)?;
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claimed: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_claim' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?3)",
            params![context.guard.binding.run,context.case,context.guard.binding.session],|r|r.get(0))?;
        ensure!(claimed,"help observation has no application claim");
        let count:i64=tx.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='application_help' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2",
            params![context.guard.binding.run,context.case],|r|r.get(0))?;
        ensure!(count==if started {0} else {1},"duplicate/out-of-order help evidence; at most one start and completion per case");
        let event=json!({"case":context.case,"session":context.guard.binding.session,"observed_micros":context.elapsed_micros(),"cache":cache,"result":detail});
        tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_help',?2,?3)",params![context.guard.binding.run,event.to_string(),now()?])?;
        tx.commit()?;Ok(())
    }).await.context("help evidence worker stopped")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_state::BoundTool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn bound(url: String) -> BoundTool {
        let cfg = crate::catalog::Config {
            help_url: Some(url),
            ..Default::default()
        };
        crate::catalog_state::bind(vec![(
            crate::catalog::build_tools(
                &cfg,
                &json!({"paths":{"/unused":{"get":{"responses":{}}}}}),
                "test",
            )
            .unwrap()
            .pop()
            .unwrap(),
            crate::payment::PaidClient::unsigned().with_transport(crate::network::HttpPolicy {
                allow_http1: true,
                allow_tls12: false,
            }),
            String::new(),
        )])
        .pop()
        .unwrap()
    }
    async fn fixture() -> (tempfile::TempDir, Claim, Connection) {
        let (dir, guard, db) = crate::qualification::tests::fixture();
        let claim = crate::qualification::claim_with(
            guard,
            "listener",
            "read",
            &crate::qualification::tests::args(),
            &crate::qualification::tests::id(),
        )
        .await
        .unwrap();
        (dir, claim, db)
    }
    fn events(db: &Connection) -> Vec<Value> {
        db.prepare("SELECT detail FROM events WHERE kind='application_help' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
            .collect()
    }
    #[tokio::test]
    async fn real_help_failure_is_not_cached_and_success_is_reused() {
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move || {
                let seen = seen.clone();
                async move {
                    if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable")
                    } else {
                        (axum::http::StatusCode::OK, "usage documentation")
                    }
                }
            }),
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tool = bound(format!("http://{}/", socket.local_addr().unwrap()));
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        for (cache, status) in [
            ("fetch", "failed"),
            ("fetch", "succeeded"),
            ("hit", "succeeded"),
        ] {
            let (_dir, claim, db) = fixture().await;
            let output = claim
                .invoke(tool.invoke_output(&serde_json::Map::new()))
                .await;
            assert_eq!(output.is_ok(), status == "succeeded");
            let rows = events(&db);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1]["cache"], cache);
            assert_eq!(rows[1]["result"]["status"], status);
            if status == "failed" {
                assert_eq!(rows[1]["result"]["failure_category"], "http_status");
                assert_eq!(rows[1]["result"]["http_status"], 503);
            }
            assert!(
                !serde_json::to_string(&rows)
                    .unwrap()
                    .contains("usage documentation")
            );
        }
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn concurrent_help_observes_shared_initialization_and_cancellation_stays_incomplete() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (a, b) = (entered.clone(), release.clone());
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move || {
                let (a, b) = (a.clone(), b.clone());
                async move {
                    a.notify_one();
                    b.notified().await;
                    "docs"
                }
            }),
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tool = bound(format!("http://{}/", socket.local_addr().unwrap()));
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let (_dir, claim, db) = fixture().await;
        let copy = tool.clone();
        let first = tokio::spawn(async move {
            claim
                .invoke(copy.invoke_output(&serde_json::Map::new()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let (_dir2, claim2, db2) = fixture().await;
        let copy = tool.clone();
        let second = tokio::spawn(async move {
            claim2
                .invoke(copy.invoke_output(&serde_json::Map::new()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while events(&db2).is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        release.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(events(&db)[1]["cache"], "fetch");
        assert_eq!(events(&db2)[1]["cache"], "shared");
        let (_dir3, claim3, db3) = fixture().await;
        let mut fresh = tool;
        fresh.help = Arc::new(tokio::sync::OnceCell::new());
        let pending = tokio::spawn(async move {
            claim3
                .invoke(fresh.invoke_output(&serde_json::Map::new()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        pending.abort();
        let _ = pending.await;
        assert_eq!(events(&db3).len(), 1);
        assert_eq!(events(&db3)[0]["result"]["status"], "started");
        server.abort();
        let _ = server.await;
    }
}
