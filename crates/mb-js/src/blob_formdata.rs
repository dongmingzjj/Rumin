//! Blob and FormData implementations (pure JS mock)

use super::*;

impl JsEngine {
    /// Setup Blob and FormData constructors
    pub fn setup_blob_formdata(&mut self) -> Result<()> {
        let code = r#"
        // Blob constructor
        (function() {
            function Blob(parts, options) {
                var opts = options || {};
                this.type = opts.type || '';
                var chunks = [];
                if (parts && parts.length) {
                    for (var i = 0; i < parts.length; i++) {
                        var part = parts[i];
                        if (typeof part === 'string') {
                            chunks.push(part);
                        } else if (part instanceof Blob) {
                            chunks.push(part._data || '');
                        } else if (part && part.buffer) {
                            // ArrayBuffer or TypedArray
                            chunks.push(String.fromCharCode.apply(null, new Uint8Array(part.buffer || part)));
                        } else {
                            chunks.push(String(part));
                        }
                    }
                }
                this._data = chunks.join('');
                Object.defineProperty(this, 'size', {
                    get: function() { return this._data.length; }
                });
            }
            Blob.prototype.text = function() {
                var data = this._data;
                return Promise.resolve(data);
            };
            Blob.prototype.arrayBuffer = function() {
                var data = this._data;
                var buf = new ArrayBuffer(data.length);
                var view = new Uint8Array(buf);
                for (var i = 0; i < data.length; i++) {
                    view[i] = data.charCodeAt(i);
                }
                return Promise.resolve(buf);
            };
            Blob.prototype.slice = function(start, end, contentType) {
                var s = start || 0;
                var e = end === undefined ? this._data.length : end;
                var newBlob = new Blob([], { type: contentType || '' });
                newBlob._data = this._data.substring(s, e);
                return newBlob;
            };
            globalThis.Blob = Blob;
        })();

        // FormData constructor
        (function() {
            function FormData(form) {
                this._data = {};
                if (form && form.tagName === 'FORM' && form.elements) {
                    for (var i = 0; i < form.elements.length; i++) {
                        var el = form.elements[i];
                        if (el.name) {
                            this.append(el.name, el.value || '');
                        }
                    }
                }
            }
            FormData.prototype.append = function(name, value) {
                if (!this._data[name]) this._data[name] = [];
                this._data[name].push(String(value));
            };
            FormData.prototype.get = function(name) {
                return this._data[name] && this._data[name].length > 0 ? this._data[name][0] : null;
            };
            FormData.prototype.getAll = function(name) {
                return this._data[name] ? this._data[name].slice() : [];
            };
            FormData.prototype.set = function(name, value) {
                this._data[name] = [String(value)];
            };
            FormData.prototype.delete = function(name) {
                delete this._data[name];
            };
            FormData.prototype.has = function(name) {
                return this._data.hasOwnProperty(name) && this._data[name].length > 0;
            };
            FormData.prototype.entries = function() {
                var result = [];
                var keys = Object.keys(this._data);
                for (var i = 0; i < keys.length; i++) {
                    var values = this._data[keys[i]];
                    for (var j = 0; j < values.length; j++) {
                        result.push([keys[i], values[j]]);
                    }
                }
                return result;
            };
            FormData.prototype.keys = function() {
                var result = [];
                var keys = Object.keys(this._data);
                for (var i = 0; i < keys.length; i++) {
                    var values = this._data[keys[i]];
                    for (var j = 0; j < values.length; j++) {
                        result.push(keys[i]);
                    }
                }
                return result;
            };
            FormData.prototype.values = function() {
                var result = [];
                var keys = Object.keys(this._data);
                for (var i = 0; i < keys.length; i++) {
                    var values = this._data[keys[i]];
                    for (var j = 0; j < values.length; j++) {
                        result.push(values[j]);
                    }
                }
                return result;
            };
            FormData.prototype.forEach = function(fn) {
                var keys = Object.keys(this._data);
                for (var i = 0; i < keys.length; i++) {
                    var values = this._data[keys[i]];
                    for (var j = 0; j < values.length; j++) {
                        fn(values[j], keys[i], this);
                    }
                }
            };
            globalThis.FormData = FormData;
        })();
        globalThis.window = globalThis;
        "#;

        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to setup Blob/FormData: {:?}", e))?;
        Ok(())
    }
}
