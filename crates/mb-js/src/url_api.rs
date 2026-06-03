//! URL and URLSearchParams API implementations (pure JS)

use super::*;

impl JsEngine {
    /// Setup URL and URLSearchParams constructors
    pub fn setup_url_api(&mut self) -> Result<()> {
        let code = r#"
        // URL constructor
        (function() {
            var _urlPattern = /^(?:([a-z][a-z0-9+\-.]*):)?\/\/([^\/?#:]*)(?::(\d+))?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/i;
            
            function _parseURL(urlStr, base) {
                var m;
                if (base) {
                    var b = _parseURL(base, null);
                    if (!b) return null;
                        if (!urlStr || urlStr === '') return b;
                        if (urlStr.charAt(0) === '/') {
                            urlStr = b.protocol + '//' + b.hostname + (b.port ? ':' + b.port : '') + urlStr;
                        } else if (urlStr.indexOf('://') >= 0) {
                            // absolute
                        } else {
                            // relative to base path
                            var basePath = b.pathname || '/';
                            var lastSlash = basePath.lastIndexOf('/');
                            urlStr = b.protocol + '//' + b.hostname + (b.port ? ':' + b.port : '') + basePath.substring(0, lastSlash + 1) + urlStr;
                        }
                }
                m = _urlPattern.exec(urlStr);
                if (!m) return null;
                return {
                    protocol: (m[1] || '').toLowerCase() + ':',
                    hostname: m[2] || '',
                    port: m[3] || '',
                    pathname: m[4] || '/',
                    search: m[5] || '',
                    hash: m[6] || ''
                };
            }

            function URL(url, base) {
                if (url === undefined) throw new TypeError('URL requires at least 1 argument');
                var s = String(url);
                var parsed = _parseURL(s, base ? String(base) : null);
                if (!parsed) throw new TypeError('Invalid URL: ' + s);
                this._protocol = parsed.protocol;
                this._hostname = parsed.hostname;
                this._port = parsed.port;
                this._pathname = parsed.pathname;
                this._search = parsed.search;
                this._hash = parsed.hash;
            }

            Object.defineProperty(URL.prototype, 'protocol', {
                get: function() { return this._protocol; },
                set: function(v) { this._protocol = String(v); }
            });
            Object.defineProperty(URL.prototype, 'hostname', {
                get: function() { return this._hostname; },
                set: function(v) { this._hostname = String(v); }
            });
            Object.defineProperty(URL.prototype, 'port', {
                get: function() { return this._port; },
                set: function(v) { this._port = String(v); }
            });
            Object.defineProperty(URL.prototype, 'pathname', {
                get: function() { return this._pathname; },
                set: function(v) { this._pathname = String(v); }
            });
            Object.defineProperty(URL.prototype, 'search', {
                get: function() { return this._search; },
                set: function(v) { this._search = String(v); }
            });
            Object.defineProperty(URL.prototype, 'hash', {
                get: function() { return this._hash; },
                set: function(v) { this._hash = String(v); }
            });
            Object.defineProperty(URL.prototype, 'host', {
                get: function() {
                    return this._hostname + (this._port ? ':' + this._port : '');
                },
                set: function(v) {
                    var s = String(v);
                    var idx = s.lastIndexOf(':');
                    if (idx >= 0) {
                        this._hostname = s.substring(0, idx);
                        this._port = s.substring(idx + 1);
                    } else {
                        this._hostname = s;
                        this._port = '';
                    }
                }
            });
            Object.defineProperty(URL.prototype, 'origin', {
                get: function() {
                    return this._protocol + '//' + this._hostname + (this._port ? ':' + this._port : '');
                }
            });
            Object.defineProperty(URL.prototype, 'href', {
                get: function() {
                    return this.origin + this._pathname + this._search + this._hash;
                },
                set: function(v) {
                    var parsed = _parseURL(String(v), null);
                    if (!parsed) throw new TypeError('Invalid URL');
                    this._protocol = parsed.protocol;
                    this._hostname = parsed.hostname;
                    this._port = parsed.port;
                    this._pathname = parsed.pathname;
                    this._search = parsed.search;
                    this._hash = parsed.hash;
                }
            });

            URL.prototype.toString = function() { return this.href; };
            URL.prototype.toJSON = function() { return this.href; };

            globalThis.URL = URL;

            // URLSearchParams constructor
            function URLSearchParams(init) {
                this._entries = [];
                if (typeof init === 'string') {
                    if (init.charAt(0) === '?') init = init.substring(1);
                    if (init.length > 0) {
                        var pairs = init.split('&');
                        for (var i = 0; i < pairs.length; i++) {
                            var pair = pairs[i];
                            var eqIdx = pair.indexOf('=');
                            if (eqIdx >= 0) {
                                this._entries.push([decodeURIComponent(pair.substring(0, eqIdx)), decodeURIComponent(pair.substring(eqIdx + 1))]);
                            } else {
                                this._entries.push([decodeURIComponent(pair), '']);
                            }
                        }
                    }
                } else if (init && typeof init === 'object') {
                    if (Array.isArray(init)) {
                        for (var i = 0; i < init.length; i++) {
                            var item = init[i];
                            if (Array.isArray(item) && item.length >= 2) {
                                this._entries.push([String(item[0]), String(item[1])]);
                            }
                        }
                    } else {
                        var keys = Object.keys(init);
                        for (var i = 0; i < keys.length; i++) {
                            this._entries.push([keys[i], String(init[keys[i]])]);
                        }
                    }
                }
            }

            URLSearchParams.prototype.get = function(key) {
                for (var i = 0; i < this._entries.length; i++) {
                    if (this._entries[i][0] === key) return this._entries[i][1];
                }
                return null;
            };
            URLSearchParams.prototype.getAll = function(key) {
                var result = [];
                for (var i = 0; i < this._entries.length; i++) {
                    if (this._entries[i][0] === key) result.push(this._entries[i][1]);
                }
                return result;
            };
            URLSearchParams.prototype.set = function(key, value) {
                var found = false;
                for (var i = this._entries.length - 1; i >= 0; i--) {
                    if (this._entries[i][0] === key) {
                        if (!found) {
                            this._entries[i][1] = String(value);
                            found = true;
                        } else {
                            this._entries.splice(i, 1);
                        }
                    }
                }
                if (!found) this._entries.push([key, String(value)]);
            };
            URLSearchParams.prototype.append = function(key, value) {
                this._entries.push([key, String(value)]);
            };
            URLSearchParams.prototype.delete = function(key) {
                for (var i = this._entries.length - 1; i >= 0; i--) {
                    if (this._entries[i][0] === key) this._entries.splice(i, 1);
                }
            };
            URLSearchParams.prototype.has = function(key) {
                for (var i = 0; i < this._entries.length; i++) {
                    if (this._entries[i][0] === key) return true;
                }
                return false;
            };
            URLSearchParams.prototype.toString = function() {
                var parts = [];
                for (var i = 0; i < this._entries.length; i++) {
                    parts.push(encodeURIComponent(this._entries[i][0]) + '=' + encodeURIComponent(this._entries[i][1]));
                }
                return parts.join('&');
            };
            URLSearchParams.prototype.entries = function() { return this._entries.slice(); };
            URLSearchParams.prototype.keys = function() {
                var result = [];
                for (var i = 0; i < this._entries.length; i++) result.push(this._entries[i][0]);
                return result;
            };
            URLSearchParams.prototype.values = function() {
                var result = [];
                for (var i = 0; i < this._entries.length; i++) result.push(this._entries[i][1]);
                return result;
            };
            URLSearchParams.prototype.forEach = function(fn) {
                for (var i = 0; i < this._entries.length; i++) {
                    fn(this._entries[i][1], this._entries[i][0], this);
                }
            };

            globalThis.URLSearchParams = URLSearchParams;
        })();
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup URL API: {:?}", e))?;
        Ok(())
    }
}
