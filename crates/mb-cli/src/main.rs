use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::{Parser, Subcommand};

use mb_network::interceptor::RequestLog;

/// minibrowser — a minimal browser engine for web scraping
#[derive(Parser)]
#[command(name = "minibrowser", version = "0.1.0")]
#[command(about = "Minimal browser engine for automated web scraping")]
struct Cli {
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
    },

    /// Replay previously recorded HTTP requests
    Replay {
        /// Path to the recorded JSON file
        file: String,

        /// Export requests as curl commands instead of replaying
        #[arg(long)]
        curl: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

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

            let browser = if let Some(ref l) = log {
                mb_core::Browser::with_request_log(Arc::clone(l))?
            } else {
                mb_core::Browser::new()?
            };

            let mut page = browser.navigate(&url).await?;

            // Output based on flags
            if title || (!eval.is_some() && !selector.is_some() && !source && !dom) {
                let t = page.dom_title();
                if t.is_empty() {
                    println!("(no title)");
                } else {
                    println!("{}", t);
                }
            }

            if let Some(selector) = &selector {
                match page.query_text(selector) {
                    Some(text) => println!("{}", text),
                    None => println!("(no match for '{}')", selector),
                }
            }

            if let Some(code) = &eval {
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

            // Save recorded requests if --record was specified
            if let (Some(path), Some(ref log)) = (&record, &log) {
                let log = log.lock().unwrap();
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
    }

    Ok(())
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
                    format!("{}...", &text[..80])
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
