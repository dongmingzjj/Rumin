use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;

use mb_network::interceptor::RequestLog;

/// minibrowser — a minimal browser engine for web scraping
#[derive(Parser)]
#[command(name = "minibrowser", version = "0.1.0")]
#[command(about = "Minimal browser engine for automated web scraping")]
struct Cli {
    /// Proxy URL (e.g. http://127.0.0.1:7890 or socks5://127.0.0.1:1080)
    #[arg(long)]
    proxy: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Navigate to a URL and optionally evaluate JavaScript
    Navigate {
        /// The URL to navigate to
        url: String,

        /// JavaScript expression to evaluate after page loads
        #[arg(short, long)]
        eval: Option<String>,

        /// CSS selector to extract text from
        #[arg(short, long)]
        selector: Option<String>,

        /// Print the full page title
        #[arg(short, long)]
        title: bool,

        /// Print the raw HTML source
        #[arg(long)]
        source: bool,

        /// Dump the DOM tree structure
        #[arg(long)]
        dom: bool,

        /// Record all HTTP requests to a JSON file
        #[arg(long)]
        record: Option<String>,

        /// Record full response bodies (makes the recording file much larger)
        #[arg(long)]
        record_full: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Custom HTTP header (可多次使用，格式: "Name: Value")
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,

        /// 选择所有匹配元素（配合 -s 使用，默认仅返回第一个）
        #[arg(long)]
        select_all: bool,

        /// 提取匹配元素的指定属性值（配合 -s 使用）
        #[arg(long)]
        attr: Option<String>,

        /// 输出 JSON 格式（便于 jq/管道处理）
        #[arg(long)]
        json: bool,
    },

    /// Replay previously recorded HTTP requests
    Replay {
        /// Path to the recorded JSON file
        file: String,

        /// Export requests as curl commands instead of replaying
        #[arg(long)]
        curl: bool,
    },

    /// Batch navigate multiple URLs concurrently
    Batch {
        /// URLs to navigate to
        urls: Vec<String>,

        /// Read URLs from a file (one per line)
        #[arg(short, long)]
        file: Option<String>,

        /// Maximum number of concurrent navigations
        #[arg(short, long, default_value = "4")]
        concurrency: usize,

        /// CSS selector to extract text from
        #[arg(short, long)]
        selector: Option<String>,

        /// JavaScript expression to evaluate
        #[arg(short, long)]
        eval: Option<String>,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Custom HTTP header (可多次使用，格式: "Name: Value")
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,

        /// 选择所有匹配元素（配合 -s 使用，默认仅返回第一个）
        #[arg(long)]
        select_all: bool,

        /// 提取匹配元素的指定属性值（配合 -s 使用）
        #[arg(long)]
        attr: Option<String>,

        /// 输出 JSON 格式（便于 jq/管道处理）
        #[arg(long)]
        json: bool,
    },
}

