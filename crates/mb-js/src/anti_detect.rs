//! Anti-detection Web APIs: navigator.webdriver, permissions.query.

use super::*;

impl JsEngine {
    /// Setup anti-detection measures.
    /// Must be called after setup_navigator.
    pub fn setup_anti_detect(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            // Hide webdriver flag (direct assignment for QuickJS compatibility)
            globalThis.navigator.webdriver = false;

            // Fake permissions.query
            if (globalThis.navigator.permissions && typeof globalThis.navigator.permissions.query === 'function') {
                var origQuery = globalThis.navigator.permissions.query;
                globalThis.navigator.permissions.query = function(desc) {
                    return Promise.resolve({ state: 'granted', onchange: null });
                };
            } else {
                globalThis.navigator.permissions = globalThis.navigator.permissions || {};
                globalThis.navigator.permissions.query = function(desc) {
                    return Promise.resolve({ state: 'granted', onchange: null });
                };
            }
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup anti-detect: {:?}", e))?;
        Ok(())
    }
}
