//! Browser Web API registrations.
//!
//! Sets up console, navigator, screen, chrome, performance, location,
//! and miscellaneous Web APIs (atob, btoa, matchMedia, localStorage, sessionStorage).

use super::*;

impl JsEngine {
    /// Setup console object with log method
    pub fn setup_console(&mut self) -> Result<()> {
        let code = r#"
        var console = {
            _output: [],
            log: function(...args) {
                var msg = args.map(function(a) {
                    if (a === null) return 'null';
                    if (a === undefined) return 'undefined';
                    if (typeof a === 'object') {
                        try { return JSON.stringify(a); } catch(e) { return String(a); }
                    }
                    return String(a);
                }).join(' ');
                this._output.push(msg);
            },
            error: function(...args) {
                var msg = args.map(function(a) { return String(a); }).join(' ');
                this._output.push('[ERROR] ' + msg);
            },
            warn: function(...args) {
                var msg = args.map(function(a) { return String(a); }).join(' ');
                this._output.push('[WARN] ' + msg);
            }
        };
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup console: {:?}", e))?;
        Ok(())
    }

    /// Get console output
    pub fn get_console_output(&mut self) -> Result<Vec<String>> {
        let code = r#"
        (function() {
            if (typeof console !== 'undefined' && console._output) {
                return console._output;
            }
            return [];
        })()
        "#;
        let output = self.context.with(|ctx| -> rquickjs::Result<Vec<String>> {
            let result: Value = ctx.eval(code)?;
            let mut output = Vec::new();
            if let Some(arr) = result.as_object() {
                let length: usize = arr.get::<_, Value>("length")
                    .ok()
                    .and_then(|v| value_to_f64(&v))
                    .unwrap_or(0.0) as usize;
                for i in 0..length {
                    if let Ok(val) = arr.get::<_, Value>(i as u32) {
                        output.push(js_value_to_string(&val));
                    }
                }
            }
            Ok(output)
        }).map_err(|e| anyhow!("Failed to get console output: {:?}", e))?;
        Ok(output)
    }

    /// Setup navigator object — set as a true global property.
    pub fn setup_navigator(&mut self) -> Result<()> {
        self.setup_navigator_with_overrides(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            "MacIntel",
        )
    }

