use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
pub async fn wait_for_callback(listener: TcpListener, tx: tokio::sync::oneshot::Sender<String>) {
    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);

        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1));

        if let Some(path) = path
            && let Ok(url) = url::Url::parse(&format!("http://localhost{path}"))
            && url.path() == "/callback"
            && let Some(code) = url
                .query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.to_string())
        {
            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h3>Authenticated. You can close this tab.</h3></body></html>";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = tx.send(code);
            return;
        }

        let response = "HTTP/1.1 404 Not Found\r\n\r\nNot found";
        let _ = stream.write_all(response.as_bytes()).await;
    }
}
