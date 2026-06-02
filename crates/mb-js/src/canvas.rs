//! Canvas getContext mock — adds getContext('2d') to element prototypes.

use super::*;

impl JsEngine {
    /// Setup canvas getContext — must be called AFTER bind_dom()
    /// so that __dom_element_proto__ and document.createElement already exist.
    pub fn setup_canvas(&mut self) -> Result<()> {
        // setup_canvas is intentionally a no-op during run_setup().
        // Call setup_canvas_after_dom() after bind_dom() to activate.
        Ok(())
    }

    /// Patch __dom_element_proto__ and document.createElement for canvas support.
    /// Call this AFTER bind_dom().
    pub fn setup_canvas_after_dom(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            var FIXED_PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQABNjN9GQAAAAlwSFlzAAAWJQAAFiUBSVIk8AAAABlJREFUCNdjYGBg+A8AAQQBAScLuN4AAAAASUVORK5CYII=';

            function createCanvasContext() {
                return {
                    fillStyle: '#000000',
                    strokeStyle: '#000000',
                    lineWidth: 1,
                    font: '10px sans-serif',
                    textAlign: 'start',
                    textBaseline: 'alphabetic',
                    globalAlpha: 1.0,
                    _width: 300,
                    _height: 150,
                    fillRect: function(x, y, w, h) {},
                    strokeRect: function(x, y, w, h) {},
                    clearRect: function(x, y, w, h) {},
                    fillText: function(text, x, y, maxWidth) {},
                    strokeText: function(text, x, y, maxWidth) {},
                    measureText: function(text) {
                        return { width: text.length * 8, height: 16 };
                    },
                    beginPath: function() {},
                    closePath: function() {},
                    moveTo: function(x, y) {},
                    lineTo: function(x, y) {},
                    arc: function(x, y, r, start, end, ccw) {},
                    fill: function() {},
                    stroke: function() {},
                    drawImage: function() {},
                    save: function() {},
                    restore: function() {},
                    translate: function(x, y) {},
                    rotate: function(angle) {},
                    scale: function(x, y) {},
                    toDataURL: function(type, quality) {
                        return FIXED_PNG;
                    },
                    getImageData: function(sx, sy, sw, sh) {
                        return {
                            width: sw || 0,
                            height: sh || 0,
                            data: new Uint8ClampedArray((sw || 0) * (sh || 0) * 4)
                        };
                    },
                    putImageData: function() {},
                    createImageData: function(w, h) {
                        return { width: w, height: h, data: new Uint8ClampedArray(w * h * 4) };
                    }
                };
            }

            // Patch __dom_element_proto__ if it exists
            if (typeof __dom_element_proto__ !== 'undefined') {
                __dom_element_proto__.getContext = function(type) {
                    if (type === '2d') {
                        return createCanvasContext();
                    }
                    return null;
                };
            }

            // Wrap createElement to add canvas-specific methods
            if (typeof document !== 'undefined' && document.createElement) {
                var origCreate = document.createElement.bind(document);
                document.createElement = function(tag) {
                    var el = origCreate(tag);
                    if (tag && tag.toLowerCase && tag.toLowerCase() === 'canvas') {
                        el.getContext = function(type) {
                            if (type === '2d') {
                                return createCanvasContext();
                            }
                            return null;
                        };
                        el.toDataURL = function() { return FIXED_PNG; };
                        el.width = 300;
                        el.height = 150;
                    }
                    return el;
                };
            }

            globalThis.window = globalThis;
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup canvas: {:?}", e))?;
        Ok(())
    }
}
