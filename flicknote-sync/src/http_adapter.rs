use async_trait::async_trait;
use futures_lite::StreamExt;
use http_client::{HttpClient, http_types};
use tokio_util::compat::TokioAsyncReadCompatExt;

#[derive(Debug)]
pub(crate) struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(
        &self,
        mut req: http_types::Request,
    ) -> Result<http_types::Response, http_types::Error> {
        let method = reqwest::Method::from_bytes(req.method().to_string().as_bytes())
            .map_err(|e| http_types::Error::new(http_types::StatusCode::InternalServerError, e))?;
        let url = req.url().as_str();

        let mut builder = self.client.request(method, url);

        for (name, values) in req.iter() {
            for value in values {
                builder = builder.header(name.as_str(), value.as_str());
            }
        }

        let bytes = req.take_body().into_bytes().await?;
        if !bytes.is_empty() {
            builder = builder.body(bytes);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| http_types::Error::new(http_types::StatusCode::InternalServerError, e))?;

        let status = http_types::StatusCode::try_from(resp.status().as_u16()).map_err(|e| {
            http_types::Error::from_str(http_types::StatusCode::InternalServerError, e.to_string())
        })?;
        let content_length = resp.content_length().map(|l| l as usize);

        let mut response = http_types::Response::new(status);

        for (name, value) in resp.headers() {
            match value.to_str() {
                Ok(v) => {
                    response.append_header(name.as_str(), v);
                }
                Err(_) => {
                    log::warn!(
                        "http_adapter: dropping non-UTF-8 response header '{}'",
                        name
                    );
                }
            }
        }

        // Stream the response body instead of eagerly buffering with bytes().await.
        // PowerSync's sync/stream endpoint is a long-lived streaming connection —
        // the old code blocked here until the server closed the connection.
        let byte_stream = resp
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));
        let stream_reader = tokio_util::io::StreamReader::new(byte_stream);
        let compat_reader = stream_reader.compat();
        let buf_reader = futures_lite::io::BufReader::new(compat_reader);
        response.set_body(http_types::Body::from_reader(buf_reader, content_length));

        Ok(response)
    }
}
