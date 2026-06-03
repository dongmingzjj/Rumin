//! Browser — orchestrates pages, shared state, and navigation

use anyhow::Result;
use std::sync::{Arc, Mutex};
use serde::Serialize;

use mb_network::client::{ClientConfig, HttpClient};
use mb_network::cookie::CookieJar;
use mb_network::interceptor::RequestLog;

use crate::page::Page;

/// Result of a single batch navigation
#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub url: String,
    pub status: u16,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The main browser instance
pub struct Browser {
    /// Shared HTTP client
    client: Arc<HttpClient>,
    /// Shared cookie jar
    cookies: Arc<Mutex<CookieJar>>,
}

impl Browser {
    /// Create a new browser with default configuration
    pub fn new() -> Result<Self> {
        let cookies = Arc::new(Mutex::new(CookieJar::new()));
        let client = HttpClient::new()?.with_cookies(Arc::clone(&cookies));

        Ok(Self {
            client: Arc::new(client),
            cookies,
        })
    }

    /// Create a browser with custom client config
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        let cookies = Arc::new(Mutex::new(CookieJar::new()));
        let client = HttpClient::with_config(config)?.with_cookies(Arc::clone(&cookies));

        Ok(Self {
            client: Arc::new(client),
            cookies,
        })
    }

    /// Create a browser with a shared request log for recording HTTP traffic
    pub fn with_request_log(log: Arc<Mutex<RequestLog>>) -> Result<Self> {
        let cookies = Arc::new(Mutex::new(CookieJar::new()));
        let client = HttpClient::new()?
            .with_cookies(Arc::clone(&cookies))
            .with_logging(log);

        Ok(Self {
            client: Arc::new(client),
            cookies,
        })
    }

    /// Create a browser with custom client config AND a shared request log
    pub fn with_config_and_request_log(
        config: ClientConfig,
        log: Arc<Mutex<RequestLog>>,
    ) -> Result<Self> {
        let cookies = Arc::new(Mutex::new(CookieJar::new()));
        let client = HttpClient::with_config(config)?
            .with_cookies(Arc::clone(&cookies))
            .with_logging(log);

        Ok(Self {
            client: Arc::new(client),
            cookies,
        })
    }

    /// Open a new page/tab
    pub fn new_page(&self) -> Page {
        Page::new(Arc::clone(&self.client), Arc::clone(&self.cookies))
    }

    /// Navigate a new page to a URL and return it
    pub async fn navigate(&self, url: &str) -> Result<Page> {
        let mut page = self.new_page();
        page.navigate(url).await?;
        Ok(page)
    }

    /// Navigate multiple URLs with a concurrency limit.
    /// Each URL gets its own Page but shares HttpClient + CookieJar.
    /// Uses std::thread::spawn + independent tokio runtimes for true concurrency,
    /// since Page (rquickjs) is !Send and cannot be used with tokio::spawn.
    pub async fn navigate_all(
        &self,
        urls: Vec<String>,
        concurrency: usize,
    ) -> Vec<BatchResult> {
        let client = Arc::clone(&self.client);
        let cookies = Arc::clone(&self.cookies);
        let concurrency = concurrency.max(1);
        let mut all_results = Vec::with_capacity(urls.len());

        for chunk in urls.chunks(concurrency) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|url| {
                    let client = Arc::clone(&client);
                    let cookies = Arc::clone(&cookies);
                    let url = url.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(1)
                            .enable_all()
                            .build()
                            .unwrap();
                        rt.block_on(async move {
                            let result = tokio::time::timeout(
                                std::time::Duration::from_secs(60),
                                async {
                                    let mut page = Page::new(client, cookies);
                                    page.navigate(&url).await?;
                                    Ok::<_, anyhow::Error>(page)
                                },
                            )
                            .await;

                            match result {
                                Ok(Ok(page)) => BatchResult {
                                    url,
                                    status: page.status,
                                    title: page.dom_title(),
                                    error: None,
                                },
                                Ok(Err(e)) => BatchResult {
                                    url,
                                    status: 0,
                                    title: String::new(),
                                    error: Some(format!("{}", e)),
                                },
                                Err(_) => BatchResult {
                                    url,
                                    status: 0,
                                    title: String::new(),
                                    error: Some("timeout after 60s".to_string()),
                                },
                            }
                        })
                    })
                })
                .collect();

            for h in handles {
                if let Ok(r) = h.join() {
                    all_results.push(r);
                }
            }
        }

        all_results
    }

    /// Get the cookie jar
    pub fn cookies(&self) -> &Arc<Mutex<CookieJar>> {
        &self.cookies
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new().expect("Failed to create default browser")
    }
}