/// navigate --json 的输出结构
#[derive(Serialize)]
struct NavigateJsonOutput {
    url: String,
    status: u16,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extracted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

/// 解析 "Name: Value" 格式的自定义 header
fn parse_header(h: &str) -> Option<(&str, &str)> {
    h.split_once(':').map(|(k, v)| (k.trim(), v.trim()))
}

/// 构建带自定义 headers 的 HttpRequest
fn build_request_with_headers(
    url: &str,
    headers: &[String],
) -> mb_network::HttpRequest {
    let mut req = mb_network::HttpRequest::get(url)
        .header("sec-fetch-user", "?1")
        .header("upgrade-insecure-requests", "1");
    for h in headers {
        if let Some((k, v)) = parse_header(h) {
            req = req.header(k, v);
        }
    }
    req
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve proxy URL from CLI flag or environment variables
    let proxy_url = cli.proxy.or_else(|| {
        std::env::var("MB_PROXY")
            .or_else(|_| std::env::var("http_proxy"))
            .or_else(|_| std::env::var("https_proxy"))
            .ok()
            .filter(|s| !s.is_empty())
    });

    match cli.command {
        Commands::Navigate {
            url,
            eval,
            selector,
            title,
            source,
            dom,
            record,
            record_full,
            verbose,
            headers,
            select_all,
            attr,
            json,
        } => {
            // Set up logging
            let level = if verbose { "debug" } else { "warn" };
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
                )
                .init();

            // Create browser (with optional request logging)
            let log: Option<Arc<Mutex<RequestLog>>> = if record.is_some() {
                Some(Arc::new(Mutex::new(
                    RequestLog::new().with_full_body(record_full),
                )))
            } else {
                None
            };

            // Build client config with optional proxy
            let client_config = if let Some(ref proxy) = proxy_url {
                let pc = if proxy.starts_with("socks5://") || proxy.starts_with("socks5h://") {
                    mb_network::client::ProxyConfig::Socks5(proxy.clone())
                } else {
                    mb_network::client::ProxyConfig::Http(proxy.clone())
                };
                mb_network::client::ClientConfigBuilder::new().proxy(pc).build()
            } else {
                mb_network::client::ClientConfigBuilder::new().build()
            };

            let browser = if let Some(ref l) = log {
                mb_core::Browser::with_config_and_request_log(client_config, Arc::clone(l))?
            } else {
                mb_core::Browser::with_config(client_config)?
            };

            // 使用自定义 headers 导航
            let mut page = if headers.is_empty() {
                browser.navigate(&url).await?
            } else {
                // 通过手动构建 request + page 来支持自定义 headers
                let mut p = browser.new_page();
                let req = build_request_with_headers(&url, &headers);
                p.navigate_with_request(req).await?;
                p
            };

            // 收集提取结果
            let mut extracted_value: Option<String> = None;

            // --json 模式：收集所有信息后统一输出
            if json {
                let page_title = page.dom_title();
                let page_status = page.status;

                // 执行提取操作
                if let Some(ref code) = eval {
                    match page.eval(code) {
                        Ok(result) => extracted_value = Some(result),
                        Err(e) => eprintln!("JS error: {}", e),
                    }
                } else if let Some(ref sel) = selector {
                    extracted_value = extract_from_dom(&page, sel, select_all, attr.as_deref());
                }

                let output = NavigateJsonOutput {
                    url: page.url.clone(),
                    status: page_status,
                    title: page_title,
                    extracted: extracted_value,
                    body: if source { Some(page.source().to_string()) } else { None },
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                // 非 JSON 模式：原有行为 + 新功能
                if title || (!eval.is_some() && !selector.is_some() && !source && !dom) {
                    let t = page.dom_title();
                    if t.is_empty() {
                        println!("(no title)");
                    } else {
                        println!("{}", t);
                    }
                }

                if let Some(ref sel) = selector {
                    if select_all {
                        // --select-all: 输出所有匹配元素，每行一个
                        let nodes = page.dom.query_selector_all(sel);
                        if nodes.is_empty() {
                            println!("(no match for '{}')", sel);
                        } else {
                            for nid in &nodes {
                                if let Some(ref attr_name) = attr {
                                    let val = mb_dom::element::get_attribute(&page.dom, *nid, attr_name)
                                        .unwrap_or_default();
                                    println!("{}", val);
                                } else {
                                    println!("{}", page.dom.text_content(*nid));
                                }
                            }
                        }
                    } else {
                        if let Some(ref attr_name) = attr {
                            // 提取单个元素的属性值
                            match page.dom.query_selector(sel) {
                                Some(nid) => {
                                    let val = mb_dom::element::get_attribute(&page.dom, nid, attr_name)
                                        .unwrap_or_default();
                                    println!("{}", val);
                                }
                                None => println!("(no match for '{}')", sel),
                            }
                        } else {
                            match page.query_text(sel) {
                                Some(text) => println!("{}", text),
                                None => println!("(no match for '{}')", sel),
                            }
                        }
                    }
                }

                if let Some(ref code) = eval {
                    match page.eval(code) {
                        Ok(result) => println!("{}", result),
                        Err(e) => eprintln!("JS error: {}", e),
                    }
                }

                if source {
                    println!("{}", page.source());
                }

                if dom {
                    print_dom_tree(page.dom(), page.dom().document_node, 0);
                }
            }

            // Save recorded requests if --record was specified
            if let (Some(path), Some(ref log)) = (&record, &log) {
                let log = log.lock().unwrap_or_else(|e| e.into_inner());
                log.save(path)?;
                eprintln!("Recorded {} request(s) to {}", log.entries().len(), path);
            }
        }

        Commands::Replay { file, curl } => {
            let log = RequestLog::load(&file)?;
            let entries = log.entries();

            if entries.is_empty() {
                println!("No recorded requests in {}", file);
                return Ok(());
            }

            if curl {
                // Export as curl commands
                for cmd in log.to_curl_commands() {
                    println!("{}", cmd);
                    println!();
                }
            } else {
                // Replay requests via HTTP client
                let client = mb_network::HttpClient::new()?;
                for (i, entry) in entries.iter().enumerate() {
                    let req = entry.to_http_request()?;
                    println!(
                        "[{}/{}] {} {}",
                        i + 1,
                        entries.len(),
                        entry.method,
                        entry.url
                    );
                    match client.execute(req).await {
                        Ok(resp) => {
                            println!("  -> {} ({} bytes)", resp.status, resp.body.len());
                        }
                        Err(e) => {
                            eprintln!("  -> ERROR: {}", e);
                        }
                    }
                }
            }
        }

        Commands::Batch {
            urls,
            file,
            concurrency,
            selector,
            eval,
            verbose,
            headers: _headers,
            select_all,
            attr,
            json,
        } => {
            // Set up logging
            let level = if verbose { "debug" } else { "warn" };
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
                )
                .init();

            // Collect URLs from file if specified
            let mut all_urls = urls;
            if let Some(ref path) = file {
                let content = std::fs::read_to_string(path)?;
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        all_urls.push(trimmed.to_string());
                    }
                }
            }

