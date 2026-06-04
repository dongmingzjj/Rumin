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

        /// 保存结果到文件（覆盖已有文件）
        #[arg(short = 'o', long = "output")]
        output: Option<String>,

        /// 注入 Cookie 字符串，格式: "name1=val1; name2=val2"
        #[arg(long)]
        cookie: Option<String>,

        /// 从 Netscape cookie 文件加载 Cookie
        #[arg(long)]
        cookie_file: Option<String>,
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

        /// 保存结果到文件（覆盖已有文件）
        #[arg(short = 'o', long = "output")]
        output: Option<String>,

        /// 注入 Cookie 字符串，格式: "name1=val1; name2=val2"
        #[arg(long)]
        cookie: Option<String>,

        /// 从 Netscape cookie 文件加载 Cookie
        #[arg(long)]
        cookie_file: Option<String>,
    },

    /// 提取页面中的所有链接
    Links {
        /// 要导航的 URL
        url: String,

        /// 只保留同域链接
        #[arg(long)]
        same_domain: bool,

        /// 只保留匹配 pattern 的链接（子串匹配）
        #[arg(long)]
        filter: Option<String>,

        /// 输出 JSON 数组格式
        #[arg(long)]
        json: bool,

        /// 保存结果到文件
        #[arg(short = 'o', long = "output")]
        output: Option<String>,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Custom HTTP header (可多次使用，格式: "Name: Value")
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,

        /// 注入 Cookie 字符串
        #[arg(long)]
        cookie: Option<String>,

        /// 从 Netscape cookie 文件加载 Cookie
        #[arg(long)]
        cookie_file: Option<String>,
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

/// 解析 "name1=val1; name2=val2" 格式的 Cookie 字符串，注入到 CookieJar
fn inject_cookie_string(jar: &Arc<Mutex<mb_network::CookieJar>>, cookie_str: &str, domain: &str) {
    if let Ok(mut jar) = jar.lock() {
        for pair in cookie_str.split(';') {
            let pair = pair.trim();
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim();
                let value = value.trim();
                if !name.is_empty() {
                    jar.insert(mb_network::Cookie {
                        name: name.to_string(),
                        value: value.to_string(),
                        domain: Some(domain.to_string()),
                        path: "/".to_string(),
                        expires: None, // session cookie
                        secure: false,
                        http_only: false,
                    });
                }
            }
        }
    }
}

/// 从 Netscape cookie 文件加载 Cookie，注入到 CookieJar
/// 文件格式：每行以 tab 分隔，字段：domain, flag, path, secure, expires, name, value
fn inject_cookie_file(jar: &Arc<Mutex<mb_network::CookieJar>>, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("读取 cookie 文件失败: {}", e))?;

    if let Ok(mut jar) = jar.lock() {
        for line in content.lines() {
            let line = line.trim();
            // 跳过空行和注释行
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() >= 7 {
                let domain = fields[0].to_string();
                let path = fields[2].to_string();
                let secure = fields[3].eq_ignore_ascii_case("TRUE");
                let expires = fields[4].parse::<i64>().ok().filter(|&e| e > 0);
                let name = fields[5].to_string();
                let value = fields[6].to_string();

                // 去除域名前导点
                let domain = domain.trim_start_matches('.').to_string();

                jar.insert(mb_network::Cookie {
                    name,
                    value,
                    domain: Some(domain),
                    path,
                    expires,
                    secure,
                    http_only: false,
                });
            }
        }
    }
    Ok(())
}

