//! Timer infrastructure (setTimeout, setInterval, clearTimeout, clearInterval).

use super::*;

/// A pending timer callback registered via setTimeout/setInterval.
#[derive(Debug, Clone)]
pub struct PendingCallback {
    pub timer_id: u32,
    pub delay_ms: u32,
    pub repeating: bool,
}

impl JsEngine {
    /// Register the global setTimeout / setInterval / clearTimeout / clearInterval functions.
    pub fn setup_timers(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            var __next_timer_id__ = 1;
            var __pending_callbacks__ = [];

            globalThis.setTimeout = function(fn, delay) {
                var id = __next_timer_id__++;
                __pending_callbacks__.push({
                    timer_id: id,
                    callback: fn,
                    delay_ms: delay || 0,
                    repeating: false
                });
                return id;
            };

            globalThis.setInterval = function(fn, delay) {
                var id = __next_timer_id__++;
                __pending_callbacks__.push({
                    timer_id: id,
                    callback: fn,
                    delay_ms: delay || 0,
                    repeating: true
                });
                return id;
            };

            globalThis.clearTimeout = function(id) {
                for (var i = 0; i < __pending_callbacks__.length; i++) {
                    if (__pending_callbacks__[i].timer_id === id) {
                        __pending_callbacks__.splice(i, 1);
                        return;
                    }
                }
            };

            globalThis.clearInterval = globalThis.clearTimeout;

            globalThis.__drainTimerCallbacks__ = function() {
                var out = __pending_callbacks__.slice();
                __pending_callbacks__.length = 0;
                return out;
            };

            globalThis.__executeAllTimerCallbacks__ = function() {
                var batch = __pending_callbacks__.slice();
                __pending_callbacks__.length = 0;
                for (var i = 0; i < batch.length; i++) {
                    var cb = batch[i];
                    if (typeof cb.callback === 'function') {
                        try { cb.callback(); } catch(e) {}
                    }
                    if (cb.repeating) {
                        __pending_callbacks__.push({
                            timer_id: cb.timer_id,
                            callback: cb.callback,
                            delay_ms: cb.delay_ms,
                            repeating: true
                        });
                    }
                }
                return batch.length;
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup timers: {:?}", e))?;
        Ok(())
    }

    /// Drain pending timer callbacks from the JS side and return them as Rust-side metadata.
    pub fn drain_callbacks(&mut self) -> Result<Vec<PendingCallback>> {
        let code = r#"
        (function() {
            if (typeof __drainTimerCallbacks__ === 'undefined') return [];
            return __drainTimerCallbacks__();
        })()
        "#;

        let callbacks = self.context.with(|ctx| -> rquickjs::Result<Vec<PendingCallback>> {
            let result: Value = ctx.eval(code)?;
            let mut callbacks = Vec::new();
            if let Some(arr) = result.as_object() {
                let len: usize = arr.get::<_, Value>("length")
                    .ok()
                    .and_then(|v| value_to_f64(&v))
                    .unwrap_or(0.0) as usize;

                for i in 0..len {
                    if let Ok(entry) = arr.get::<_, Value>(i as u32) {
                        if let Some(obj) = entry.as_object() {
                            let timer_id = obj.get::<_, Value>("timer_id")
                                .ok()
                                .and_then(|v| value_to_f64(&v))
                                .unwrap_or(0.0) as u32;

                            let delay_ms = obj.get::<_, Value>("delay_ms")
                                .ok()
                                .and_then(|v| value_to_f64(&v))
                                .unwrap_or(0.0) as u32;

                            let repeating = obj.get::<_, Value>("repeating")
                                .ok()
                                .and_then(|v| value_to_bool(&v))
                                .unwrap_or(false);

                            callbacks.push(PendingCallback {
                                timer_id,
                                delay_ms,
                                repeating,
                            });
                        }
                    }
                }
            }
            Ok(callbacks)
        }).map_err(|e| anyhow!("Failed to drain callbacks: {:?}", e))?;
        Ok(callbacks)
    }

    /// Drain and execute pending timer callbacks, up to 5 rounds.
    /// Also drains microtasks after each round to handle timer callbacks
    /// that create Promises (e.g. setTimeout(() => { fetch(...).then(...) }, 0)).
    pub fn drain_and_execute_timers(&mut self) -> Result<()> {
        for _round in 0..5u32 {
            let code = r#"
            (function() {
                if (typeof __executeAllTimerCallbacks__ === 'undefined') return 0;
                return __executeAllTimerCallbacks__();
            })()
            "#;

            let count = self.context.with(|ctx| -> rquickjs::Result<u32> {
                let result: Value = ctx.eval(code)?;
                // Drain microtasks inside ctx.with() — timer callbacks may create
                // Promises whose .then() callbacks need to run
                for _ in 0..50 {
                    if !ctx.execute_pending_job() { break; }
                }
                Ok(result.as_float().unwrap_or(0.0) as u32)
            }).map_err(|e| anyhow!("Failed to execute timer callbacks: {:?}", e))?;

            if count == 0 {
                break;
            }
        }
        Ok(())
    }
}
