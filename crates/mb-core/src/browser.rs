//! Browser — orchestrates pages, shared state, and navigation

use anyhow::Result;
use std::sync::{Arc, Mutex};
use serde::Serialize;

use mb_network::client::{ClientConfig, HttpClient};
use mb_network::cookie::CookieJar;
use mb_network::interceptor::RequestLog;

use crate::page::Page;

/// 提取配置 — 告诉 navigate_all 在每个页面上执行什么提取操作
#[derive(Debug, Clone, Default)]
pub struct ExtractionConfig {
    /// CSS 选择器
    pub selector: Option<String>,
    /// 是否选择所有匹配元素（而非仅第一个）
    pub select_all: bool,
    /// JavaScript 表达式
    pub eval: Option<String>,
    /// 提取指定属性值（配合 selector 使用）
    pub attr: Option<String>,
}

/// Result of a single batch navigation
#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub url: String,
    pub status: u16,
    pub title: String,
    /// 提取结果（selector/eval 的输出）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<String>,
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
        extraction: Option<ExtractionConfig>,
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
                    let ext = extraction.clone();
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
                                Ok(Ok(mut page)) => {
                                    let status = page.status;
                                    let title = page.dom_title();
                                    let extracted = ext.and_then(|ext| {
                                        Self::extract_from_page_static(&mut page, &ext)
                                    });
                                    BatchResult {
                                        url,
                                        status,
                                        title,
                                        extracted,
                                        error: None,
                                    }
                                },
                                Ok(Err(e)) => BatchResult {
                                    url,
                                    status: 0,
                                    title: String::new(),
                                    extracted: None,
                                    error: Some(format!("{}", e)),
                                },
                                Err(_) => BatchResult {
                                    url,
                                    status: 0,
                                    title: String::new(),
                                    extracted: None,
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

    /// 从页面提取内容（静态方法，在线程内调用）
    fn extract_from_page_static(page: &mut Page, config: &ExtractionConfig) -> Option<String> {
        // 优先执行 eval
        if let Some(ref code) = config.eval {
            match page.eval(code) {
                Ok(result) => return Some(result),
                Err(_) => return None,
            }
        }
        // 其次执行 selector
        if let Some(ref selector) = config.selector {
            if config.select_all {
                // --select-all: 返回所有匹配元素
                let nodes = page.dom.query_selector_all(selector);
                if nodes.is_empty() {
                    return None;
                }
                let values: Vec<String> = nodes.iter().map(|&nid| {
                    if let Some(ref attr_name) = config.attr {
                        // 提取属性值
                        mb_dom::element::get_attribute(&page.dom, nid, attr_name)
                            .unwrap_or_default()
                    } else {
                        // 提取文本内容
                        page.dom.text_content(nid)
                    }
                }).collect();
                return Some(values.join("\n"));
            } else {
                // 单元素模式
                if let Some(node_id) = page.dom.query_selector(selector) {
                    if let Some(ref attr_name) = config.attr {
                        return mb_dom::element::get_attribute(&page.dom, node_id, attr_name);
                    } else {
                        return Some(page.dom.text_content(node_id));
                    }
                }
                return None;
            }
        }
        None
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new().expect("Failed to create default browser")
    }
}
