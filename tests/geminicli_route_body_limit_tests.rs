use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

#[tokio::test]
async fn geminicli_response_route_returns_413_for_oversized_body() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();

    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!(
        "pollux-geminicli-body-limit-{}-{}.sqlite",
        std::process::id(),
        nanos
    ));

    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();
    // Keep test behavior stable regardless of the repo's runtime `config.toml`.
    let model = pollux::config::CONFIG
        .geminicli()
        .model_list
        .first()
        .cloned()
        .unwrap_or_else(|| "gemini-2.5-pro".to_string());
    cfg.providers.geminicli.model_list = vec![model.clone()];

    let providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state = pollux::server::router::PolluxState::new(
        providers,
        pollux_key.clone(),
        cfg.basic.insecure_cookie,
    );
    let app = pollux::server::router::pollux_router(state);

    let oversized_input = "a".repeat(30 * 1024 * 1024 + 1024);
    let oversized_payload = format!(r#"{{"input":"{oversized_input}"}}"#);
    let uri = format!("/geminicli/v1beta/models/{model}:generateContent");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from(oversized_payload))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert!(body_str.contains(r#""code":413"#));
    assert!(body_str.contains(r#""status":"PAYLOAD_TOO_LARGE""#));
    assert!(body_str.contains(r#""message":"request body too large""#));

    let _ = fs::remove_file(&temp_path);
}
