//! Browser — orchestrates pages, shared state, and navigation

use anyhow::Result;
use std::sync::{Arc, Mutex};

use mb_network::client::{ClientConfig, HttpClient};
use mb_network::cookie::CookieJar;

use crate::page::Page;

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
        let client = HttpClient::new()?;
        let cookies = CookieJar::new();

        Ok(Self {
            client: Arc::new(client),
            cookies: Arc::new(Mutex::new(cookies)),
        })
    }

    /// Create a browser with custom client config
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        let client = HttpClient::with_config(config)?;
        let cookies = CookieJar::new();

        Ok(Self {
            client: Arc::new(client),
            cookies: Arc::new(Mutex::new(cookies)),
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
