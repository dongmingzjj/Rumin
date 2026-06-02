//! getComputedStyle mock — returns reasonable default CSS values.

use super::*;

impl JsEngine {
    /// Setup getComputedStyle global function
    pub fn setup_computed_style(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            var defaults = {
                'display': 'block',
                'position': 'static',
                'top': 'auto',
                'right': 'auto',
                'bottom': 'auto',
                'left': 'auto',
                'width': '0px',
                'height': '0px',
                'margin-top': '0px',
                'margin-right': '0px',
                'margin-bottom': '0px',
                'margin-left': '0px',
                'padding-top': '0px',
                'padding-right': '0px',
                'padding-bottom': '0px',
                'padding-left': '0px',
                'border-top-width': '0px',
                'border-right-width': '0px',
                'border-bottom-width': '0px',
                'border-left-width': '0px',
                'color': 'rgb(0, 0, 0)',
                'background-color': 'rgba(0, 0, 0, 0)',
                'font-size': '16px',
                'font-family': 'Times New Roman',
                'font-weight': '400',
                'line-height': 'normal',
                'text-align': 'start',
                'visibility': 'visible',
                'opacity': '1',
                'overflow': 'visible',
                'z-index': 'auto',
                'float': 'none',
                'clear': 'none',
                'cursor': 'auto',
                'pointer-events': 'auto',
                'transform': 'none',
                'transition': 'none',
                'animation': 'none'
            };

            globalThis.getComputedStyle = function(element) {
                var style = {};
                for (var prop in defaults) {
                    if (defaults.hasOwnProperty(prop)) {
                        style[prop] = defaults[prop];
                    }
                }
                // Override with element.style if available
                if (element && element.style) {
                    for (var s in element.style) {
                        if (element.style.hasOwnProperty(s) && element.style[s]) {
                            style[s] = element.style[s];
                        }
                    }
                }
                style.getPropertyValue = function(prop) {
                    return style[prop] || '';
                };
                style.setProperty = function() {};
                style.removeProperty = function() { return ''; };
                return style;
            };
            globalThis.window = globalThis;
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup getComputedStyle: {:?}", e))?;
        Ok(())
    }
}
