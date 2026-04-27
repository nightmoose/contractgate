use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use serde::{de::DeserializeOwned, Serialize};

/// Exit codes for client-level errors (network, auth, server).
pub const EXIT_CLIENT_ERROR: i32 = 10;
pub const EXIT_AUTH_ERROR: i32 = 11;
pub const EXIT_NOT_FOUND: i32 = 12;

/// Thin blocking HTTP wrapper around the ContractGate gateway.
pub struct GatewayClient {
    base_url: String,
    api_key: String,
    inner: Client,
}

impl GatewayClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Result<Self> {
        let inner = Client::builder()
            .use_rustls_tls()
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            inner,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth_header(&self) -> String {
        self.api_key.clone()
    }

    /// POST JSON body, deserialize response.
    pub fn post<B: Serialize, R: DeserializeOwned>(&self, path: &str, body: &B) -> Result<R> {
        let resp = self
            .inner
            .post(self.url(path))
            .header("x-api-key", self.auth_header())
            .json(body)
            .send()
            .with_context(|| format!("POST {path}"))?;
        self.decode(resp)
    }

    /// GET, deserialize response.
    pub fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let resp = self
            .inner
            .get(self.url(path))
            .header("x-api-key", self.auth_header())
            .send()
            .with_context(|| format!("GET {path}"))?;
        self.decode(resp)
    }

    fn decode<R: DeserializeOwned>(&self, resp: Response) -> Result<R> {
        let status = resp.status();
        if status == 401 {
            bail!("authentication failed (401) — check CONTRACTGATE_API_KEY");
        }
        if status == 404 {
            bail!("not found (404)");
        }
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("server error {status}: {body}");
        }
        let val: R = resp.json().context("decoding JSON response")?;
        Ok(val)
    }
}
