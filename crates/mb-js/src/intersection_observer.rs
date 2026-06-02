//! IntersectionObserver Web API (simplified).

use super::*;

impl JsEngine {
    /// Register IntersectionObserver constructor (simplified: async trigger via setTimeout).
    /// Must be called after setup_timers (depends on setTimeout).
    pub fn setup_intersection_observer(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            globalThis.IntersectionObserver = function(callback, options) {
                this._callback = callback;
                this._options = options || {};
                this._targets = [];
                this._records = [];
            };

            globalThis.IntersectionObserver.prototype.observe = function(target) {
                var self = this;
                this._targets.push(target);
                // Simplified: trigger callback asynchronously with isIntersecting=true
                setTimeout(function() {
                    var entry = {
                        target: target,
                        isIntersecting: true,
                        intersectionRatio: 1,
                        boundingClientRect: { top: 0, left: 0, width: 0, height: 0, right: 0, bottom: 0 },
                        rootBounds: null,
                        time: Date.now()
                    };
                    self._records.push(entry);
                    if (typeof self._callback === 'function') {
                        try { self._callback([entry], self); } catch(e) {}
                    }
                }, 0);
            };

            globalThis.IntersectionObserver.prototype.unobserve = function(target) {
                var idx = this._targets.indexOf(target);
                if (idx >= 0) this._targets.splice(idx, 1);
            };

            globalThis.IntersectionObserver.prototype.disconnect = function() {
                this._targets = [];
            };

            globalThis.IntersectionObserver.prototype.takeRecords = function() {
                var records = this._records.slice();
                this._records.length = 0;
                return records;
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup intersection observer: {:?}", e))?;
        Ok(())
    }
}
