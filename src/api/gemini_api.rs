use crate::config::{GEMINI_GENERATE_URL, GEMINI_STREAM_URL};
use backon::{ExponentialBuilder, Retryable};
use reqwest::StatusCode;

pub struct GeminiApi;

impl GeminiApi {
    /// Low-level POST helper with automatic retries for network / HTTP errors
    pub async fn try_post_cli<T>(
        client: reqwest::Client,
        token: impl AsRef<str>,
        stream: bool,
        retry_policy: ExponentialBuilder,
        body: &T,
    ) -> Result<reqwest::Response, reqwest::Error>
    where
        T: serde::Serialize,
    {
        let url = if stream {
            GEMINI_STREAM_URL.clone()
        } else {
            GEMINI_GENERATE_URL.clone()
        };

        (|| async {
            let resp = client
                .post(url.clone())
                .bearer_auth(token.as_ref())
                .json(body)
                .send()
                .await?;
            match resp.error_for_status_ref() {
                Ok(_) => Ok(resp),
                Err(e)
                    if matches!(
                        e.status(),
                        Some(
                            StatusCode::TOO_MANY_REQUESTS
                                | StatusCode::UNAUTHORIZED
                                | StatusCode::FORBIDDEN
                        )
                    ) =>
                {
                    Ok(resp)
                }
                Err(e) => Err(e),
            }
        })
        .retry(retry_policy)
        .await
    }
}
