//! The HTTP layer the debrid providers use in production.
//!
//! It is a thin adapter on purpose: `segmenta-core` describes *what* to send and parses the
//! answer, so the only thing here is "perform this request and give me status and body".
//! Keeping it behind a trait lets the seam tests inject canned answers without pretending
//! that `api.real-debrid.com` lives on localhost.

use segmenta_core::debrid::{DebridError, DebridHttp, DebridRequest};

/// Requests over `reqwest`, with a timeout so a provider that never answers does not hang a
/// download forever.
pub struct ReqwestDebrid;

#[async_trait::async_trait]
impl DebridHttp for ReqwestDebrid {
    async fn send(&self, request: &DebridRequest) -> Result<(u16, String), DebridError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| DebridError::Unreachable)?;
        let mut builder = match request.method {
            "GET" => client.get(&request.url),
            _ => client.post(&request.url),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .body(request.body.clone())
            .send()
            .await
            .map_err(|_| DebridError::Unreachable)?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Ok((status, body))
    }
}