/// 辅助函数：将输出写入文件或返回字符串
fn write_output(content: &str, output_path: &Option<String>) -> Result<()> {
    if let Some(path) = output_path {
        std::fs::write(path, content)?;
    } else {
        print!("{}", content);
    }
    Ok(())
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
            output,
            cookie,
            cookie_file,
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

            // 注入 Cookie（在导航之前）
            if let Some(ref cookie_str) = cookie {
                let domain = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                inject_cookie_string(browser.cookies(), cookie_str, &domain);
            }
            if let Some(ref cookie_path) = cookie_file {
                inject_cookie_file(browser.cookies(), cookie_path)?;
            }

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

                let output_data = NavigateJsonOutput {
                    url: page.url.clone(),
                    status: page_status,
                    title: page_title,
                    extracted: extracted_value,
                    body: if source { Some(page.source().to_string()) } else { None },
                };
                let content = format!("{}\n", serde_json::to_string_pretty(&output_data)?);
                write_output(&content, &output)?;
            } else {
                // 非 JSON 模式：原有行为 + 新功能
                let mut buf = String::new();

                if title || (!eval.is_some() && !selector.is_some() && !source && !dom) {
                    let t = page.dom_title();
                    if t.is_empty() {
                        buf.push_str("(no title)\n");
                    } else {
                        buf.push_str(&t);
                        buf.push('\n');
                    }
                }

                if let Some(ref sel) = selector {
                    if select_all {
                        let nodes = page.dom.query_selector_all(sel);
                        if nodes.is_empty() {
                            buf.push_str(&format!("(no match for '{}')\n", sel));
                        } else {
                            for nid in &nodes {
                                if let Some(ref attr_name) = attr {
                                    let val = mb_dom::element::get_attribute(&page.dom, *nid, attr_name)
                                        .unwrap_or_default();
                                    buf.push_str(&val);
                                    buf.push('\n');
                                } else {
                                    buf.push_str(&page.dom.text_content(*nid));
                                    buf.push('\n');
                                }
                            }
                        }
                    } else {
                        if let Some(ref attr_name) = attr {
                            match page.dom.query_selector(sel) {
                                Some(nid) => {
                                    let val = mb_dom::element::get_attribute(&page.dom, nid, attr_name)
                                        .unwrap_or_default();
                                    buf.push_str(&val);
                                    buf.push('\n');
                                }
                                None => buf.push_str(&format!("(no match for '{}')\n", sel)),
                            }
                        } else {
                            match page.query_text(sel) {
                                Some(text) => {
                                    buf.push_str(&text);
                                    buf.push('\n');
                                }
                                None => buf.push_str(&format!("(no match for '{}')\n", sel)),
                            }
                        }
                    }
                }

                if let Some(ref code) = eval {
                    match page.eval(code) {
                        Ok(result) => {
                            buf.push_str(&result);
                            buf.push('\n');
                        }
                        Err(e) => eprintln!("JS error: {}", e),
                    }
                }

                if source {
                    buf.push_str(page.source());
                    buf.push('\n');
                }

                if dom {
                    // DOM 树输出到 stdout（不适合写文件，保持原行为）
                    print_dom_tree(page.dom(), page.dom().document_node, 0);
                }

                // 将缓冲内容写入文件或 stdout
                if !dom {
                    write_output(&buf, &output)?;
                } else if output.is_some() {
                    // dom 模式下如果指定了 -o，警告不支持
                    eprintln!("警告: --dom 模式不支持 -o 输出到文件，已直接输出到终端");
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
            output,
            cookie,
            cookie_file,
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

            // 注入 Cookie（在导航之前）
            if let Some(ref cookie_str) = cookie {
                // 使用第一个 URL 的域名作为默认域名
                let default_domain = url::Url::parse(&all_urls[0])
                    .ok()
                    .and_then(|u| u.host_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                inject_cookie_string(browser.cookies(), cookie_str, &default_domain);
            }
            if let Some(ref cookie_path) = cookie_file {
                inject_cookie_file(browser.cookies(), cookie_path)?;
            }

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
                let content = format!("{}\n", serde_json::to_string_pretty(&results)?);
                write_output(&content, &output)?;
            } else {
                // 原有文本输出格式
                let mut buf = String::new();
                for r in &results {
                    if let Some(ref err) = r.error {
                        buf.push_str(&format!("{}\t\t0\tERROR: {}\n", r.url, err));
                    } else {
                        let extracted_col = match &r.extracted {
                            Some(val) => format!("\t{}", val.replace('\n', "\\n")),
                            None => String::new(),
                        };
                        buf.push_str(&format!("{}\\t{}{}{}\n", r.url, r.title, extracted_col, format!("\t{}", r.status)));
                    }
                }
                write_output(&buf, &output)?;
            }
        }

        Commands::Links {
            url,
            same_domain,
            filter,
            json: json_output,
            output,
            verbose,
            headers,
            cookie,
            cookie_file,
        } => {
            // Set up logging
            let level = if verbose { "debug" } else { "warn" };
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
                )
                .init();

            // 创建浏览器
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

            // 注入 Cookie
            if let Some(ref cookie_str) = cookie {
                let domain = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                inject_cookie_string(browser.cookies(), cookie_str, &domain);
            }
            if let Some(ref cookie_path) = cookie_file {
                inject_cookie_file(browser.cookies(), cookie_path)?;
            }

            // 导航到页面
            let page = if headers.is_empty() {
                browser.navigate(&url).await?
            } else {
                let mut p = browser.new_page();
                let req = build_request_with_headers(&url, &headers);
                p.navigate_with_request(req).await?;
                p
            };

            // 解析基准 URL（用于将相对链接转为绝对链接）
            let base_url = url::Url::parse(&page.url).ok();

            // 提取所有 <a href="..."> 链接
            let anchor_nodes = page.dom.query_selector_all("a");
            let mut links: Vec<String> = Vec::new();

            for nid in &anchor_nodes {
                if let Some(href) = mb_dom::element::get_attribute(&page.dom, *nid, "href") {
                    let href = href.trim().to_string();
                    // 跳过空 href、javascript:、锚点链接
                    if href.is_empty()
                        || href.starts_with("javascript:")
                        || href.starts_with("mailto:")
                        || href.starts_with("tel:")
                        || href.starts_with('#')
                    {
                        continue;
                    }
                    // 将相对链接转为绝对链接
                    let absolute = if let Some(ref base) = base_url {
                        match base.join(&href) {
                            Ok(resolved) => resolved.to_string(),
                            Err(_) => href,
                        }
                    } else {
                        href
                    };
                    links.push(absolute);
                }
            }

            // 去重
            links.sort();
            links.dedup();

            // --same-domain 过滤：只保留同域链接
            if same_domain {
                if let Some(ref base) = base_url {
                    let base_host = base.host_str().unwrap_or("");
                    links.retain(|link| {
                        if let Ok(parsed) = url::Url::parse(link) {
                            parsed.host_str().unwrap_or("") == base_host
                        } else {
                            false
                        }
                    });
                }
            }

            // --filter 过滤：子串匹配
            if let Some(ref pattern) = filter {
                links.retain(|link| link.contains(pattern.as_str()));
            }

            // 输出结果
            if json_output {
                let content = format!("{}\n", serde_json::to_string_pretty(&links)?);
                write_output(&content, &output)?;
            } else {
                let mut buf = String::new();
                for link in &links {
                    buf.push_str(link);
                    buf.push('\n');
                }
                write_output(&buf, &output)?;
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
