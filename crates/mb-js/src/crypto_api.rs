//! crypto.getRandomValues() + crypto.subtle.digest('SHA-256') implementation

use super::*;
use sha2::{Sha256, Digest};
use rquickjs::Function;

impl JsEngine {
    /// Setup crypto.getRandomValues() and crypto.subtle.digest()
    pub fn setup_crypto_api(&mut self) -> Result<()> {
        // First register the native SHA-256 function
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let sha_fn = Function::new(ctx.clone(), |data: rquickjs::Value| -> rquickjs::Result<String> {
                // Extract bytes from Uint8Array-like
                let bytes = if let Some(obj) = data.as_object() {
                    let len: usize = obj.get::<_, Value>("length")
                        .ok()
                        .and_then(|v| v.as_float())
                        .unwrap_or(0.0) as usize;
                    let mut buf = Vec::with_capacity(len);
                    for i in 0..len {
                        if let Ok(b) = obj.get::<_, Value>(i as u32) {
                            buf.push(b.as_float().unwrap_or(0.0) as u8);
                        }
                    }
                    buf
                } else {
                    return Ok(String::new());
                };

                // Compute SHA-256
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                let result = hasher.finalize();
                Ok(format!("{:x}", result))
            })?;
            ctx.globals().set("__sha256_hex__", sha_fn)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup SHA-256 native: {:?}", e))?;

        // Then setup the crypto object with subtle.digest
        let code = r#"
        globalThis.crypto = {
            getRandomValues: function(arr) {
                for (var i = 0; i < arr.length; i++) {
                    arr[i] = Math.floor(Math.random() * 256);
                }
                return arr;
            },
            subtle: {
                digest: function(algorithm, data) {
                    return new Promise(function(resolve, reject) {
                        try {
                            var algo = (typeof algorithm === 'string') ? algorithm : algorithm.name;
                            if (algo.toUpperCase() !== 'SHA-256') {
                                reject(new Error('Unsupported algorithm: ' + algo));
                                return;
                            }
                            // data should be an ArrayBuffer or TypedArray
                            var bytes;
                            if (data instanceof ArrayBuffer) {
                                bytes = new Uint8Array(data);
                            } else if (data.buffer) {
                                bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
                            } else {
                                bytes = new Uint8Array(data);
                            }
                            var hex = __sha256_hex__(bytes);
                            // Convert hex string to ArrayBuffer
                            var arr = new Uint8Array(hex.length / 2);
                            for (var i = 0; i < arr.length; i++) {
                                arr[i] = parseInt(hex.substr(i * 2, 2), 16);
                            }
                            resolve(arr.buffer);
                        } catch(e) {
                            reject(e);
                        }
                    });
                }
            }
        };
        globalThis.window = globalThis;
        "#;

        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to setup crypto API: {:?}", e))?;
        Ok(())
    }
}
