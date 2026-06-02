//! MutationObserver Web API (simplified).

use super::*;

impl JsEngine {
    /// Register MutationObserver constructor and global helper functions.
    pub fn setup_mutation_observer(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            globalThis.__mutation_observers__ = [];

            globalThis.MutationObserver = function(callback) {
                this._callback = callback;
                this._targets = [];
                this._records = [];
                globalThis.__mutation_observers__.push(this);
            };

            globalThis.MutationObserver.prototype.observe = function(target, config) {
                this._config = config || {};
                this._target = target;
                if (this._targets.indexOf(target) < 0) {
                    this._targets.push(target);
                }
            };

            globalThis.MutationObserver.prototype.disconnect = function() {
                this._targets = [];
                this._target = null;
            };

            globalThis.MutationObserver.prototype.takeRecords = function() {
                var records = this._records.slice();
                this._records.length = 0;
                return records;
            };

            // Called by mutation helpers to notify observers
            globalThis.__notify_mutation_observers__ = function(nodeId, type, data) {
                for (var i = 0; i < globalThis.__mutation_observers__.length; i++) {
                    var obs = globalThis.__mutation_observers__[i];
                    if (!obs._target) continue;
                    var targetId = obs._target._nodeId;
                    if (!targetId) continue;

                    var config = obs._config || {};
                    var shouldNotify = false;

                    if (type === 'attributes' && config.attributes) shouldNotify = true;
                    if (type === 'childList' && config.childList) shouldNotify = true;
                    if (type === 'characterData' && config.characterData) shouldNotify = true;

                    // Also match if observing subtree and the mutation target is a descendant
                    if (!shouldNotify && config.subtree) {
                        var el = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[nodeId] : null;
                        while (el) {
                            if (el._nodeId === targetId) { shouldNotify = true; break; }
                            el = el.parentNode;
                        }
                    }

                    if (shouldNotify || (targetId === nodeId)) {
                        var record = {
                            type: type,
                            target: obs._target,
                            addedNodes: [],
                            removedNodes: [],
                            attributeName: data.name || null,
                            oldValue: data.oldValue || null
                        };
                        if (data.childId) {
                            var child = (typeof __dom_elements__ !== 'undefined') ? __dom_elements__[data.childId] : null;
                            if (type === 'childList' && data.action === 'add') {
                                if (child) record.addedNodes.push(child);
                            } else if (type === 'childList' && data.action === 'remove') {
                                if (child) record.removedNodes.push(child);
                            }
                        }
                        obs._records.push(record);
                    }
                }
            };

            // Flush all pending observer callbacks
            globalThis.__flush_mutation_observers__ = function() {
                for (var i = 0; i < globalThis.__mutation_observers__.length; i++) {
                    var obs = globalThis.__mutation_observers__[i];
                    if (obs._records.length > 0 && typeof obs._callback === 'function') {
                        var records = obs._records.slice();
                        obs._records.length = 0;
                        try { obs._callback(records, obs); } catch(e) {}
                    }
                }
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup mutation observer: {:?}", e))?;

        // Patch mutation helpers in dom_bridge to notify observers
        let patch_code = r#"
        (function() {
            if (typeof __mut_set_attr__ === 'undefined') return;
            var _orig_set_attr = __mut_set_attr__;
            __mut_set_attr__ = function(nid, name, value) {
                _orig_set_attr(nid, name, value);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(nid, 'attributes', {name: name});
                }
            };
            var _orig_remove_attr = __mut_remove_attr__;
            __mut_remove_attr__ = function(nid, name) {
                _orig_remove_attr(nid, name);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(nid, 'attributes', {name: name});
                }
            };
            var _orig_set_text = __mut_set_text__;
            __mut_set_text__ = function(nid, text) {
                _orig_set_text(nid, text);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(nid, 'characterData', {});
                }
            };
            var _orig_set_inner_html = __mut_set_inner_html__;
            __mut_set_inner_html__ = function(nid, html) {
                _orig_set_inner_html(nid, html);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(nid, 'childList', {});
                }
            };
            var _orig_append_child = __mut_append_child__;
            __mut_append_child__ = function(parentId, childTag) {
                _orig_append_child(parentId, childTag);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(parentId, 'childList', {action: 'add', childId: childTag});
                }
            };
            var _orig_remove_child = __mut_remove_child__;
            __mut_remove_child__ = function(parentId, childId) {
                _orig_remove_child(parentId, childId);
                if (typeof __notify_mutation_observers__ === 'function') {
                    __notify_mutation_observers__(parentId, 'childList', {action: 'remove', childId: childId});
                }
            };
        })();
        "#;
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(patch_code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to patch mutation helpers: {:?}", e))?;

        Ok(())
    }
}
