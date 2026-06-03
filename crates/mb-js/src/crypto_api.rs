//! crypto.getRandomValues() implementation (pure JS)

use super::*;

impl JsEngine {
    /// Setup crypto.getRandomValues()
    pub fn setup_crypto_api(&mut self) -> Result<()> {
        let code = r#"
        globalThis.crypto = {
            getRandomValues: function(arr) {
                for (var i = 0; i < arr.length; i++) {
                    arr[i] = Math.floor(Math.random() * 256);
                }
                return arr;
            },
            subtle: undefined
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
