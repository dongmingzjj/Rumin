//! IndexedDB mock implementation — pure JS, in-memory storage.
//!
//! Provides a minimal IndexedDB API:
//! - indexedDB.open(name, version) → IDBOpenDBRequest
//! - indexedDB.deleteDatabase(name)
//! - db.transaction(storeNames, mode) → IDBTransaction
//! - store.get/put/delete/getAll/clear → IDBRequest
//! - db.createObjectStore(name, {keyPath, autoIncrement})
//! - All Request onsuccess callbacks fire via setTimeout(fn, 0)

use super::*;

impl JsEngine {
    /// Setup indexedDB as a pure JS mock with in-memory storage.
    pub fn setup_indexed_db(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            // Internal in-memory database storage
            var _idb_databases = {};

            function IDBRequest() {
                this.result = undefined;
                this.error = null;
                this.readyState = 'pending';
                this.onsuccess = null;
                this.onerror = null;
            }
            IDBRequest.prototype._complete = function(result) {
                var self = this;
                self.result = result;
                self.readyState = 'done';
                if (typeof self.onsuccess === 'function') {
                    setTimeout(function() {
                        self.onsuccess({ target: self });
                    }, 0);
                }
            };
            IDBRequest.prototype._fail = function(err) {
                var self = this;
                self.error = err;
                self.readyState = 'done';
                if (typeof self.onerror === 'function') {
                    setTimeout(function() {
                        self.onerror({ target: self });
                    }, 0);
                }
            };

            function IDBObjectStore(name, opts) {
                this.name = name;
                this.keyPath = (opts && opts.keyPath) || null;
                this.autoIncrement = (opts && opts.autoIncrement) || false;
                this._data = {};
                this._nextAutoId = 1;
            }
            IDBObjectStore.prototype.get = function(key) {
                var req = new IDBRequest();
                var val = this._data.hasOwnProperty(key) ? this._data[key] : undefined;
                setTimeout(function() { req._complete(val); }, 0);
                return req;
            };
            IDBObjectStore.prototype.put = function(value) {
                var req = new IDBRequest();
                var key;
                if (this.keyPath && typeof value === 'object' && value !== null) {
                    key = value[this.keyPath];
                }
                if (key === undefined || key === null) {
                    if (this.autoIncrement) {
                        key = this._nextAutoId++;
                        if (typeof value === 'object' && value !== null && this.keyPath) {
                            value[this.keyPath] = key;
                        }
                    } else {
                        key = String(Math.random());
                    }
                }
                this._data[key] = value;
                setTimeout(function() { req._complete(key); }, 0);
                return req;
            };
            IDBObjectStore.prototype.delete = function(key) {
                var req = new IDBRequest();
                delete this._data[key];
                setTimeout(function() { req._complete(undefined); }, 0);
                return req;
            };
            IDBObjectStore.prototype.getAll = function() {
                var req = new IDBRequest();
                var values = [];
                for (var k in this._data) {
                    if (this._data.hasOwnProperty(k)) {
                        values.push(this._data[k]);
                    }
                }
                setTimeout(function() { req._complete(values); }, 0);
                return req;
            };
            IDBObjectStore.prototype.clear = function() {
                var req = new IDBRequest();
                this._data = {};
                this._nextAutoId = 1;
                setTimeout(function() { req._complete(undefined); }, 0);
                return req;
            };

            function IDBTransaction(storeNames, mode, db) {
                this.mode = mode || 'readonly';
                this.objectStoreNames = Array.isArray(storeNames) ? storeNames : [storeNames];
                this._db = db;
                this._aborted = false;
            }
            IDBTransaction.prototype.objectStore = function(name) {
                if (!this._db._stores[name]) {
                    throw new Error('Object store not found: ' + name);
                }
                return this._db._stores[name];
            };
            IDBTransaction.prototype.abort = function() {
                this._aborted = true;
            };

            function IDBDatabase(name, version) {
                this.name = name;
                this.version = version;
                this.objectStoreNames = [];
                this._stores = {};
            }
            IDBDatabase.prototype.createObjectStore = function(name, opts) {
                var store = new IDBObjectStore(name, opts);
                this._stores[name] = store;
                this.objectStoreNames.push(name);
                return store;
            };
            IDBDatabase.prototype.deleteObjectStore = function(name) {
                delete this._stores[name];
                var idx = this.objectStoreNames.indexOf(name);
                if (idx >= 0) this.objectStoreNames.splice(idx, 1);
            };
            IDBDatabase.prototype.transaction = function(storeNames, mode) {
                return new IDBTransaction(storeNames, mode || 'readonly', this);
            };
            IDBDatabase.prototype.close = function() {};

            function IDBOpenDBRequest() {
                IDBRequest.call(this);
                this.onupgradeneeded = null;
            }
            IDBOpenDBRequest.prototype = Object.create(IDBRequest.prototype);
            IDBOpenDBRequest.prototype.constructor = IDBOpenDBRequest;

            globalThis.indexedDB = {
                open: function(name, version) {
                    var req = new IDBOpenDBRequest();
                    name = name || 'default';
                    version = version || 1;
                    setTimeout(function() {
                        var existing = _idb_databases[name];
                        if (!existing) {
                            var db = new IDBDatabase(name, version);
                            _idb_databases[name] = db;
                            req.result = db;
                            req.readyState = 'done';
                            if (typeof req.onupgradeneeded === 'function') {
                                try {
                                    req.onupgradeneeded({ target: req, oldVersion: 0, newVersion: version });
                                } catch(e) {}
                            }
                            if (typeof req.onsuccess === 'function') {
                                try {
                                    req.onsuccess({ target: req });
                                } catch(e) {}
                            }
                        } else {
                            req.result = existing;
                            req.readyState = 'done';
                            if (typeof req.onsuccess === 'function') {
                                try {
                                    req.onsuccess({ target: req });
                                } catch(e) {}
                            }
                        }
                    }, 0);
                    return req;
                },
                deleteDatabase: function(name) {
                    var req = new IDBRequest();
                    name = name || 'default';
                    setTimeout(function() {
                        delete _idb_databases[name];
                        req._complete(null);
                    }, 0);
                    return req;
                }
            };

            // Also expose IDBKeyRange for completeness
            globalThis.IDBKeyRange = {
                only: function(value) { return { lower: value, upper: value, lowerOpen: false, upperOpen: false }; },
                lowerBound: function(lower, open) { return { lower: lower, lowerOpen: !!open }; },
                upperBound: function(upper, open) { return { upper: upper, upperOpen: !!open }; },
                bound: function(lower, upper, lowerOpen, upperOpen) { return { lower: lower, upper: upper, lowerOpen: !!lowerOpen, upperOpen: !!upperOpen }; }
            };
        })();
        "#;

        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                let _: Value = ctx.eval(code)?;
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to setup indexedDB: {:?}", e))?;
        Ok(())
    }
}
