use async_trait::async_trait;
use http_client::{HttpClient, http_types};

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

        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| http_types::Error::new(http_types::StatusCode::InternalServerError, e))?;
        response.set_body(body_bytes.as_ref());

        Ok(response)
    }
}
