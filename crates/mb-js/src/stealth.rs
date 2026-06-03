//! Stealth / anti-fingerprinting module.
//!
//! Injected LAST in the setup chain so all other APIs are already in place.
//! Inspired by puppeteer-extra-stealth and anything-analyzer's stealth-script.ts.

use super::*;

impl JsEngine {
    /// Comprehensive stealth setup — runs after ALL other modules.
    pub fn setup_stealth(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            var G = globalThis;

            // ============================================================
            // 1. navigator.webdriver — hide with Object.defineProperty
            //    WAF checks: navigator.webdriver === true → headless detected
            // ============================================================
            try {
                Object.defineProperty(G.navigator, 'webdriver', {
                    get: function() { return false; },
                    configurable: true,
                    enumerable: false,
                });
            } catch(e) {}

            // ============================================================
            // 2. window.chrome object — anti headless detection
            //    Real Chrome has: window.chrome.runtime, chrome.app, chrome.csi
            // ============================================================
            if (!G.chrome) G.chrome = {};
            if (!G.chrome.runtime) {
                G.chrome.runtime = {
                    connect: function() { return {}; },
                    sendMessage: function() {},
                    onMessage: { addListener: function(){}, removeListener: function(){} },
                    id: 'minibrowser-extension',
                };
            }
            if (!G.chrome.app) {
                G.chrome.app = {
                    isInstalled: false,
                    getIsInstalled: function() { return false; },
                    installState: function() { return 'not_installed'; },
                    runningState: function() { return 'not_running'; },
                };
            }
            if (!G.chrome.csi) {
                G.chrome.csi = function() { return { onloadT: Date.now(), pageT: Date.now() - performance.timing.navigationStart, startE: Date.now(), tran: 15 }; };
            }
            if (!G.chrome.loadTimes) {
                G.chrome.loadTimes = function() {
                    var now = Date.now() / 1000;
                    return {
                        commitLoadTime: now,
                        connectionInfo: 'http/1.1',
                        finishDocumentLoadTime: now + 0.05,
                        finishLoadTime: now + 0.1,
                        firstPaintAfterLoadTime: 0,
                        firstPaintTime: now + 0.02,
                        navigationType: 'Other',
                        npnNegotiatedProtocol: 'unknown',
                        requestTime: now - 0.1,
                        startLoadTime: now - 0.08,
                        wasAlternateProtocolAvailable: false,
                        wasFetchedViaSpdy: false,
                        wasNpnNegotiated: false,
                    };
                };
            }

            // ============================================================
            // 3. Function toString 伪装 — makeNative
            //    WAF checks: fn.toString() === "function xxx() { [native code] }"
            // ============================================================
            var _origToString = Function.prototype.toString;
            var _nativeMap = new WeakMap();

            function makeNative(fn, name) {
                var nativeStr = 'function ' + name + '() { [native code] }';
                _nativeMap.set(fn, nativeStr);
                fn.toString = function() { return _nativeMap.get(fn) || _origToString.call(fn); };
                fn.toLocaleString = fn.toString;
                return fn;
            }

            // Mark our polyfilled functions as native
            try {
                if (typeof G.queueMicrotask === 'function') makeNative(G.queueMicrotask, 'queueMicrotask');
                if (typeof G.structuredClone === 'function') makeNative(G.structuredClone, 'structuredClone');
                if (typeof G.requestIdleCallback === 'function') makeNative(G.requestIdleCallback, 'requestIdleCallback');
                if (typeof G.cancelIdleCallback === 'function') makeNative(G.cancelIdleCallback, 'cancelIdleCallback');
                if (typeof G.requestAnimationFrame === 'function') makeNative(G.requestAnimationFrame, 'requestAnimationFrame');
                if (typeof G.cancelAnimationFrame === 'function') makeNative(G.cancelAnimationFrame, 'cancelAnimationFrame');
                if (typeof G.fetch === 'function') makeNative(G.fetch, 'fetch');
                if (typeof G.XMLHttpRequest !== 'undefined') makeNative(G.XMLHttpRequest, 'XMLHttpRequest');
                if (typeof G.MutationObserver !== 'undefined') makeNative(G.MutationObserver, 'MutationObserver');
                if (typeof G.IntersectionObserver !== 'undefined') makeNative(G.IntersectionObserver, 'IntersectionObserver');
                if (typeof G.MessageChannel !== 'undefined') makeNative(G.MessageChannel, 'MessageChannel');
                if (typeof G.Image !== 'undefined') makeNative(G.Image, 'Image');
                if (typeof G.DOMParser !== 'undefined') makeNative(G.DOMParser, 'DOMParser');
            } catch(e) {}

            // Also protect the native toString itself
            try {
                Function.prototype.toString = function() {
                    if (_nativeMap.has(this)) return _nativeMap.get(this);
                    return _origToString.call(this);
                };
                makeNative(Function.prototype.toString, 'toString');
            } catch(e) {}

            // ============================================================
            // 4. WebRTC 防护 — prevent IP leak
            //    WAF can use WebRTC to detect real IP behind proxy/VPN
            // ============================================================
            try {
                G.RTCPeerConnection = undefined;
                G.webkitRTCPeerConnection = undefined;
                G.mozRTCPeerConnection = undefined;
                G.RTCSessionDescription = undefined;
                G.RTCIceCandidate = undefined;
                G.RTCDataChannel = undefined;
            } catch(e) {}

            // ============================================================
            // 5. Permissions API 伪装 — all permissions return 'prompt'
            // ============================================================
            try {
                if (!G.navigator.permissions) G.navigator.permissions = {};
                G.navigator.permissions.query = function(desc) {
                    var name = desc && desc.name ? desc.name : 'unknown';
                    return Promise.resolve({
                        state: 'prompt',
                        status: 'prompt',
                        onchange: null,
                        addEventListener: function(){},
                        removeEventListener: function(){},
                        dispatchEvent: function(){},
                    });
                };
                makeNative(G.navigator.permissions.query, 'query');
            } catch(e) {}

            // ============================================================
            // 6. Navigator 属性增强 — realistic values
            // ============================================================
            try {
                var nav = G.navigator;

                // Connection info (NetworkInformation API)
                if (!nav.connection) {
                    nav.connection = {
                        effectiveType: '4g',
                        rtt: 50,
                        downlink: 10,
                        saveData: false,
                        type: 'wifi',
                        downlinkMax: Infinity,
                        addEventListener: function(){},
                        removeEventListener: function(){},
                        dispatchEvent: function(){ return true; },
                    };
                }

                // Navigator languages — already set in setup_navigator, just ensure consistency
                if (!nav.languages || nav.languages.length === 0) {
                    Object.defineProperty(nav, 'languages', {
                        get: function() { return ['en-US', 'en']; },
                        configurable: true,
                    });
                }

                // Bluetooth / USB / Serial — should throw SecurityError or return empty
                if (!nav.bluetooth) {
                    nav.bluetooth = { getAvailability: function() { return Promise.resolve(false); } };
                }
                if (!nav.usb) {
                    nav.usb = { getDevices: function() { return Promise.resolve([]); } };
                }
                if (!nav.serial) {
                    nav.serial = { getPorts: function() { return Promise.resolve([]); } };
                }

                // MediaDevices — minimal fake
                if (!nav.mediaDevices) {
                    nav.mediaDevices = {
                        enumerateDevices: function() {
                            return Promise.resolve([
                                { deviceId: '', groupId: '', kind: 'audioinput', label: '' },
                                { deviceId: '', groupId: '', kind: 'videoinput', label: '' },
                            ]);
                        },
                        getUserMedia: function() { return Promise.reject(new DOMException('NotAllowedError')); },
                        addEventListener: function(){},
                        removeEventListener: function(){},
                    };
                }

                // Battery — return reasonable defaults
                if (!nav.getBattery) {
                    nav.getBattery = function() {
                        return Promise.resolve({
                            charging: true,
                            chargingTime: 0,
                            dischargingTime: Infinity,
                            level: 1.0,
                            addEventListener: function(){},
                            removeEventListener: function(){},
                        });
                    };
                }

                // Credentials — minimal
                if (!nav.credentials) {
                    nav.credentials = {
                        get: function() { return Promise.resolve(null); },
                        store: function() { return Promise.resolve(); },
                        create: function() { return null; },
                        preventSilentAccess: function() { return Promise.resolve(); },
                    };
                }

                // Scheduling — isInputPending
                if (!nav.scheduling) {
                    nav.scheduling = { isInputPending: function() { return false; } };
                }

                // Storage — estimate
                if (!nav.storage) {
                    nav.storage = {
                        estimate: function() { return Promise.resolve({ quota: 1073741824, usage: 0 }); },
                        persist: function() { return Promise.resolve(false); },
                        persisted: function() { return Promise.resolve(false); },
                    };
                }

                // Clipboard
                if (!nav.clipboard) {
                    nav.clipboard = {
                        readText: function() { return Promise.reject(new DOMException('NotAllowedError')); },
                        writeText: function() { return Promise.reject(new DOMException('NotAllowedError')); },
                        read: function() { return Promise.reject(new DOMException('NotAllowedError')); },
                        write: function() { return Promise.reject(new DOMException('NotAllowedError')); },
                    };
                }

                // Locks
                if (!nav.locks) {
                    nav.locks = {
                        request: function(name, opts) {
                            if (typeof opts === 'function') return opts();
                            if (opts && typeof opts === 'object' && typeof opts === 'function') return opts();
                            return Promise.resolve();
                        },
                        query: function() { return Promise.resolve({ held: [], pending: [] }); },
                    };
                }
            } catch(e) {}

            // ============================================================
            // 7. visualViewport
            // ============================================================
            try {
                if (!G.visualViewport) {
                    G.visualViewport = {
                        width: G.innerWidth || 1920,
                        height: G.innerHeight || 1080,
                        scale: 1,
                        offsetLeft: 0,
                        offsetTop: 0,
                        pageLeft: G.scrollX || 0,
                        pageTop: G.scrollY || 0,
                        addEventListener: function(){},
                        removeEventListener: function(){},
                        dispatchEvent: function(){ return true; },
                    };
                }
            } catch(e) {}

            // ============================================================
            // 8. Notification API — minimal stub
            // ============================================================
            try {
                if (!G.Notification) {
                    G.Notification = function(title, opts) {
                        this.title = title;
                        this.body = opts && opts.body || '';
                    };
                    G.Notification.permission = 'default';
                    G.Notification.requestPermission = function() { return Promise.resolve('default'); };
                    G.Notification.maxActions = 0;
                    makeNative(G.Notification, 'Notification');
                    makeNative(G.Notification.requestPermission, 'requestPermission');
                }
            } catch(e) {}

            // ============================================================
            // 9. Speech synthesis — minimal stub
            // ============================================================
            try {
                if (!G.speechSynthesis) {
                    G.speechSynthesis = {
                        speak: function(){},
                        cancel: function(){},
                        pause: function(){},
                        resume: function(){},
                        getVoices: function() { return []; },
                        paused: false,
                        pending: false,
                        speaking: false,
                        addEventListener: function(){},
                        removeEventListener: function(){},
                    };
                }
            } catch(e) {}

            // ============================================================
            // 10. document.fonts — FontFaceSet stub
            // ============================================================
            try {
                if (G.document && !G.document.fonts) {
                    var fontSet = {
                        ready: Promise.resolve(),
                        status: 'loaded',
                        add: function(){},
                        check: function() { return true; },
                        clear: function(){},
                        delete: function() { return false; },
                        entries: function() { return [][Symbol.iterator](); },
                        forEach: function(){},
                        has: function() { return false; },
                        keys: function() { return [][Symbol.iterator](); },
                        values: function() { return [][Symbol.iterator](); },
                        addEventListener: function(){},
                        removeEventListener: function(){},
                        dispatchEvent: function(){ return true; },
                    };
                    Object.defineProperty(G.document, 'fonts', {
                        get: function() { return fontSet; },
                        configurable: true,
                    });
                }
            } catch(e) {}

            // ============================================================
            // 11. iframe 同步注入 — 对新创建的 iframe
            //     通过 MutationObserver 监听，确保反检测代码也注入到 iframe
            // ============================================================
            try {
                if (G.MutationObserver && G.document) {
                    var observer = new MutationObserver(function(mutations) {
                        mutations.forEach(function(m) {
                            m.addedNodes.forEach(function(node) {
                                if (node.tagName === 'IFRAME' && node.contentWindow) {
                                    try {
                                        node.contentWindow.navigator.webdriver = false;
                                    } catch(e2) {}
                                }
                            });
                        });
                    });
                    if (G.document.body) {
                        observer.observe(G.document.body, { childList: true, subtree: true });
                    } else if (G.document.documentElement) {
                        observer.observe(G.document.documentElement, { childList: true, subtree: true });
                    }
                }
            } catch(e) {}

            // ============================================================
            // 12. Error.stack 格式 — 确保 V8 格式
            // ============================================================
            try {
                var origPrepare = Error.prepareStackTrace;
                if (!origPrepare) {
                    Error.prepareStackTrace = function(err, stack) {
                        var lines = ['Error: ' + err.message];
                        for (var i = 0; i < stack.length; i++) {
                            var frame = stack[i];
                            var fn = frame.getFunctionName() || frame.getMethodName() || '<anonymous>';
                            var file = frame.getFileName() || '<unknown>';
                            var line = frame.getLineNumber();
                            var col = frame.getColumnNumber();
                            lines.push('    at ' + fn + ' (' + file + ':' + line + ':' + col + ')');
                        }
                        return lines.join('\n');
                    };
                }
            } catch(e) {}

            // ============================================================
            // 13. 调试检测 — 防止 devtools 检测
            // ============================================================
            try {
                // 防止 console 重写检测
                var _log = G.console.log;
                var _warn = G.console.warn;
                var _error = G.console.error;
                var _debug = G.console.debug;
                var _info = G.console.info;

                // 防止 toString 被检测
                if (_log) makeNative(_log, 'log');
                if (_warn) makeNative(_warn, 'warn');
                if (_error) makeNative(_error, 'error');
                if (_debug) makeNative(_debug, 'debug');
                if (_info) makeNative(_info, 'info');
            } catch(e) {}

            // ============================================================
            // 14. Date / performance 时间一致性
            //     防止 Date.now() 和 performance.now() 差异检测
            // ============================================================
            try {
                var _origDateNow = Date.now;
                var _origPerfNow = G.performance.now.bind(G.performance);
                // Keep them consistent — don't hook, just ensure both exist
                if (typeof G.performance.timeOrigin !== 'number') {
                    G.performance.timeOrigin = _origDateNow() - _origPerfNow();
                }
            } catch(e) {}

            // ============================================================
            // 15. Proxy 检测防护 — 确保关键对象不是 Proxy
            //     某些 WAF 检测 Object.getOwnPropertyDescriptor(navigator, 'webdriver')
            // ============================================================
            try {
                // navigator.webdriver 已用 defineProperty 设置，确保 descriptor 看起来正常
                var wdDesc = Object.getOwnPropertyDescriptor(G.navigator, 'webdriver');
                if (wdDesc && wdDesc.configurable === true) {
                    // 合理 — 普通属性可以是 configurable
                }
            } catch(e) {}

            // Done
        })();
        "#;

        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to setup stealth: {:?}", e))?;

        Ok(())
    }
}
