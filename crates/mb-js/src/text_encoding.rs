//! TextEncoder / TextDecoder polyfill (pure JS implementation)

use anyhow::Result;
use super::JsEngine;

impl JsEngine {
    /// Set up TextEncoder and TextDecoder globals via pure JavaScript.
    pub fn setup_text_encoding(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            if (typeof globalThis.TextEncoder === 'undefined') {
                globalThis.TextEncoder = function() {};
                TextEncoder.prototype.encode = function(str) {
                    var arr = [];
                    for (var i = 0; i < str.length; i++) {
                        var c = str.charCodeAt(i);
                        if (c < 128) {
                            arr.push(c);
                        } else if (c < 2048) {
                            arr.push(192 | (c >> 6));
                            arr.push(128 | (c & 63));
                        } else {
                            arr.push(224 | (c >> 12));
                            arr.push(128 | ((c >> 6) & 63));
                            arr.push(128 | (c & 63));
                        }
                    }
                    return new Uint8Array(arr);
                };
            }

            if (typeof globalThis.TextDecoder === 'undefined') {
                globalThis.TextDecoder = function(encoding) {
                    this.encoding = encoding || 'utf-8';
                };
                TextDecoder.prototype.decode = function(bytes) {
                    if (!bytes) return '';
                    var arr = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
                    var str = '';
                    for (var i = 0; i < arr.length; i++) {
                        if (arr[i] < 128) {
                            str += String.fromCharCode(arr[i]);
                        } else if (arr[i] < 224) {
                            str += String.fromCharCode(((arr[i] & 31) << 6) | (arr[i+1] & 63));
                            i++;
                        } else {
                            str += String.fromCharCode(((arr[i] & 15) << 12) | ((arr[i+1] & 63) << 6) | (arr[i+2] & 63));
                            i += 2;
                        }
                    }
                    return str;
                };
            }

            // Also add Uint8Array if not present
            if (typeof Uint8Array === 'undefined') {
                globalThis.Uint8Array = Array;
            }
        })();
        "#;
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: rquickjs::Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow::anyhow!("setup_text_encoding failed: {:?}", e))?;
        Ok(())
    }
}