            if all_urls.is_empty() {
                eprintln!("No URLs specified. Provide URLs as arguments or via --file.");
                std::process::exit(1);
            }

            // Build client config with optional proxy
            let client_config = if let Some(ref proxy) = proxy_url {
                let pc = if proxy.starts_with("socks5://") || proxy.starts_with("socks5h://") {
                    mb_network::client::ProxyConfig::Socks5(proxy.clone())
                } else {
                    mb_network::client::ProxyConfig::Http(proxy.clone())
                };
                mb_network::client::ClientConfigBuilder::new().proxy(pc).build()
            } else {
                mb_network::client::ClientConfigBuilder::new().build()
            };

            let browser = mb_core::Browser::with_config(client_config)?;

            // 构建提取配置
            let extraction_config = if selector.is_some() || eval.is_some() {
                Some(mb_core::ExtractionConfig {
                    selector: selector.clone(),
                    select_all,
                    eval: eval.clone(),
                    attr: attr.clone(),
                })
            } else {
                None
            };

            let results = browser.navigate_all(all_urls, concurrency, extraction_config).await;

            if json {
                // --json: 输出 JSON 数组
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                // 原有文本输出格式
                for r in &results {
                    if let Some(ref err) = r.error {
                        println!("{}\t\t0\tERROR: {}", r.url, err);
                    } else {
                        let extracted_col = match &r.extracted {
                            Some(val) => format!("\t{}", val.replace('\n', "\\n")),
                            None => String::new(),
                        };
                        println!("{}\t{}{}{}", r.url, r.title, extracted_col, format!("\t{}", r.status));
                    }
                }
            }
        }
    }

    Ok(())
}

/// 从页面 DOM 提取内容（供 navigate 命令使用）
fn extract_from_dom(page: &mb_core::Page, selector: &str, select_all: bool, attr: Option<&str>) -> Option<String> {
    if select_all {
        let nodes = page.dom.query_selector_all(selector);
        if nodes.is_empty() {
            return None;
        }
        let values: Vec<String> = nodes.iter().map(|&nid| {
            if let Some(attr_name) = attr {
                mb_dom::element::get_attribute(&page.dom, nid, attr_name)
                    .unwrap_or_default()
            } else {
                page.dom.text_content(nid)
            }
        }).collect();
        Some(values.join("\n"))
    } else {
        if let Some(nid) = page.dom.query_selector(selector) {
            if let Some(attr_name) = attr {
                mb_dom::element::get_attribute(&page.dom, nid, attr_name)
            } else {
                Some(page.dom.text_content(nid))
            }
        } else {
            None
        }
    }
}

/// Recursively print the DOM tree
fn print_dom_tree(dom: &mb_dom::tree::DomTree, node: mb_dom::node::NodeId, depth: usize) {
    use mb_dom::node::NodeKind;

    let indent = "  ".repeat(depth);
    let dom_node = dom.get_node(node);

    match &dom_node.kind {
        NodeKind::Document(_) => {
            println!("{}[Document]", indent);
        }
        NodeKind::Element(data) => {
            let attrs: Vec<String> = data.attributes.iter()
                .map(|attr| {
                    format!("{}=\"{}\"", attr.name, attr.value)
                })
                .collect();
            if attrs.is_empty() {
                println!("{}<{}>", indent, data.tag_name.to_lowercase());
            } else {
                println!("{}<{} {}>", indent, data.tag_name.to_lowercase(), attrs.join(" "));
            }
        }
        NodeKind::Text(data) => {
            let text = data.data.trim();
            if !text.is_empty() {
                let preview = if text.len() > 80 {
                    let end = text.char_indices()
                        .map(|(i, _)| i)
                        .filter(|&i| i <= 80)
                        .last()
                        .unwrap_or(0);
                    if end == 0 {
                        let s: String = text.chars().take(20).collect();
                        format!("{}...", s)
                    } else {
                        format!("{}...", &text[..end])
                    }
                } else {
                    text.to_string()
                };
                println!("{}\"{}\"", indent, preview);
            }
        }
        NodeKind::Comment(data) => {
            println!("{}<!-- {} -->", indent, data.data);
        }
    }

    // Recurse into children
    for child in dom.children(node) {
        print_dom_tree(dom, child, depth + 1);
    }
}
