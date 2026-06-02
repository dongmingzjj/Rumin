//! Event, CustomEvent, and EventTarget Web API.

use super::*;

impl JsEngine {
    /// Register Event, CustomEvent constructors and window-level event methods.
    pub fn setup_events(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            // Event constructor
            globalThis.Event = function(type, options) {
                options = options || {};
                this.type = type;
                this.target = null;
                this.currentTarget = null;
                this.bubbles = !!options.bubbles;
                this.cancelable = !!options.cancelable;
                this.defaultPrevented = false;
                this.timeStamp = Date.now();
                this.isTrusted = false;
                this._propagationStopped = false;
                this._immediatePropagationStopped = false;
            };
            globalThis.Event.prototype.preventDefault = function() {
                if (this.cancelable) this.defaultPrevented = true;
            };
            globalThis.Event.prototype.stopPropagation = function() {
                this._propagationStopped = true;
            };
            globalThis.Event.prototype.stopImmediatePropagation = function() {
                this._propagationStopped = true;
                this._immediatePropagationStopped = true;
            };

            // CustomEvent constructor
            globalThis.CustomEvent = function(type, options) {
                options = options || {};
                globalThis.Event.call(this, type, options);
                this.detail = options.detail !== undefined ? options.detail : null;
            };
            globalThis.CustomEvent.prototype = Object.create(globalThis.Event.prototype);
            globalThis.CustomEvent.prototype.constructor = globalThis.CustomEvent;

            // EventTarget class (global constructor)
            globalThis.EventTarget = function() {
                this._listeners = {};
            };
            globalThis.EventTarget.prototype.addEventListener = function(type, listener) {
                if (!this._listeners) this._listeners = {};
                if (!this._listeners[type]) this._listeners[type] = [];
                this._listeners[type].push(listener);
            };
            globalThis.EventTarget.prototype.removeEventListener = function(type, listener) {
                if (!this._listeners || !this._listeners[type]) return;
                var idx = this._listeners[type].indexOf(listener);
                if (idx >= 0) this._listeners[type].splice(idx, 1);
            };
            globalThis.EventTarget.prototype.dispatchEvent = function(event) {
                event.target = this;
                event.currentTarget = this;
                var listeners = (this._listeners && this._listeners[event.type]) || [];
                for (var i = 0; i < listeners.length; i++) {
                    if (event._immediatePropagationStopped) break;
                    if (typeof listeners[i] === 'function') {
                        listeners[i].call(this, event);
                    }
                }
                return !event.defaultPrevented;
            };

            // EventTarget on globalThis (window)
            globalThis._listeners = {};
            globalThis.addEventListener = globalThis.EventTarget.prototype.addEventListener.bind(globalThis);
            globalThis.removeEventListener = globalThis.EventTarget.prototype.removeEventListener.bind(globalThis);
            globalThis.dispatchEvent = globalThis.EventTarget.prototype.dispatchEvent.bind(globalThis);
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup events: {:?}", e))?;
        Ok(())
    }
}
