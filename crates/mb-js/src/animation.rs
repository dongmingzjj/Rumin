//! requestAnimationFrame / cancelAnimationFrame Web API.

use super::*;

impl JsEngine {
    /// Register requestAnimationFrame and cancelAnimationFrame on globalThis.
    pub fn setup_animation_frames(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            globalThis.__next_raf_id__ = 1;
            globalThis.__pending_raf__ = [];

            globalThis.requestAnimationFrame = function(callback) {
                var id = globalThis.__next_raf_id__++;
                globalThis.__pending_raf__.push({ id: id, callback: callback });
                return id;
            };

            globalThis.cancelAnimationFrame = function(id) {
                for (var i = 0; i < globalThis.__pending_raf__.length; i++) {
                    if (globalThis.__pending_raf__[i].id === id) {
                        globalThis.__pending_raf__.splice(i, 1);
                        return;
                    }
                }
            };

            globalThis.__executeAllRAFCallbacks__ = function() {
                var batch = globalThis.__pending_raf__.slice();
                globalThis.__pending_raf__.length = 0;
                var now = (typeof performance !== 'undefined' && performance.now) ? performance.now() : Date.now();
                for (var i = 0; i < batch.length; i++) {
                    if (typeof batch[i].callback === 'function') {
                        try { batch[i].callback(now); } catch(e) {}
                    }
                }
                return batch.length;
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup animation frames: {:?}", e))?;
        Ok(())
    }
}
