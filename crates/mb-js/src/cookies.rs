//! Cookie support for the JS engine.

use super::*;

impl JsEngine {
    /// Set the CookieJar for document.cookie support (shared reference)
    pub fn set_cookie_jar(&mut self, jar: Arc<Mutex<CookieJar>>, url: &str) -> Result<()> {
        // Extract hostname from URL (without port)
        let hostname = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default();
        // Register native cookie functions using the shared Arc
        self.context.with(|ctx| -> rquickjs::Result<()> {
            use rquickjs::Function;
            use rquickjs::function::Rest;

            // _get_cookies() -> cookie string for current domain
            let hostname_get = hostname.clone();
            let jar_get = Arc::clone(&jar);
            let get_fn = Function::new(ctx.clone(), move |_args: Rest<rquickjs::Value>| -> rquickjs::Result<String> {
                let result = jar_get.lock()
                    .ok()
                    .map(|j| {
                        j.get_matching(&hostname_get, "/").iter()
                            .filter(|c| !c.http_only)
                            .map(|c| format!("{}={}", c.name, c.value))
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default();
                Ok(result)
            })?;
            ctx.globals().set("_get_cookies", get_fn)?;

            // _set_cookie(cookie_str) -> parse and store a cookie
            let hostname_set = hostname.clone();
            let jar_set = Arc::clone(&jar);
            let set_fn = Function::new(ctx.clone(), move |args: Rest<rquickjs::Value>| -> rquickjs::Result<()> {
                let cookie_str = args.get(0)
                    .and_then(|v| v.as_string())
                    .and_then(|s| s.to_string().ok())
                    .unwrap_or_default();
                if !cookie_str.is_empty() {
                    if let Ok(mut j) = jar_set.lock() {
                        j.parse_set_cookie(&cookie_str, &hostname_set);
                    }
                }
                Ok(())
            })?;
            ctx.globals().set("_set_cookie", set_fn)?;

            Ok(())
        }).map_err(|e| anyhow!("Failed to setup cookies: {:?}", e))?;
        Ok(())
    }
}