    /// Setup navigator object with custom user-agent and platform.
    pub fn setup_navigator_with_overrides(&mut self, user_agent: &str, platform: &str) -> Result<()> {
        // Build the JS code with properly escaped user-agent and platform strings
        let ua_json = serde_json::to_string(user_agent).unwrap_or_else(|_| "\"\"".to_string());
        let plat_json = serde_json::to_string(platform).unwrap_or_else(|_| "\"\"".to_string());
        let code = format!(r#"
        globalThis.navigator = {{
            userAgent: {ua_json},
            platform: {plat_json},
            language: 'en-US',
            languages: ['en-US', 'en'],
            cookieEnabled: true,
            onLine: true,
            vendor: 'Google Inc.',
            maxTouchPoints: 0,
            deviceMemory: 8,
            hardwareConcurrency: 8,
            plugins: [
                {{ name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' }},
                {{ name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' }},
                {{ name: 'Native Client', filename: 'internal-nacl-plugin', description: '' }}
            ],
            mimeTypes: []
        }};
        globalThis.navigator.plugins.length = 3;
        globalThis.window = globalThis;
        "#);

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup navigator: {:?}", e))?;
        Ok(())
    }

    /// Setup screen object with typical desktop resolution
    pub fn setup_screen(&mut self) -> Result<()> {
        let code = r#"
        globalThis.screen = {
            width: 1920,
            height: 1080,
            availWidth: 1920,
            availHeight: 1040,
            colorDepth: 24,
            pixelDepth: 24,
            orientation: { angle: 0, type: 'landscape-primary' }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup screen: {:?}", e))?;
        Ok(())
    }

    /// Setup window.chrome object for anti-detection
    pub fn setup_chrome(&mut self) -> Result<()> {
        let code = r#"
        globalThis.chrome = {
            runtime: {
                OnInstalledReason: {},
                OnRestartRequiredReason: {},
                PlatformArch: {},
                PlatformNaclArch: {},
                PlatformOs: {},
                RequestUpdateCheckStatus: {}
            },
            loadTimes: function() {
                return {
                    commitLoadTime: Date.now() / 1000,
                    connectionInfo: 'h2',
                    finishDocumentLoadTime: Date.now() / 1000,
                    finishLoadTime: Date.now() / 1000,
                    firstPaintAfterLoadTime: 0,
                    firstPaintTime: Date.now() / 1000,
                    navigationType: 'Other',
                    npnNegotiatedProtocol: 'h2',
                    requestTime: Date.now() / 1000,
                    startLoadTime: Date.now() / 1000,
                    wasAlternateProtocolAvailable: false,
                    wasFetchedViaSpdy: true,
                    wasNpnNegotiated: true
                };
            },
            csi: function() {
                var _ps = globalThis.__perf_start__ || Date.now();
                return {
                    onloadT: Date.now(),
                    pageT: Date.now() - _ps,
                    startE: Date.now(),
                    tran: 15
                };
            }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup chrome: {:?}", e))?;
        Ok(())
    }

    /// Setup performance.now() and timeOrigin for anti-detection
    pub fn setup_performance(&mut self) -> Result<()> {
        let code = r#"
        globalThis.__perf_start__ = Date.now();
        globalThis.performance = {
            timeOrigin: globalThis.__perf_start__,
            now: function() {
                return Date.now() - globalThis.__perf_start__;
            },
            timing: {
                navigationStart: globalThis.__perf_start__,
                unloadEventStart: 0,
                unloadEventEnd: 0,
                redirectStart: 0,
                redirectEnd: 0,
                fetchStart: globalThis.__perf_start__,
                domainLookupStart: globalThis.__perf_start__,
                domainLookupEnd: globalThis.__perf_start__,
                connectStart: globalThis.__perf_start__,
                connectEnd: globalThis.__perf_start__,
                secureConnectionStart: globalThis.__perf_start__,
                requestStart: globalThis.__perf_start__,
                responseStart: globalThis.__perf_start__,
                responseEnd: globalThis.__perf_start__,
                domLoading: globalThis.__perf_start__,
                domInteractive: globalThis.__perf_start__,
                domContentLoadedEventStart: globalThis.__perf_start__,
                domContentLoadedEventEnd: globalThis.__perf_start__,
                domComplete: globalThis.__perf_start__,
                loadEventStart: globalThis.__perf_start__,
                loadEventEnd: globalThis.__perf_start__
            }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup performance: {:?}", e))?;
        Ok(())
    }

    /// Setup misc Web APIs: atob, btoa, matchMedia, localStorage, sessionStorage
    pub fn setup_misc(&mut self) -> Result<()> {
        let code = r#"
        // Base64 encode/decode (atob/btoa)
        globalThis.atob = function(s) {
            var chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
            var result = '';
            for (var i = 0; i < s.length; i += 4) {
                var c1 = s.charAt(i), c2 = s.charAt(i+1), c3 = s.charAt(i+2), c4 = s.charAt(i+3);
                var a = chars.indexOf(c1);
                var b = c2 === '=' ? 0 : chars.indexOf(c2);
                var c = c3 === '=' ? 0 : chars.indexOf(c3);
                var d = c4 === '=' ? 0 : chars.indexOf(c4);
                result += String.fromCharCode((a << 2) | (b >> 4));
                if (c3 !== '=') result += String.fromCharCode(((b & 15) << 4) | (c >> 2));
                if (c4 !== '=') result += String.fromCharCode(((c & 3) << 6) | d);
            }
            return result;
        };
        globalThis.btoa = function(s) {
            var chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
            var result = '';
            for (var i = 0; i < s.length; i += 3) {
                var a = s.charCodeAt(i);
                var b = s.charCodeAt(i + 1) || 0;
                var c = s.charCodeAt(i + 2) || 0;
                result += chars.charAt(a >> 2);
                result += chars.charAt(((a & 3) << 4) | (b >> 4));
                result += i + 1 < s.length ? chars.charAt(((b & 15) << 2) | (c >> 6)) : '=';
                result += i + 2 < s.length ? chars.charAt(c & 63) : '=';
            }
            return result;
        };
        globalThis.matchMedia = function(query) {
            return {
                matches: false,
                media: query,
                onchange: null,
                addListener: function() {},
                removeListener: function() {},
                addEventListener: function() {},
                removeEventListener: function() {},
                dispatchEvent: function() { return true; }
            };
        };

        // localStorage — in-memory key-value store (persists within session)
        (function() {
            var _store = {};
            globalThis.localStorage = {
                getItem: function(key) { return _store.hasOwnProperty(key) ? _store[key] : null; },
                setItem: function(key, value) { _store[key] = String(value); },
                removeItem: function(key) { delete _store[key]; },
                clear: function() { _store = {}; },
                get length() { return Object.keys(_store).length; },
                key: function(index) { return Object.keys(_store)[index] || null; }
            };
        })();

        // sessionStorage — in-memory key-value store (per-tab, cleared on close)
        (function() {
            var _store = {};
            globalThis.sessionStorage = {
                getItem: function(key) { return _store.hasOwnProperty(key) ? _store[key] : null; },
                setItem: function(key, value) { _store[key] = String(value); },
                removeItem: function(key) { delete _store[key]; },
                clear: function() { _store = {}; },
                get length() { return Object.keys(_store).length; },
                key: function(index) { return Object.keys(_store)[index] || null; }
            };
        })();

        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup misc: {:?}", e))?;
        Ok(())
    }

    /// Setup location object — set as a true global property.
    pub fn setup_location(&mut self, url: &str) -> Result<()> {
        let (protocol, host, pathname, search, hash) = Self::parse_url_components(url);
        let origin = if host.is_empty() { String::new() } else { format!("{}{}", protocol, host) };
        // Derive port from protocol
        let port = if protocol == "https://" {
            "443".to_string()
        } else if protocol == "http://" {
            "80".to_string()
        } else {
            String::new()
        };

        let code = format!(r#"
        globalThis.location = {{
            href: {},
            protocol: {},
            host: {},
            hostname: {},
            port: {},
            pathname: {},
            search: {},
            hash: {},
            origin: {}
        }};
        globalThis.window = globalThis;
        "#, format_args!("{:?}", url),
            format_args!("{:?}", protocol),
            format_args!("{:?}", host),
            format_args!("{:?}", host),
            format_args!("{:?}", port),
            format_args!("{:?}", pathname),
            format_args!("{:?}", search),
            format_args!("{:?}", hash),
            format_args!("{:?}", origin));

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code.as_str())?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup location: {:?}", e))?;
        Ok(())
    }

    /// Parse URL into components (simple implementation)
    pub(crate) fn parse_url_components(url: &str) -> (String, String, String, String, String) {
        let mut protocol = String::from("about:");
        let mut host = String::new();
        let mut pathname = String::from("/");
        let mut search = String::new();
        let mut hash = String::new();

        if let Some(proto_end) = url.find("://") {
            protocol = url[..proto_end + 3].to_string();
            let rest = &url[proto_end + 3..];

            if let Some(path_start) = rest.find('/') {
                host = rest[..path_start].to_string();
                let path_and_query = &rest[path_start..];

                if let Some(hash_pos) = path_and_query.find('#') {
                    hash = path_and_query[hash_pos..].to_string();
                    let path_and_query = &path_and_query[..hash_pos];
                    if let Some(query_pos) = path_and_query.find('?') {
                        search = path_and_query[query_pos..].to_string();
                        pathname = path_and_query[..query_pos].to_string();
                    } else {
                        pathname = path_and_query.to_string();
                    }
                } else if let Some(query_pos) = path_and_query.find('?') {
                    search = path_and_query[query_pos..].to_string();
                    pathname = path_and_query[..query_pos].to_string();
                } else {
                    pathname = path_and_query.to_string();
                }
            } else {
                host = rest.to_string();
            }
        }

        (protocol, host, pathname, search, hash)
    }
}
