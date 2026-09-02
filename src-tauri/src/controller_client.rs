use std::collections::BTreeMap;
use std::path::PathBuf;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, Uri};
use serde::de::DeserializeOwned;
use serde::Serialize;
use webtop_contracts::{ApiError, ErrorCode};

pub struct ControllerClient {
    socket: PathBuf,
}

impl ControllerClient {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    pub async fn request<B: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<B>,
    ) -> Result<R, ApiError> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| invalid_request())?;
        let bytes = body
            .map(|body| serde_json::to_vec(&body))
            .transpose()
            .map_err(|_| invalid_request())?
            .unwrap_or_default();
        let request = Request::builder()
            .method(method)
            .uri(Uri::new(&self.socket, path))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|_| invalid_request())?;
        let client: Client<_, Full<Bytes>> = Client::unix();
        let response = client.request(request).await.map_err(|_| unavailable())?;
        let status = response.status();
        let response_bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|_| unavailable())?
            .to_bytes();
        if !status.is_success() {
            let error = serde_json::from_slice(&response_bytes).unwrap_or_else(|_| unavailable());
            return Err(error);
        }
        if status == StatusCode::NO_CONTENT || response_bytes.is_empty() {
            return serde_json::from_value(serde_json::Value::Null).map_err(|_| invalid_request());
        }
        serde_json::from_slice(&response_bytes).map_err(|_| unavailable())
    }

    pub async fn request_ndjson<B, T, F>(
        &self,
        method: &str,
        path: &str,
        body: Option<B>,
        mut on_item: F,
    ) -> Result<(), ApiError>
    where
        B: Serialize,
        T: DeserializeOwned,
        F: FnMut(T) -> Result<(), ApiError>,
    {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| invalid_request())?;
        let bytes = body
            .map(|body| serde_json::to_vec(&body))
            .transpose()
            .map_err(|_| invalid_request())?
            .unwrap_or_default();
        let request = Request::builder()
            .method(method)
            .uri(Uri::new(&self.socket, path))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|_| invalid_request())?;
        let client: Client<_, Full<Bytes>> = Client::unix();
        let response = client.request(request).await.map_err(|_| unavailable())?;
        let status = response.status();
        if !status.is_success() {
            let response_bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|_| unavailable())?
                .to_bytes();
            return Err(serde_json::from_slice(&response_bytes).unwrap_or_else(|_| unavailable()));
        }

        let mut response_body = response.into_body();
        let mut buffer = Vec::new();
        while let Some(frame) = response_body.frame().await {
            let frame = frame.map_err(|_| unavailable())?;
            let Ok(data) = frame.into_data() else {
                continue;
            };
            buffer.extend_from_slice(&data);
            while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=newline).collect::<Vec<_>>();
                if line[..line.len() - 1].iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let item =
                    serde_json::from_slice(&line[..line.len() - 1]).map_err(|_| unavailable())?;
                on_item(item)?;
            }
        }
        if !buffer.iter().all(u8::is_ascii_whitespace) {
            let item = serde_json::from_slice(&buffer).map_err(|_| unavailable())?;
            on_item(item)?;
        }
        Ok(())
    }
}

fn invalid_request() -> ApiError {
    ApiError {
        code: ErrorCode::InvalidRequest,
        params: BTreeMap::new(),
    }
}

fn unavailable() -> ApiError {
    ApiError {
        code: ErrorCode::ControllerUnavailable,
        params: BTreeMap::new(),
    }
}
